//! Connecting an agent from the browser with a code, so nobody copies a token
//! (design.md, Accounts; #54), after RFC 8628's device authorization grant. The agent
//! asks for a code with no token; its human types the user code on `/device`, logged
//! in, and approves; the agent's next poll with the device code makes the token and
//! takes it. A request waits in `device_requests`, which holds only the codes'
//! hashes and never a token.

use sqlx::{SqliteConnection, SqlitePool};
use time::{Duration, OffsetDateTime};

use crate::ids::{self, USER_CODE_LETTERS};
use crate::time::{now_utc, parse, stamp};

/// How long a code works, from the request: long enough to sign up and verify an
/// address on the way, short enough that a code sent to someone else soon dies
/// (RFC 8628, section 5.4).
pub const EXPIRES_MINUTES: i64 = 15;
/// The seconds an agent waits between polls at first (RFC 8628, section 3.2).
pub const INTERVAL_SECS: i64 = 5;
/// What `slow_down` adds to the interval (RFC 8628, section 3.5).
const SLOW_DOWN_SECS: i64 = 5;
/// The most `slow_down` raises the interval to.
const MAX_INTERVAL_SECS: i64 = 60;
/// How much of the agent's User-Agent the approval page shows.
const USER_AGENT_CHARS: usize = 200;
/// The token's name when the agent gives none.
pub const DEFAULT_NAME: &str = "agent";

/// A user code as stored and compared: its letters upper-cased, with anything
/// outside the codes' alphabet left out, so the dash, spaces and lower case don't
/// matter (RFC 8628, section 6.1).
fn user_code_hash(typed: &str) -> Vec<u8> {
    ids::token_hash(&letters_of(typed))
}

fn letters_of(typed: &str) -> String {
    typed
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| u8::try_from(*c).is_ok_and(|byte| USER_CODE_LETTERS.contains(&byte)))
        .collect()
}

/// A typed user code as the pages show it: `XXXX-XXXX` once it has its 8 letters,
/// else its letters as typed.
pub fn shown(typed: &str) -> String {
    let letters = letters_of(typed);
    match (letters.get(..4), letters.get(4..)) {
        (Some(head), Some(tail)) if letters.len() == 8 => format!("{head}-{tail}"),
        _ => letters,
    }
}

/// What the agent gets back.
pub struct Asked {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: i64,
}

/// Records an agent's request for a token named `name`, with its User-Agent.
pub async fn ask(
    pool: &SqlitePool,
    name: &str,
    user_agent: Option<&str>,
) -> Result<Asked, sqlx::Error> {
    let at = now_utc();
    let (created_at, expires_at) = (stamp(at), stamp(at + Duration::minutes(EXPIRES_MINUTES)));
    let user_agent: Option<String> =
        user_agent.map(|ua| ua.chars().take(USER_AGENT_CHARS).collect());
    // Two waiting requests can't share a user code; a draw that collides, about one
    // in ten million while 2000 wait, draws again.
    loop {
        let device_code = ids::new_device_code();
        let user_code = ids::new_user_code();
        let (device_hash, code_hash) = (ids::token_hash(&device_code), user_code_hash(&user_code));
        let done = sqlx::query!(
            "INSERT INTO device_requests
                 (device_hash, user_code_hash, name, user_agent, created_at, expires_at,
                  interval_secs)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT DO NOTHING",
            device_hash,
            code_hash,
            name,
            user_agent,
            created_at,
            expires_at,
            INTERVAL_SECS
        )
        .execute(pool)
        .await?;
        if done.rows_affected() == 1 {
            return Ok(Asked {
                device_code,
                user_code,
                expires_in: EXPIRES_MINUTES * 60,
                interval: INTERVAL_SECS,
            });
        }
    }
}

/// A request waiting for its human, as the approval page shows it.
pub struct Waiting {
    pub name: String,
    pub user_agent: Option<String>,
    pub created_at: String,
}

/// The request the user code `typed` names, if it is waiting: unexpired, and
/// neither approved nor denied.
pub async fn waiting(pool: &SqlitePool, typed: &str) -> Result<Option<Waiting>, sqlx::Error> {
    let hash = user_code_hash(typed);
    let now = stamp(now_utc());
    sqlx::query_as!(
        Waiting,
        "SELECT name, user_agent, created_at FROM device_requests
         WHERE user_code_hash = ? AND expires_at > ?
           AND approved_at IS NULL AND denied_at IS NULL",
        hash,
        now
    )
    .fetch_optional(pool)
    .await
}

/// Approves the waiting request `typed` names for `account_id`, its token to be
/// named `name`. False when no such request waits.
pub async fn approve(
    pool: &SqlitePool,
    typed: &str,
    account_id: &str,
    name: &str,
) -> Result<bool, sqlx::Error> {
    let hash = user_code_hash(typed);
    let now = stamp(now_utc());
    let done = sqlx::query!(
        "UPDATE device_requests SET account_id = ?, approved_at = ?, name = ?
         WHERE user_code_hash = ? AND expires_at > ?
           AND approved_at IS NULL AND denied_at IS NULL",
        account_id,
        now,
        name,
        hash,
        now
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Denies the waiting request `typed` names. False when no such request waits.
pub async fn deny(pool: &SqlitePool, typed: &str) -> Result<bool, sqlx::Error> {
    let hash = user_code_hash(typed);
    let now = stamp(now_utc());
    let done = sqlx::query!(
        "UPDATE device_requests SET denied_at = ?
         WHERE user_code_hash = ? AND expires_at > ?
           AND approved_at IS NULL AND denied_at IS NULL",
        now,
        hash,
        now
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// What a poll finds, named as RFC 8628's section 3.5 names its answers.
pub enum Poll {
    /// No request has that device code, or it expired.
    Expired,
    /// The human denied it; the request is gone.
    Denied,
    /// Not yet approved: poll again after `interval` seconds.
    Pending { interval: i64 },
    /// Polled too soon: the interval is now `interval` seconds, for every later poll.
    SlowDown { interval: i64 },
    /// Approved: the token, made just now, and its account. The request is gone.
    Approved { account_id: String, token: String },
}

/// Answers a poll with `device_code`, in one transaction: an approved request
/// becomes its token and is deleted, so the token is shown once.
pub async fn poll(pool: &SqlitePool, device_code: &str) -> Result<Poll, sqlx::Error> {
    let hash = ids::token_hash(device_code.trim());
    let at = now_utc();
    let now = stamp(at);
    let mut tx = crate::db::write_tx(pool).await?;
    // An approval whose account was deleted since reads as no request; deletion
    // deletes it anyway.
    let row = sqlx::query!(
        r#"SELECT r.name, r.interval_secs, r.last_polled_at, r.account_id, r.approved_at,
                  r.denied_at,
                  (r.account_id IS NULL OR EXISTS (
                      SELECT 1 FROM accounts a WHERE a.id = r.account_id AND a.deleted_at IS NULL
                  )) AS "account_lives!: bool"
           FROM device_requests r WHERE r.device_hash = ? AND r.expires_at > ?"#,
        hash,
        now
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row.filter(|row| row.account_lives) else {
        return Ok(Poll::Expired);
    };
    if row.denied_at.is_some() {
        delete_in(&mut tx, &hash).await?;
        tx.commit().await?;
        return Ok(Poll::Denied);
    }
    if let (Some(account_id), Some(_)) = (row.account_id, row.approved_at) {
        let token = ids::new_token();
        let (token_id, token_hash) = (ids::new_id(), ids::token_hash(&token));
        sqlx::query!(
            "INSERT INTO tokens (id, account_id, name, hash, created_at, from_code_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            token_id,
            account_id,
            row.name,
            token_hash,
            now,
            now
        )
        .execute(&mut *tx)
        .await?;
        delete_in(&mut tx, &hash).await?;
        tx.commit().await?;
        return Ok(Poll::Approved { account_id, token });
    }
    // The stamps keep whole seconds, so a poll on time can read a second early.
    let too_soon = row
        .last_polled_at
        .as_deref()
        .and_then(parse)
        .is_some_and(|last| (at - last).whole_seconds() < row.interval_secs - 1);
    let interval = if too_soon {
        (row.interval_secs + SLOW_DOWN_SECS).min(MAX_INTERVAL_SECS)
    } else {
        row.interval_secs
    };
    sqlx::query!(
        "UPDATE device_requests SET last_polled_at = ?, interval_secs = ? WHERE device_hash = ?",
        now,
        interval,
        hash
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(if too_soon {
        Poll::SlowDown { interval }
    } else {
        Poll::Pending { interval }
    })
}

async fn delete_in(conn: &mut SqliteConnection, device_hash: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM device_requests WHERE device_hash = ?",
        device_hash
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Ends every request `account_id` approved whose agent hasn't taken its token
/// yet, in the caller's transaction: a password reset, which revokes every token,
/// and deletion end them too.
pub async fn cancel_approvals_in(
    conn: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM device_requests WHERE account_id = ?",
        account_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes the requests that expired by `now`, for the hourly tidy.
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let now = stamp(now);
    sqlx::query!("DELETE FROM device_requests WHERE expires_at <= ?", now)
        .execute(pool)
        .await?;
    Ok(())
}
