//! The screening queue (design.md, Submission and screening): entries in `screening`
//! are screened in the fair queue's order within the day's cost caps, their verdicts
//! stored, and each entry, or its whole batch, placed once decided. An account under
//! the review guard has its entries sent to review unscreened.

use analyzer::analyze;
use sqlx::SqlitePool;
use time::OffsetDateTime;

use crate::embargo::embargoed_until;
use crate::screening::{Call, MAX_ATTEMPTS, ModelError, Verdict};
use crate::{AppState, accounts, entries, series, time::stamp};

/// The share of the day's cap one account may spend (design.md: 10%).
const ACCOUNT_SHARE_PERCENT: i64 = 10;

/// An entry waiting to be screened.
struct Waiting {
    seq: i64,
    account_id: String,
    source: String,
    screen_attempts: i64,
}

/// Screens entry `seq` if it is in `screening` without a verdict and the caps allow,
/// then places it, or its batch, when decided. Returns its status afterwards, or
/// `None` if there is no such entry.
pub async fn screen_entry(
    state: &AppState,
    seq: i64,
    now: OffsetDateTime,
) -> Result<Option<String>, sqlx::Error> {
    // One screening at a time, so an entry is never screened twice at once and the
    // day's cap is overshot by at most one item's calls. The check for a verdict
    // comes after the lock, so a second caller finds the first one's.
    let _turn = state.screening_lock.lock().await;
    let waiting = sqlx::query_as!(
        Waiting,
        r#"SELECT seq AS "seq!: i64", account_id, source, screen_attempts
           FROM entries WHERE seq = ? AND status = 'screening' AND screen_verdict IS NULL"#,
        seq
    )
    .fetch_optional(&state.pool)
    .await?;
    if let Some(waiting) = waiting {
        screen_waiting(state, &waiting, now).await?;
    }
    entries::status(&state.pool, seq).await
}

async fn screen_waiting(
    state: &AppState,
    waiting: &Waiting,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let text = screened_text(state, waiting).await?;
    let verdict = match screen_text(state, &waiting.account_id, Some(waiting.seq), &text, now)
        .await?
    {
        Screened::Verdict(verdict) => verdict,
        Screened::Guarded => Verdict::Review {
            why: "the account is under the review guard".into(),
        },
        Screened::OverCap | Screened::Failed(ModelError::Http(_) | ModelError::Status { .. }) => {
            // Waits for a later tick; an outage doesn't count against the entry.
            return Ok(());
        }
        Screened::Failed(ModelError::Unreadable(_))
            if waiting.screen_attempts + 1 < MAX_ATTEMPTS =>
        {
            entries::count_screen_attempt(&state.pool, waiting.seq).await?;
            return Ok(());
        }
        Screened::Failed(ModelError::Unreadable(_)) => Verdict::Review {
            why: format!("screening replies couldn't be read {MAX_ATTEMPTS} times"),
        },
    };
    entries::store_verdict(&state.pool, waiting.seq, &verdict).await?;
    settle(state, waiting.seq, now).await
}

/// What the screen reads for an entry: its text, then the declaration of every run it
/// uses (design.md: "Screening checks run declarations with their batch"), since the
/// run's description is shown with it once public.
async fn screened_text(state: &AppState, waiting: &Waiting) -> Result<String, sqlx::Error> {
    let mut text = waiting.source.clone();
    for (id, declaration) in entries::run_declarations(&state.pool, waiting.seq).await? {
        text.push_str(&format!(
            "\n\nThe declaration of run `{id}`, which this entry uses:\n\n{declaration}\n"
        ));
    }
    Ok(text)
}

/// What screening one text came to.
pub enum Screened {
    Verdict(Verdict),
    /// The account is under the review guard, so nothing was screened: what it sends
    /// goes to the owner's review queue.
    Guarded,
    /// The day, or the account's share of it, is spent; nothing was called.
    OverCap,
    Failed(ModelError),
}

/// Screens `text` for `account_id` within the caps, and logs every call it made;
/// for an account under the review guard, screens nothing and says so. The caller
/// holds `screening_lock`, or is the series route, which takes it here.
pub async fn screen_text(
    state: &AppState,
    account_id: &str,
    entry_seq: Option<i64>,
    text: &str,
    now: OffsetDateTime,
) -> Result<Screened, sqlx::Error> {
    // design.md: once any of an account's entries reaches review, everything it
    // submits goes straight to review, whatever the day's cap.
    if accounts::is_guarded(&state.pool, account_id).await? {
        return Ok(Screened::Guarded);
    }
    if over_cap(state, account_id, now).await? {
        return Ok(Screened::OverCap);
    }
    let screening = state.screener.screen(text).await;
    log_calls(&state.pool, entry_seq, account_id, &screening.calls, now).await?;
    let smoke = state.smoke_account.as_deref() == Some(account_id);
    Ok(match screening.verdict {
        // Staging's smoke-test entries are screened but never rejected, so CI's smoke
        // test can publish them; production refuses the setting.
        Ok(Verdict::Reject { reasons }) if smoke => Screened::Verdict(Verdict::Pass {
            note: Some(format!(
                "staging smoke account: the screen would have rejected it: {}",
                reasons.join("; ")
            )),
        }),
        Ok(verdict) => Screened::Verdict(verdict),
        Err(error) => {
            tracing::warn!(%error, ?entry_seq, "screening failed");
            Screened::Failed(error)
        }
    })
}

/// Whether the day, or the account's share of it, is spent.
async fn over_cap(
    state: &AppState,
    account_id: &str,
    now: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    let (total, account) = spent_on(&state.pool, account_id, now).await?;
    let cap = state.screening_daily_nano_usd;
    Ok(total >= cap || account.saturating_mul(100) >= cap.saturating_mul(ACCOUNT_SHARE_PERCENT))
}

/// What screening has spent today (UTC), in all and by `account_id`.
pub async fn spent_on(
    pool: &SqlitePool,
    account_id: &str,
    now: OffsetDateTime,
) -> Result<(i64, i64), sqlx::Error> {
    let day = format!("{}%", now.date());
    let row = sqlx::query!(
        r#"SELECT COALESCE(SUM(cost_nano_usd), 0) AS "total!: i64",
                  COALESCE(SUM(CASE WHEN account_id = ?2 THEN cost_nano_usd END), 0) AS "account!: i64"
           FROM screenings WHERE at LIKE ?1"#,
        day,
        account_id
    )
    .fetch_one(pool)
    .await?;
    Ok((row.total, row.account))
}

/// Logs each model call and what it cost (design.md: "the server logs what each
/// screening cost").
pub async fn log_calls(
    pool: &SqlitePool,
    entry_seq: Option<i64>,
    account_id: &str,
    calls: &[Call],
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let at = stamp(now);
    for call in calls {
        let model = call.model.id();
        let cost = call.model.cost_nano_usd(call.usage);
        sqlx::query!(
            "INSERT INTO screenings (entry_seq, account_id, model, input_tokens, output_tokens,
                 cost_nano_usd, at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            entry_seq,
            account_id,
            model,
            call.usage.input_tokens,
            call.usage.output_tokens,
            cost,
            at
        )
        .execute(pool)
        .await?;
        tracing::info!(model, cost_nano_usd = cost, ?entry_seq, "screening call");
    }
    Ok(())
}

/// A screened entry, or a member of its batch, as `settle` sees it.
struct Member {
    seq: i64,
    id: String,
    status: String,
    screen_verdict: Option<String>,
}

/// Places a screened entry, or decides its batch (design.md, Agent interface: "A
/// batch is screened and published as a unit"): one member rejected rejects every
/// member still in screening; a member in review goes to review and the passed ones
/// wait; when every member passed, all are placed.
async fn settle(state: &AppState, seq: i64, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let members = sqlx::query_as!(
        Member,
        r#"SELECT m.seq AS "seq!: i64", m.id, m.status, m.screen_verdict
           FROM entries e JOIN entries m
             ON m.seq = e.seq OR (e.batch_seq IS NOT NULL AND m.batch_seq = e.batch_seq)
           WHERE e.seq = ? AND m.status NOT IN ('withdrawn', 'removed')
           ORDER BY m.seq"#,
        seq
    )
    .fetch_all(&state.pool)
    .await?;
    let in_screening = |member: &&Member| member.status == "screening";
    if let Some(failed) = members
        .iter()
        .find(|member| member.screen_verdict.as_deref() == Some("reject"))
    {
        // A member already in review or held goes back too: its batch has failed.
        let undecided =
            |member: &&Member| matches!(member.status.as_str(), "screening" | "review" | "held");
        for member in members.iter().filter(undecided) {
            let reasons = (member.screen_verdict.as_deref() != Some("reject")).then(|| {
                serde_json::json!([format!(
                    "another entry in its batch, {}, was rejected, and a batch is screened as one",
                    failed.id
                )])
                .to_string()
            });
            entries::reject(&state.pool, member.seq, reasons.as_deref()).await?;
        }
        return Ok(());
    }
    for member in members.iter().filter(in_screening) {
        if member.screen_verdict.as_deref() == Some("review") {
            entries::to_review(&state.pool, member.seq).await?;
        }
    }
    // Placed only when every member passed; one in review, or not yet screened,
    // holds the rest in screening.
    let decided = members.iter().all(|member| {
        member.status == "screening" && member.screen_verdict.as_deref() == Some("pass")
    });
    if !decided {
        return Ok(());
    }
    for member in members.iter().filter(in_screening) {
        place_passed(state, member.seq, now).await?;
    }
    Ok(())
}

/// Places an entry that passed: embargoed while its date is ahead, waiting while a
/// series it or its batch cites is hidden, otherwise public.
async fn place_passed(state: &AppState, seq: i64, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let Some(source) = sqlx::query_scalar!(
        "SELECT source FROM entries WHERE seq = ? AND status = 'screening'",
        seq
    )
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(());
    };
    let analysis = analyze(&source);
    if embargoed_until(analysis.fields.embargo_until.as_deref(), now).is_some() {
        entries::embargo(&state.pool, seq).await?;
    } else if series::waits(&state.pool, seq).await? {
        entries::wait_on_series(&state.pool, seq).await?;
    } else {
        entries::publish(&state.pool, seq, &analysis).await?;
    }
    Ok(())
}

/// Places entry `seq`, or decides its batch, once the owner has given it a verdict.
pub async fn settle_decided(
    state: &AppState,
    seq: i64,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let _turn = state.screening_lock.lock().await;
    settle(state, seq, now).await
}

/// Places every entry left in `screening` with a verdict, as a crash between storing
/// the verdict and placing it would leave one, or a batch whose member in review
/// has been decided since.
pub async fn settle_screened(state: &AppState, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let _turn = state.screening_lock.lock().await;
    let screened = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries
           WHERE status = 'screening' AND screen_verdict IS NOT NULL ORDER BY seq"#
    )
    .fetch_all(&state.pool)
    .await?;
    for seq in screened {
        settle(state, seq, now).await?;
    }
    Ok(())
}

/// Entries in `screening` with no verdict yet, in the fair queue's order: by
/// position, then submission time, so every account's first goes ahead of anyone's
/// second.
pub async fn unscreened(pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries
           WHERE status = 'screening' AND screen_verdict IS NULL
           ORDER BY queue_position, submitted_at, seq"#
    )
    .fetch_all(pool)
    .await
}
