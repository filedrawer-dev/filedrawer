//! The link checks' rows (`link_checks`, migrations 0020 to 0022), and the recheck
//! of the links that are due.

use std::collections::HashMap;

use sqlx::{SqliteConnection, SqlitePool};
use time::{Duration, OffsetDateTime};

use analyzer::PinFlag;

use super::{CheckedLink, Field, PinCheck, Reach, check};
use crate::AppState;
use crate::time::stamp;

/// How long a dead link, or a pin found bad, waits for its next check (design.md,
/// Submission and screening: a week).
pub const RECHECK_DAYS: i64 = 7;

/// How long a link that said "retry later", or a pin the server couldn't check,
/// waits the first time: an hour (the owner's decision, 2026-11-07: "set up a timer
/// to run more checks in an hour"). It doubles with each retry in a row, at most
/// [`RECHECK_DAYS`] (uncertainties.md, "What a recheck covers, and when").
const FIRST_RETRY: Duration = Duration::hours(1);

/// The most entries one run rechecks; the rest wait for the next run.
const RECHECKS_PER_RUN: i64 = 100;

/// When a link is checked again.
struct NextCheck {
    /// `None`: never, as it is live and its pin, if any, verified.
    at: Option<OffsetDateTime>,
    /// Its retries in a row, counting this check.
    retries: i64,
}

/// When `link`, checked at `now` after `retries_before` retries in a row, is
/// checked again.
fn next_check(link: &CheckedLink, retries_before: i64, now: OffsetDateTime) -> NextCheck {
    let week = Duration::days(RECHECK_DAYS);
    match (&link.reach, &link.pin) {
        // Definite answers: a week, without the hourly retries.
        (Reach::Dead(_), _) | (_, Some(PinCheck::Failed(_) | PinCheck::OnNoBranch(_))) => {
            NextCheck {
                at: Some(now + week),
                retries: 0,
            }
        }
        (Reach::RetryLater(_), _) | (_, Some(PinCheck::NotVerified(_))) => {
            // 2^8 hours is past a week, so the doubling stops there.
            let doublings = u32::try_from(retries_before.clamp(0, 8)).unwrap_or(8);
            let wait = (FIRST_RETRY * 2i32.pow(doublings)).min(week);
            NextCheck {
                at: Some(now + wait),
                retries: retries_before + 1,
            }
        }
        _ => NextCheck {
            at: None,
            retries: 0,
        },
    }
}

// The columns' values. The table's CHECK constraints allow no others, so each
// reading back takes the last value for anything else.

impl Field {
    fn from_column(name: &str) -> Field {
        match name {
            "code" => Field::Code,
            _ => Field::Logs,
        }
    }
}

impl Reach {
    /// The `reach` column and its detail.
    fn to_columns(&self) -> (&'static str, Option<&str>) {
        match self {
            Reach::Live => ("live", None),
            Reach::Dead(why) => ("dead", Some(why)),
            Reach::RetryLater(why) => ("retry", Some(why)),
        }
    }

    fn from_columns(reach: &str, detail: Option<String>) -> Reach {
        let detail = detail.unwrap_or_default();
        match reach {
            "live" => Reach::Live,
            "dead" => Reach::Dead(detail),
            _ => Reach::RetryLater(detail),
        }
    }
}

impl PinCheck {
    /// The `pin` column and its detail.
    fn to_columns(&self) -> (&'static str, Option<&str>) {
        match self {
            PinCheck::Verified => ("verified", None),
            PinCheck::Failed(why) => ("failed", Some(why)),
            PinCheck::OnNoBranch(why) => ("no_branch", Some(why)),
            PinCheck::NotVerified(why) => ("not_verified", Some(why)),
        }
    }

    fn from_columns(pin: &str, detail: Option<String>) -> PinCheck {
        let detail = detail.unwrap_or_default();
        match pin {
            "verified" => PinCheck::Verified,
            "failed" => PinCheck::Failed(detail),
            "no_branch" => PinCheck::OnNoBranch(detail),
            _ => PinCheck::NotVerified(detail),
        }
    }
}

/// The card's pin flag from the `link_flags` view's `pin_flag` column.
pub fn pin_flag(column: Option<&str>) -> Option<PinFlag> {
    column.and_then(|pin| PinCheck::from_columns(pin, None).flag())
}

/// Stores entry `seq`'s links as its submission checked them at `now`, replacing
/// what it had. Each retry count starts again, so resubmitting can't push a check
/// out.
pub async fn store(
    transaction: &mut SqliteConnection,
    seq: i64,
    links: &[CheckedLink],
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    store_with(transaction, seq, links, now, HashMap::new()).await
}

/// Makes every stored link of entry `seq` due for a recheck at `now`, as a release
/// does (design.md, Embargo: "At release the server checks the `code` and `logs`
/// links again").
pub async fn make_due(
    connection: &mut SqliteConnection,
    seq: i64,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let at = stamp(now);
    sqlx::query!(
        "UPDATE link_checks SET next_check_at = ? WHERE entry_seq = ?",
        at,
        seq
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// Stores entry `seq`'s links as a recheck found them at `now`, carrying on each
/// link's retries in a row.
async fn store_rechecked(
    transaction: &mut SqliteConnection,
    seq: i64,
    links: &[CheckedLink],
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let retries_before: HashMap<(String, String), i64> = sqlx::query!(
        "SELECT field, url, retries FROM link_checks WHERE entry_seq = ?",
        seq
    )
    .fetch_all(&mut *transaction)
    .await?
    .into_iter()
    .map(|row| ((row.field, row.url), row.retries))
    .collect();
    store_with(transaction, seq, links, now, retries_before).await
}

/// Stores entry `seq`'s links checked at `now`, given each link's retries in a row
/// before this check.
async fn store_with(
    transaction: &mut SqliteConnection,
    seq: i64,
    links: &[CheckedLink],
    now: OffsetDateTime,
    retries_before: HashMap<(String, String), i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM link_checks WHERE entry_seq = ?", seq)
        .execute(&mut *transaction)
        .await?;
    let checked_at = stamp(now);
    for link in links {
        let field = link.field.name();
        let (reach, reach_detail) = link.reach.to_columns();
        let (pin, pin_detail) = link.pin.as_ref().map(PinCheck::to_columns).unzip();
        let pin_detail = pin_detail.flatten();
        let before = retries_before
            .get(&(field.to_string(), link.url.clone()))
            .copied()
            .unwrap_or(0);
        let next = next_check(link, before, now);
        let next_check_at = next.at.map(stamp);
        sqlx::query!(
            "INSERT INTO link_checks
                 (entry_seq, field, url, reach, reach_detail, pin, pin_detail, checked_at,
                  next_check_at, retries)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            seq,
            field,
            link.url,
            reach,
            reach_detail,
            pin,
            pin_detail,
            checked_at,
            next_check_at,
            next.retries
        )
        .execute(&mut *transaction)
        .await?;
    }
    Ok(())
}

/// Entry `seq`'s links as last checked, `code` first.
pub async fn stored(pool: &SqlitePool, seq: i64) -> Result<Vec<CheckedLink>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT field, url, reach, reach_detail, pin, pin_detail FROM link_checks
         WHERE entry_seq = ? ORDER BY field, rowid",
        seq
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| CheckedLink {
            field: Field::from_column(&row.field),
            url: row.url,
            reach: Reach::from_columns(&row.reach, row.reach_detail),
            pin: row
                .pin
                .map(|pin| PinCheck::from_columns(&pin, row.pin_detail)),
        })
        .collect())
}

/// Rechecks the links of every entry with a link whose next check (`next_check`) has
/// come by `now`, unless the entry is rejected, withdrawn or removed. Its GitHub calls
/// count against no account's share, and it stops while fewer than three quarters of
/// the hour's calls are left. Returns how many entries it rechecked.
pub async fn recheck_due(state: &AppState, now: OffsetDateTime) -> Result<usize, sqlx::Error> {
    let now_text = stamp(now);
    let due = sqlx::query_scalar!(
        r#"SELECT DISTINCT l.entry_seq AS "seq!: i64"
           FROM link_checks l JOIN entries e ON e.seq = l.entry_seq
           WHERE l.next_check_at <= ?
             AND e.status NOT IN ('rejected', 'withdrawn', 'removed')
           ORDER BY l.entry_seq
           LIMIT ?"#,
        now_text,
        RECHECKS_PER_RUN
    )
    .fetch_all(&state.pool)
    .await?;
    let mut rechecked = 0;
    for seq in due {
        if state.link_checker.github_low() || state.link_checker.github_paused() {
            break;
        }
        recheck_entry(state, seq, now).await?;
        rechecked += 1;
    }
    Ok(rechecked)
}

/// Checks entry `seq`'s links again at `now`. A pin found bad only flags the card:
/// nothing rejects an entry after submission (the owner's decision, 2026-11-07).
async fn recheck_entry(state: &AppState, seq: i64, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let source = sqlx::query_scalar!("SELECT source FROM entries WHERE seq = ?", seq)
        .fetch_optional(&state.pool)
        .await?;
    let Some(source) = source else {
        return Ok(());
    };
    let analysis = analyzer::analyze(&source);
    let links = check(state, &[&analysis.fields], None)
        .await
        .pop()
        .unwrap_or_default();
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    // A PUT during the fetches stored its own text's links, which stand.
    let unchanged = sqlx::query_scalar!("SELECT source FROM entries WHERE seq = ?", seq)
        .fetch_optional(&mut *transaction)
        .await?
        .is_some_and(|now_stored| now_stored == source);
    if unchanged {
        store_rechecked(&mut transaction, seq, &links, now).await?;
    }
    transaction.commit().await
}
