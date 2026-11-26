//! Withdrawal notices (design.md, Entry lifecycle; #31): an author withdraws a public
//! version from the account page with a reason, the version becomes `withdrawn`, and
//! its page shows a notice in its place. The reason shows once the screen has passed
//! it, as it checks entry text; until then, or when it doesn't pass, the notice says
//! so without it.

use sqlx::{SqliteConnection, SqlitePool};
use time::OffsetDateTime;

use crate::screening::{MAX_ATTEMPTS, ModelError, Subject, Verdict};
use crate::{AppState, accounts, queue};

/// The most characters a reason may hold.
pub const MAX_REASON_CHARS: usize = 1000;

/// Why a reason isn't taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BadReason {
    #[error("give the reason you withdraw it, which the notice shows once screening checks it")]
    Missing,
    #[error("the reason is {0} characters, over the cap of {MAX_REASON_CHARS}")]
    TooLong(usize),
}

/// `reason` as the notice keeps it: trimmed, with Windows line ends made plain, or
/// why it isn't taken.
pub fn reason(reason: Option<&str>) -> Result<String, BadReason> {
    let reason = reason.unwrap_or("").replace("\r\n", "\n");
    let reason = reason.trim();
    let chars = reason.chars().count();
    if chars == 0 {
        return Err(BadReason::Missing);
    }
    if chars > MAX_REASON_CHARS {
        return Err(BadReason::TooLong(chars));
    }
    Ok(reason.to_string())
}

/// Withdraws public entry `seq` with `reason`, at `at`, in the caller's transaction.
/// Returns whether it was still public, and so withdrawn now.
pub async fn withdraw(
    connection: &mut SqliteConnection,
    seq: i64,
    reason: &str,
    at: &str,
) -> Result<bool, sqlx::Error> {
    let changed = sqlx::query!(
        "UPDATE entries SET status = 'withdrawn' WHERE seq = ? AND status = 'public'",
        seq
    )
    .execute(&mut *connection)
    .await?
    .rows_affected();
    if changed == 0 {
        return Ok(false);
    }
    sqlx::query!(
        "INSERT INTO withdrawal_notices (entry_seq, reason, withdrawn_at) VALUES (?, ?, ?)",
        seq,
        reason,
        at
    )
    .execute(&mut *connection)
    .await?;
    Ok(true)
}

/// When `account_id` withdrew its public entry `id`, if it did.
pub async fn withdrawn_at(
    pool: &SqlitePool,
    account_id: &str,
    id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT n.withdrawn_at FROM withdrawal_notices n JOIN entries e ON e.seq = n.entry_seq
         WHERE e.id = ? AND e.account_id = ? AND e.status = 'withdrawn'",
        id,
        account_id
    )
    .fetch_optional(pool)
    .await
}

/// Where a notice's reason stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Waiting for the screen.
    Waiting,
    /// Passed, so the notice shows it.
    Shown(String),
    /// The screen refused it, for these reasons, which only its author sees.
    Refused(Vec<String>),
    /// Held for the archive's owner: the screen saw a likely hazard, or the account
    /// is under the review guard.
    Held,
}

impl Reason {
    fn of(screen: Option<&str>, reason: String, screen_reasons: Option<&str>) -> Reason {
        match screen {
            None => Reason::Waiting,
            Some("pass") => Reason::Shown(reason),
            Some("reject") => Reason::Refused(
                screen_reasons
                    .and_then(|reasons| serde_json::from_str(reasons).ok())
                    .unwrap_or_default(),
            ),
            Some(_) => Reason::Held,
        }
    }

    /// What the notice's `reason:` line says.
    pub fn line(&self) -> &'static str {
        match self {
            Reason::Waiting => "not shown yet: screening checks it first",
            Reason::Shown(_) => "below the delimiter",
            Reason::Refused(_) => "not shown: screening refused it",
            Reason::Held => "not shown: held for the archive's owner",
        }
    }

    /// Its one word in JSON.
    pub fn status(&self) -> &'static str {
        match self {
            Reason::Waiting => "screening",
            Reason::Shown(_) => "shown",
            Reason::Refused(_) => "refused",
            Reason::Held => "held",
        }
    }

    /// The reason itself, once it shows.
    pub fn shown(&self) -> Option<&str> {
        match self {
            Reason::Shown(text) => Some(text),
            _ => None,
        }
    }
}

/// A withdrawn version's notice, as its page shows it.
pub struct Notice {
    pub id: String,
    pub account_id: String,
    /// `None` once the account is deleted.
    pub display_name: Option<String>,
    /// The linked GitHub account's login, while cards show it.
    pub github_login: Option<String>,
    pub submitted_at: String,
    pub published_at: Option<String>,
    pub withdrawn_at: String,
    /// The public version this one replaced.
    pub previous_id: Option<String>,
    /// The public version that replaced this one.
    pub superseded_by: Option<String>,
    pub reason: Reason,
}

/// The notice of withdrawn entry `id`, if it is one its author withdrew while public.
pub async fn notice(pool: &SqlitePool, id: &str) -> Result<Option<Notice>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT e.id, e.account_id, a.display_name AS "display_name?: String",
                  h.login AS "github_login?: String", e.submitted_at, e.published_at,
                  (SELECT p.id FROM entries p WHERE p.seq = e.previous_seq AND p.status = 'public') AS "previous_id?",
                  (SELECT x.id FROM entries x WHERE x.previous_seq = e.seq AND x.status = 'public') AS "superseded_by?",
                  n.reason, n.withdrawn_at, n.screen, n.screen_reasons
           FROM withdrawal_notices n JOIN entries e ON e.seq = n.entry_seq
           JOIN authors a ON a.id = e.account_id
           LEFT JOIN github_handles h ON h.account_id = e.account_id
           WHERE e.id = ? AND e.status = 'withdrawn'"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| Notice {
        reason: Reason::of(
            row.screen.as_deref(),
            row.reason,
            row.screen_reasons.as_deref(),
        ),
        id: row.id,
        account_id: row.account_id,
        display_name: row.display_name,
        github_login: row.github_login,
        submitted_at: row.submitted_at,
        published_at: row.published_at,
        withdrawn_at: row.withdrawn_at,
        previous_id: row.previous_id,
        superseded_by: row.superseded_by,
    }))
}

/// A version the account withdrew while public, as its account page lists it.
pub struct Own {
    pub id: String,
    pub withdrawn_at: String,
    pub reason: Reason,
}

/// The account's withdrawn public versions, newest withdrawal first.
pub async fn own(pool: &SqlitePool, account_id: &str) -> Result<Vec<Own>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT e.id, n.reason, n.withdrawn_at, n.screen, n.screen_reasons
         FROM withdrawal_notices n JOIN entries e ON e.seq = n.entry_seq
         WHERE e.account_id = ? AND e.status = 'withdrawn'
         ORDER BY n.withdrawn_at DESC, e.seq DESC",
        account_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Own {
            reason: Reason::of(
                row.screen.as_deref(),
                row.reason,
                row.screen_reasons.as_deref(),
            ),
            id: row.id,
            withdrawn_at: row.withdrawn_at,
        })
        .collect())
}

/// The withdrawn entries whose reasons wait for the screen, oldest withdrawal first.
pub async fn unscreened(pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT n.entry_seq AS "seq!: i64" FROM withdrawal_notices n
           JOIN entries e ON e.seq = n.entry_seq
           WHERE n.screen IS NULL AND e.status = 'withdrawn'
           ORDER BY n.withdrawn_at, n.entry_seq"#
    )
    .fetch_all(pool)
    .await
}

/// A reason waiting for the screen.
struct Waiting {
    account_id: String,
    reason: String,
    screen_attempts: i64,
}

/// Screens the reason of withdrawn entry `seq` if it has no verdict yet and the caps
/// allow, and stores the verdict: a pass shows it, a rejection keeps it from the
/// notice with the reasons for its author, and a likely hazard holds it for the
/// archive's owner. Staging's smoke-test account passes unscreened; an account
/// under the review guard has it held, unscreened. An outage, or a spent cap, leaves
/// it for a later tick.
pub async fn screen(state: &AppState, seq: i64, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    // As `queue::screen_entry`: one screening at a time, the check after the lock.
    let _turn = state.screening_lock.lock().await;
    let Some(waiting) = sqlx::query_as!(
        Waiting,
        "SELECT e.account_id, n.reason, n.screen_attempts
         FROM withdrawal_notices n JOIN entries e ON e.seq = n.entry_seq
         WHERE n.entry_seq = ? AND n.screen IS NULL AND e.status = 'withdrawn'",
        seq
    )
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(());
    };
    let verdict = if state.smoke_account.as_deref() == Some(waiting.account_id.as_str()) {
        Verdict::Pass {
            note: None,
            hazard_doubt: None,
        }
    } else if accounts::is_guarded(&state.pool, &waiting.account_id).await? {
        Verdict::Review {
            why: "the account is under the review guard".into(),
        }
    } else if queue::over_cap(state, &waiting.account_id, now).await? {
        return Ok(());
    } else {
        let screening = state
            .screener
            .screen_as(&waiting.reason, Subject::Notice)
            .await;
        state.screening_health.note(&screening, Subject::Notice);
        queue::log_calls(
            &state.pool,
            Some(seq),
            &waiting.account_id,
            &screening.calls,
            now,
            Subject::Notice,
        )
        .await?;
        match screening.verdict {
            Ok(verdict) => verdict,
            Err(error) => {
                tracing::warn!(%error, entry_seq = seq, "screening a withdrawal notice failed");
                match after_failure(&state.pool, seq, &waiting, &error).await? {
                    Some(verdict) => verdict,
                    None => return Ok(()),
                }
            }
        }
    };
    store(&state.pool, seq, &verdict, now).await
}

/// What a failed call means for the reason, as `queue::after_failure` reads one for
/// an entry: an outage waits for a later tick without counting; a reply that can't
/// be read, or a request the API refuses, counts, and the reason is held for the
/// owner at [`MAX_ATTEMPTS`] of them.
async fn after_failure(
    pool: &SqlitePool,
    seq: i64,
    waiting: &Waiting,
    error: &ModelError,
) -> Result<Option<Verdict>, sqlx::Error> {
    let counts = matches!(error, ModelError::Unreadable(_)) || error.refuses_request();
    if !counts {
        return Ok(None);
    }
    if waiting.screen_attempts + 1 < MAX_ATTEMPTS {
        sqlx::query!(
            "UPDATE withdrawal_notices SET screen_attempts = screen_attempts + 1
             WHERE entry_seq = ?",
            seq
        )
        .execute(pool)
        .await?;
        return Ok(None);
    }
    Ok(Some(Verdict::Review {
        why: format!("screening failed {MAX_ATTEMPTS} times: {error}"),
    }))
}

/// Stores `verdict` on the reason of entry `seq`, unless it has one.
async fn store(
    pool: &SqlitePool,
    seq: i64,
    verdict: &Verdict,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let (screen, reasons) = match verdict {
        Verdict::Pass { .. } => ("pass", None),
        Verdict::Reject { reasons } => ("reject", Some(serde_json::json!(reasons).to_string())),
        Verdict::Review { why } => ("review", Some(serde_json::json!([why]).to_string())),
    };
    let at = crate::time::stamp(now);
    sqlx::query!(
        "UPDATE withdrawal_notices SET screen = ?, screen_reasons = ?, screened_at = ?
         WHERE entry_seq = ? AND screen IS NULL",
        screen,
        reasons,
        at,
        seq
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// How many reasons wait for the screen, and how many it held for the archive's
/// owner, for staging's status.
pub async fn counts(pool: &SqlitePool) -> Result<(i64, i64), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT COALESCE(SUM(n.screen IS NULL), 0) AS "waiting!: i64",
                  COALESCE(SUM(n.screen = 'review'), 0) AS "held!: i64"
           FROM withdrawal_notices n JOIN entries e ON e.seq = n.entry_seq
           WHERE e.status = 'withdrawn'"#
    )
    .fetch_one(pool)
    .await?;
    Ok((row.waiting, row.held))
}

/// A reason held for the archive's owner: its entry, and why the screen held it.
pub struct HeldReason {
    pub id: String,
    pub why: String,
}

/// Every reason held for the archive's owner, the oldest withdrawal first, for the
/// daily email: no admin page shows them yet.
pub async fn held(pool: &SqlitePool) -> Result<Vec<HeldReason>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT e.id, n.screen_reasons
         FROM withdrawal_notices n JOIN entries e ON e.seq = n.entry_seq
         WHERE e.status = 'withdrawn' AND n.screen = 'review'
         ORDER BY n.withdrawn_at, n.entry_seq"
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| HeldReason {
            id: row.id,
            why: row
                .screen_reasons
                .as_deref()
                .and_then(|reasons| serde_json::from_str::<Vec<String>>(reasons).ok())
                .map(|reasons| reasons.join("; "))
                .unwrap_or_default(),
        })
        .collect())
}
