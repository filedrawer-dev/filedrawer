//! The daily allowance (design.md, Limits and abuse): each accepted submission takes a
//! unit of its account's allowance, dated to the earliest UTC day with room, and an
//! entry isn't screened before the earliest day among its units (the
//! `entry_screen_from` view, which the screening queue reads too). A series takes a
//! unit dated today, since it is screened at once. Refused requests take none (the
//! owner's decision, 2026-11-06).

use axum::http::StatusCode;
use sqlx::{SqliteConnection, SqlitePool};
use time::format_description::well_known::Rfc3339;
use time::{Date, Duration, OffsetDateTime};

use crate::error::AppError;
use crate::time::DAY;
use crate::{AppState, invites};

/// The units an account gets each day in its first [`NEW_ACCOUNT_DAYS`] days.
const NEW_ACCOUNT_UNITS: i64 = 20;
/// The units an account gets each day after that.
const UNITS: i64 = 50;
const NEW_ACCOUNT_DAYS: i64 = 7;
/// The days, today first, that an account's units may be dated to.
const WINDOW_DAYS: i64 = 10;

/// `day` as the database stores it.
pub fn day_text(day: Date) -> String {
    day.format(DAY)
        .expect("a date formats as year, month and day")
}

/// Whether `account_id` is outside the allowance: staging's smoke-test and
/// screen-test accounts (the owner's decision, 2026-11-06), and its CI screen
/// account (2026-11-07).
fn exempt(state: &AppState, account_id: &str) -> bool {
    [
        &state.smoke_account,
        &state.screen_test_account,
        &state.ci_screen_account,
    ]
    .into_iter()
    .any(|exempt| exempt.as_deref() == Some(account_id))
}

/// How an account started, as its allowance sees it.
#[derive(Debug, Clone, Copy)]
struct Start {
    /// The UTC day it was made.
    created: Date,
    /// Whether it signed up with an invite code that isn't revoked, or is the admin,
    /// which skips the new-account limit (design.md, Limits and abuse).
    invited: bool,
}

impl Start {
    /// The units the account gets on `day`.
    fn units_on(self, day: Date) -> i64 {
        if !self.invited && day < self.created + Duration::days(NEW_ACCOUNT_DAYS) {
            NEW_ACCOUNT_UNITS
        } else {
            UNITS
        }
    }
}

/// How `account_id` started. The server writes `created_at`; were it unreadable,
/// the account counts as made `today`, so as new.
async fn start_of(
    conn: &mut SqliteConnection,
    account_id: &str,
    today: Date,
) -> Result<Start, sqlx::Error> {
    let account = sqlx::query!(
        r#"SELECT created_at, is_admin AS "is_admin: bool" FROM accounts WHERE id = ?"#,
        account_id
    )
    .fetch_one(&mut *conn)
    .await?;
    let created =
        OffsetDateTime::parse(&account.created_at, &Rfc3339).map_or(today, |at| at.date());
    // The owner's decision, 2026-11-09.
    let invited = account.is_admin || invites::invited(conn, account_id).await?;
    Ok(Start { created, invited })
}

/// Takes a unit for each of `entries`, which `account_id` submitted, dated to the
/// earliest days with room from `today` on. Runs in the caller's transaction, which
/// must already have written, so it holds SQLite's write lock and the counts read
/// here stay true until it commits. Returns false, taking none, when they don't all
/// fit in the window; the caller then refuses the request with [`refusal`].
pub async fn take(
    conn: &mut SqliteConnection,
    state: &AppState,
    account_id: &str,
    entries: &[i64],
    today: Date,
) -> Result<bool, sqlx::Error> {
    if exempt(state, account_id) {
        return Ok(true);
    }
    let start = start_of(conn, account_id, today).await?;
    let from = day_text(today);
    let used = sqlx::query!(
        r#"SELECT day, COUNT(*) AS "units!: i64" FROM allowance_units
           WHERE account_id = ? AND day >= ? GROUP BY day"#,
        account_id,
        from
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut days: Vec<String> = Vec::with_capacity(entries.len());
    for offset in 0..WINDOW_DAYS {
        let day = today + Duration::days(offset);
        let text = day_text(day);
        let taken = used
            .iter()
            .find(|row| row.day == text)
            .map_or(0, |row| row.units);
        let room = usize::try_from(start.units_on(day) - taken).unwrap_or(0);
        let wanted = entries.len() - days.len();
        days.extend(std::iter::repeat_n(text, room.min(wanted)));
    }
    if days.len() < entries.len() {
        return Ok(false);
    }
    for (seq, day) in entries.iter().zip(&days) {
        sqlx::query!(
            "INSERT INTO allowance_units (account_id, day, entry_seq) VALUES (?, ?, ?)",
            account_id,
            day,
            seq
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(true)
}

/// The refusal for `count` entries that don't fit in the allowance's window.
pub fn refusal(count: usize) -> AppError {
    let what = if count == 1 {
        "this entry doesn't".to_string()
    } else {
        format!("these {count} entries don't")
    };
    AppError::Refused {
        status: StatusCode::TOO_MANY_REQUESTS,
        reasons: vec![format!(
            "{what} fit in the account's daily allowance over the next {WINDOW_DAYS} days \
             ({NEW_ACCOUNT_UNITS} a day in an account's first {NEW_ACCOUNT_DAYS} days, then {UNITS}); \
             send it again once some of its waiting entries have been screened"
        )],
    }
}

/// A unit a series took, which it gives back if the server turns it away.
pub struct SeriesUnit(Option<i64>);

/// Takes a unit dated today for a series or series version, which is screened at
/// once and so can't wait: past the day's units if need be, but no more than one
/// day's allowance of them for series. `None` when the account's series have taken
/// that many today; the caller refuses the request with [`series_refusal`].
pub async fn take_for_series(
    state: &AppState,
    account_id: &str,
    today: Date,
) -> Result<Option<SeriesUnit>, sqlx::Error> {
    if exempt(state, account_id) {
        return Ok(Some(SeriesUnit(None)));
    }
    let mut conn = state.pool.acquire().await?;
    let most = start_of(&mut conn, account_id, today)
        .await?
        .units_on(today);
    let day = day_text(today);
    let taken = sqlx::query_scalar!(
        r#"INSERT INTO allowance_units (account_id, day, kind)
           SELECT ?1, ?2, 'series' WHERE (SELECT COUNT(*) FROM allowance_units
                                WHERE account_id = ?1 AND day = ?2 AND kind = 'series') < ?3
           RETURNING seq AS "seq!: i64""#,
        account_id,
        day,
        most
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(taken.map(|seq| SeriesUnit(Some(seq))))
}

/// The refusal for a series past a day's allowance of series.
pub fn series_refusal() -> AppError {
    AppError::Refused {
        status: StatusCode::TOO_MANY_REQUESTS,
        reasons: vec![
            "the account's series have taken today's allowance; send it again tomorrow (UTC)"
                .into(),
        ],
    }
}

/// Gives back a series' unit, when the server turned the series away for its own
/// reasons: the day's screening budget spent, or the model unavailable.
pub async fn give_back_series(pool: &SqlitePool, unit: SeriesUnit) -> Result<(), sqlx::Error> {
    if let SeriesUnit(Some(seq)) = unit {
        sqlx::query!("DELETE FROM allowance_units WHERE seq = ?", seq)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Gives back the units entry `seq` took, in the caller's transaction: it was
/// rejected before it was screened, as a batch's other members are when one fails
/// (design.md, Agent interface).
pub async fn give_back(conn: &mut SqliteConnection, seq: i64) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM allowance_units WHERE entry_seq = ?", seq)
        .execute(conn)
        .await?;
    Ok(())
}

/// Deletes every unit `account_id` took, in the caller's transaction, as deleting
/// the account does before its entries go.
pub async fn delete_account(
    conn: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM allowance_units WHERE account_id = ?",
        account_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The day entry `seq` waits for before it is screened, if that is after `today`.
pub async fn screen_from(
    pool: &SqlitePool,
    seq: i64,
    today: Date,
) -> Result<Option<Date>, sqlx::Error> {
    let first = sqlx::query_scalar!(
        r#"SELECT day AS "day!" FROM entry_screen_from WHERE entry_seq = ?"#,
        seq
    )
    .fetch_optional(pool)
    .await?;
    Ok(first
        .and_then(|day| Date::parse(&day, DAY).ok())
        .filter(|day| *day > today))
}

/// The day the last of a batch's members `seqs` waits for, if any waits past `today`.
pub async fn last_screen_from(
    pool: &SqlitePool,
    seqs: &[i64],
    today: Date,
) -> Result<Option<Date>, sqlx::Error> {
    let mut last = None;
    for seq in seqs {
        last = last.max(screen_from(pool, *seq, today).await?);
    }
    Ok(last)
}

/// The lines a reply carries above the delimiter for an entry, or a batch, whose
/// screening waits until `screen_from`: the field, then what it means.
pub fn screen_from_lines(screen_from: Option<Date>) -> String {
    screen_from.map_or(String::new(), |day| {
        format!(
            "screen_from: {}\n\
             The account's daily allowance is taken until screen_from (UTC), so screening waits until then.\n",
            day_text(day)
        )
    })
}
