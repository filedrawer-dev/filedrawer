//! The read counter behind the home page's Most read tab (design.md, Agent interface;
//! #107): how many different readers fetched each entry, per UTC day, through the
//! skills (`agent`) or as a page in a browser (`page`).
//!
//! A reader is an account, when the request carries a token or a session, or else
//! the client's address with its User-Agent. It is stored only as a hash salted with
//! a key that lives in memory, never in the database, so neither the database nor its
//! backups can tell who read what; the hourly tidier keeps only counts once a day
//! ends. A restart makes a new key, so a reader may count twice on that day.

use std::collections::HashSet;
use std::sync::Mutex;

use axum::http::{HeaderMap, header};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use time::{Date, Duration, OffsetDateTime};

use crate::auth::bearer_token;
use crate::db;
use crate::limits::ClientIp;
use crate::respond::wants_page;
use crate::search_log::SKILL_CLIENT;

/// The most readers counted in one UTC day, across entries and kinds; reads past it
/// go uncounted, and the day's `dropped` counts them.
pub const DAILY_CAP: i64 = 10_000;

/// How many days Most read ranks by: today and the 29 before it.
pub const WINDOW_DAYS: i64 = 30;

/// How many days the days' totals are kept, for the owner's daily email, as long as
/// the search log's rows.
const DAYS_KEPT: i64 = 90;

/// How long after midnight UTC a day stays open, for a read that began just before.
const GRACE: Duration = Duration::minutes(10);

/// The first day of Most read's window on `today`.
pub fn window_start(today: Date) -> Date {
    today - Duration::days(WINDOW_DAYS - 1)
}

/// Today's salt and the readers counted today, in memory only.
#[derive(Debug)]
pub struct ReadCounter {
    cap: i64,
    today: Mutex<Option<Today>>,
}

#[derive(Debug)]
struct Today {
    day: Date,
    salt: [u8; 32],
    seen: HashSet<[u8; 32]>,
}

impl ReadCounter {
    /// A counter that takes at most `cap` readers a day.
    pub fn new(cap: i64) -> ReadCounter {
        ReadCounter {
            cap,
            today: Mutex::new(None),
        }
    }

    /// The most readers it takes a day.
    pub fn cap(&self) -> i64 {
        self.cap
    }

    /// The hash that stands for `reader` of `entry_seq` as `kind` on `day`, or `None`
    /// when this server has already counted it today.
    fn first_read(&self, day: Date, entry_seq: i64, kind: &str, reader: &str) -> Option<[u8; 32]> {
        let mut today = self
            .today
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if today.as_ref().is_none_or(|today| today.day != day) {
            let mut salt = [0u8; 32];
            rand::fill(&mut salt[..]);
            *today = Some(Today {
                day,
                salt,
                seen: HashSet::new(),
            });
        }
        let today = today.as_mut().expect("set above");
        let mut hash = Sha256::new();
        hash.update(today.salt);
        hash.update(entry_seq.to_be_bytes());
        hash.update(kind.as_bytes());
        hash.update([0]);
        hash.update(reader.as_bytes());
        let hash: [u8; 32] = hash.finalize().into();
        today.seen.insert(hash).then_some(hash)
    }
}

impl Default for ReadCounter {
    fn default() -> ReadCounter {
        ReadCounter::new(DAILY_CAP)
    }
}

/// What a read of an entry is, if it counts: by the skills, or a page a person opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Agent,
    Page,
}

impl Kind {
    /// The kind of a GET with `headers`, or `None` for a client that doesn't count: a
    /// tool other than the skills, or a browser's prefetch or prerender. A browser
    /// opening `/e/<id>.txt` counts as a page.
    pub fn of(headers: &HeaderMap) -> Option<Kind> {
        if user_agent(headers).starts_with(SKILL_CLIENT) {
            return Some(Kind::Agent);
        }
        let prefetch = headers.contains_key("sec-purpose")
            || headers
                .get("purpose")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("prefetch"));
        (wants_page(headers) && bearer_token(headers).is_none() && !prefetch).then_some(Kind::Page)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Agent => "agent",
            Kind::Page => "page",
        }
    }
}

/// Who read, for counting: the account behind the request's token or session, or
/// else the client's address and User-Agent, so people on one address with different
/// tools count apart.
pub fn reader(account: Option<&str>, ip: &ClientIp, headers: &HeaderMap) -> String {
    match account {
        Some(account) => format!("account {account}"),
        None => format!("ip {} {}", ip.0, user_agent(headers)),
    }
}

/// The request's User-Agent, or empty when it has none readable.
fn user_agent(headers: &HeaderMap) -> &str {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

/// Whether `account`'s reads count: not an admin's, nor a suspended account's.
pub async fn counts_as_reader(pool: &SqlitePool, account: &str) -> Result<bool, sqlx::Error> {
    let excluded = sqlx::query_scalar!(
        r#"SELECT is_admin = 1 OR suspended_at IS NOT NULL AS "excluded!: bool"
           FROM accounts WHERE id = ?"#,
        account
    )
    .fetch_optional(pool)
    .await?;
    Ok(excluded == Some(false))
}

/// Counts one read of `entry_seq` by `reader` at `now`, unless it was counted today
/// already. Past the day's cap the read goes uncounted, and the day's `dropped`
/// counts it.
pub async fn record(
    pool: &SqlitePool,
    counter: &ReadCounter,
    entry_seq: i64,
    kind: Kind,
    reader: &str,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let kind = kind.name();
    let day = now.date();
    let Some(hash) = counter.first_read(day, entry_seq, kind, reader) else {
        return Ok(());
    };
    let day = day.to_string();
    let hash = hash.to_vec();
    let mut tx = db::write_tx(pool).await?;
    let counted = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM entry_readers WHERE day = ?"#,
        day
    )
    .fetch_one(&mut *tx)
    .await?;
    if counted < counter.cap {
        sqlx::query!(
            "INSERT OR IGNORE INTO entry_readers (entry_seq, day, kind, reader)
             VALUES (?, ?, ?, ?)",
            entry_seq,
            day,
            kind,
            hash
        )
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query!(
            "INSERT INTO read_days (day, dropped) VALUES (?, 1)
             ON CONFLICT (day) DO UPDATE SET dropped = dropped + 1",
            day
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Closes each day that ended more than ten minutes before `now`, oldest first:
/// adds its readers into each entry's daily counts and the day's total, and deletes
/// its rows. Then adds each daily count that left Most read's window into its month,
/// and deletes it. Each step is one transaction, so a run cut short or run twice
/// counts nothing twice.
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let open = (now - GRACE).date().to_string();
    let days = sqlx::query_scalar!(
        "SELECT DISTINCT day FROM entry_readers WHERE day < ? ORDER BY day",
        open
    )
    .fetch_all(pool)
    .await?;
    for day in days {
        let mut tx = db::write_tx(pool).await?;
        sqlx::query!(
            "INSERT INTO entry_reads_daily (entry_seq, day, kind, readers)
             SELECT entry_seq, day, kind, COUNT(*) FROM entry_readers WHERE day = ?1
             GROUP BY entry_seq, kind
             ON CONFLICT (entry_seq, day, kind) DO UPDATE SET readers = readers + excluded.readers",
            day
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO read_days (day, readers)
             SELECT ?1, COUNT(*) FROM entry_readers WHERE day = ?1
             ON CONFLICT (day) DO UPDATE SET readers = readers + excluded.readers",
            day
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM entry_readers WHERE day = ?", day)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    let start = window_start(now.date()).to_string();
    let mut tx = db::write_tx(pool).await?;
    sqlx::query!(
        "INSERT INTO entry_reads_monthly (entry_seq, month, kind, readers)
         SELECT entry_seq, substr(day, 1, 7), kind, SUM(readers) FROM entry_reads_daily
         WHERE day < ?1 GROUP BY entry_seq, substr(day, 1, 7), kind
         ON CONFLICT (entry_seq, month, kind) DO UPDATE SET readers = readers + excluded.readers",
        start
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM entry_reads_daily WHERE day < ?", start)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let before = (now.date() - Duration::days(DAYS_KEPT)).to_string();
    sqlx::query!("DELETE FROM read_days WHERE day < ?", before)
        .execute(pool)
        .await?;
    Ok(())
}
