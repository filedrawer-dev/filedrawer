//! Changing an account's email address (design.md, Accounts; #125). A request, made
//! from a session that proved itself, waits in `email_changes` until the link
//! emailed to the new address is confirmed from a session of the same account; only
//! then does the account's address change. The row is the one place the new address
//! sits meanwhile: the mail queue's `change` row holds none, and reads this one when
//! it sends.

use sqlx::{SqliteConnection, SqlitePool};
use time::{Duration, OffsetDateTime};

use crate::accounts::EMAIL_LINK_HOURS;
use crate::mail_queue::{self, Queued};
use crate::time::{now, now_utc, stamp};
use crate::{AppState, db, ids};

/// Records that `account_id` asked to change its address to `new_email`, replacing
/// any change it had waiting, and queues the emails: a notice to its current
/// address, and the link to the new one, or, when another account holds that
/// address, the note signup sends an address that has an account. The caller has
/// checked who asks and counted the request.
pub async fn request(
    state: &AppState,
    account_id: &str,
    new_email: &str,
) -> Result<(), sqlx::Error> {
    let at = now_utc();
    let (created_at, expires_at) = (stamp(at), stamp(at + Duration::hours(EMAIL_LINK_HOURS)));
    let mut tx = db::write_tx(&state.pool).await?;
    sqlx::query!(
        "INSERT INTO email_changes (account_id, new_email, created_at, expires_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (account_id) DO UPDATE
         SET new_email = excluded.new_email, link_hash = NULL,
             created_at = excluded.created_at, expires_at = excluded.expires_at",
        account_id,
        new_email,
        created_at,
        expires_at
    )
    .execute(&mut *tx)
    .await?;
    mail_queue::queue_in(&mut tx, account_id, &Queued::ChangeNotice, at).await?;
    match holder(&mut tx, account_id, new_email).await? {
        Some(other) => mail_queue::queue_in(&mut tx, &other, &Queued::Taken, at).await?,
        None => mail_queue::queue_in(&mut tx, account_id, &Queued::Change, at).await?,
    }
    tx.commit().await?;
    mail_queue::wake(state);
    Ok(())
}

/// The account other than `account_id` that has `email`, if any.
async fn holder(
    conn: &mut SqliteConnection,
    account_id: &str,
    email: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM accounts WHERE email = ? AND id != ?"#,
        email,
        account_id
    )
    .fetch_optional(conn)
    .await
}

/// The address `account_id` asked to change to, while its change waits.
pub async fn pending(pool: &SqlitePool, account_id: &str) -> Result<Option<String>, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    pending_in(&mut conn, account_id).await
}

/// [`pending`] inside the caller's transaction; the mail queue's notice goes only
/// while a change waits.
pub(crate) async fn pending_in(
    conn: &mut SqliteConnection,
    account_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let now = now();
    sqlx::query_scalar!(
        "SELECT new_email FROM email_changes WHERE account_id = ? AND expires_at > ?",
        account_id,
        now
    )
    .fetch_optional(conn)
    .await
}

/// The address the emailed link with `secret` would give `account_id`, if the link
/// works: it is the account's, unexpired, and the last one sent.
pub async fn linked_address(
    pool: &SqlitePool,
    account_id: &str,
    secret: &str,
) -> Result<Option<String>, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    linked_address_in(&mut conn, account_id, secret).await
}

async fn linked_address_in(
    conn: &mut SqliteConnection,
    account_id: &str,
    secret: &str,
) -> Result<Option<String>, sqlx::Error> {
    let hash = ids::token_hash(secret);
    let now = now();
    sqlx::query_scalar!(
        "SELECT new_email FROM email_changes
         WHERE account_id = ? AND link_hash = ? AND expires_at > ?",
        account_id,
        hash,
        now
    )
    .fetch_optional(conn)
    .await
}

/// What pressing the confirm page's button did.
pub enum Confirmed {
    /// The account's address is now this one.
    Done(String),
    /// The link is used, expired, replaced, or another account's.
    Dead,
    /// Another account took the address since the link was sent.
    Taken,
}

/// Changes `account_id`'s address through its emailed link, in one transaction: the
/// address is the new one and verified, since the link reached it; every unused
/// verify and reset link of the account is spent and its waiting verify, reset,
/// taken and change emails are dropped, since they were for the old inbox; and the pending
/// change is gone. Sessions, tokens and a linked GitHub account stay: whoever asked
/// proved themselves, and holds a session now.
pub async fn confirm(
    pool: &SqlitePool,
    account_id: &str,
    secret: &str,
) -> Result<Confirmed, sqlx::Error> {
    let mut tx = db::write_tx(pool).await?;
    let Some(new_email) = linked_address_in(&mut tx, account_id, secret).await? else {
        return Ok(Confirmed::Dead);
    };
    cancel_in(&mut tx, account_id).await?;
    if holder(&mut tx, account_id, &new_email).await?.is_some() {
        tx.commit().await?;
        return Ok(Confirmed::Taken);
    }
    let at = now();
    sqlx::query!(
        "UPDATE accounts SET email = ?, email_verified_at = ? WHERE id = ?",
        new_email,
        at,
        account_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE email_tokens SET used_at = ? WHERE account_id = ? AND used_at IS NULL",
        at,
        account_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM outgoing_mail
         WHERE account_id = ? AND dropped_at IS NULL
           AND kind IN ('verify', 'reset', 'taken', 'change', 'change_notice')",
        account_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Confirmed::Done(new_email))
}

/// Ends `account_id`'s pending change, if it has one, and with it its link.
pub async fn cancel(pool: &SqlitePool, account_id: &str) -> Result<(), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    cancel_in(&mut conn, account_id).await
}

/// [`cancel`] inside the caller's transaction: a password reset and the account's
/// deletion end the change too.
pub async fn cancel_in(conn: &mut SqliteConnection, account_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM email_changes WHERE account_id = ?", account_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Deletes the pending changes that expired by `now`, for the hourly tidy.
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let now = stamp(now);
    sqlx::query!("DELETE FROM email_changes WHERE expires_at <= ?", now)
        .execute(pool)
        .await?;
    Ok(())
}

/// A link the mail queue is about to send `account_id`'s new address.
pub(crate) struct Link {
    pub to: String,
    pub secret: String,
    pub hash: Vec<u8>,
    /// When the link stops working, as the pending change's row has it.
    pub expires_at: String,
}

/// Mints the link for `account_id`'s waiting change, replacing any sent before;
/// none when no change waits or another account holds the address by now.
pub(crate) async fn mint_link(
    conn: &mut SqliteConnection,
    account_id: &str,
) -> Result<Option<Link>, sqlx::Error> {
    let Some(to) = pending_in(conn, account_id).await? else {
        return Ok(None);
    };
    if holder(conn, account_id, &to).await?.is_some() {
        return Ok(None);
    }
    let secret = ids::new_secret();
    let hash = ids::token_hash(&secret);
    let expires_at = sqlx::query_scalar!(
        "UPDATE email_changes SET link_hash = ? WHERE account_id = ? RETURNING expires_at",
        hash,
        account_id
    )
    .fetch_one(conn)
    .await?;
    Ok(Some(Link {
        to,
        secret,
        hash,
        expires_at,
    }))
}

/// Retires the link with `hash`, whose email never went out; a link minted since
/// stays.
pub(crate) async fn retire_link(pool: &SqlitePool, hash: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE email_changes SET link_hash = NULL WHERE link_hash = ?",
        hash
    )
    .execute(pool)
    .await?;
    Ok(())
}
