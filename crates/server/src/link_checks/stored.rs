//! The link checks' rows (`link_checks`, migration 0020), and the weekly recheck of
//! the flagged ones.

use sqlx::{SqliteConnection, SqlitePool};
use time::OffsetDateTime;

use super::{CheckedLink, Field, PinCheck, Reach, check};
use crate::AppState;
use crate::time::stamp;

/// How long a flagged link waits between checks (design.md, Embargo: the server
/// rechecks every flagged link weekly).
pub const RECHECK_DAYS: i64 = 7;

/// The most entries one run rechecks; the rest wait for the next run.
const RECHECKS_PER_RUN: i64 = 100;

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
            PinCheck::NotVerified(why) => ("not_verified", Some(why)),
        }
    }

    fn from_columns(pin: &str, detail: Option<String>) -> PinCheck {
        let detail = detail.unwrap_or_default();
        match pin {
            "verified" => PinCheck::Verified,
            "failed" => PinCheck::Failed(detail),
            _ => PinCheck::NotVerified(detail),
        }
    }
}

/// Stores entry `seq`'s links as checked at `now`, replacing what it had.
pub async fn store(
    transaction: &mut SqliteConnection,
    seq: i64,
    links: &[CheckedLink],
    now: OffsetDateTime,
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
        sqlx::query!(
            "INSERT INTO link_checks
                 (entry_seq, field, url, reach, reach_detail, pin, pin_detail, checked_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            seq,
            field,
            link.url,
            reach,
            reach_detail,
            pin,
            pin_detail,
            checked_at
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

/// Rechecks the links of every entry with a link that is dead, said "retry later",
/// or holds a pin not verified or failed, last checked [`RECHECK_DAYS`] or more
/// before `now`, unless the entry is rejected, withdrawn or removed. Its GitHub calls
/// count against no account's share, and it stops while fewer than half the hour's
/// calls are left. Returns how many entries it rechecked.
pub async fn recheck_due(state: &AppState, now: OffsetDateTime) -> Result<usize, sqlx::Error> {
    let before = stamp(now - time::Duration::days(RECHECK_DAYS));
    let due = sqlx::query_scalar!(
        r#"SELECT DISTINCT l.entry_seq AS "seq!: i64"
           FROM link_checks l JOIN entries e ON e.seq = l.entry_seq
           WHERE (l.reach != 'live' OR l.pin IN ('not_verified', 'failed'))
             AND l.checked_at <= ?
             AND e.status NOT IN ('rejected', 'withdrawn', 'removed')
           ORDER BY l.entry_seq
           LIMIT ?"#,
        before,
        RECHECKS_PER_RUN
    )
    .fetch_all(&state.pool)
    .await?;
    let mut rechecked = 0;
    for seq in due {
        if state.link_checker.github_low() {
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
        store(&mut transaction, seq, &links, now).await?;
    }
    transaction.commit().await
}
