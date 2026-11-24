//! Claiming an account an agent made (design.md, Accounts; #130). The agent names
//! an address with its token and gets a code to show its human; the address gets a
//! link. The claim waits in `account_claims` until the link's page is given the
//! code, and only then does the account get the address, a name and a password. The
//! row is the one place the address sits meanwhile: the mail queue's `claim` row
//! holds none, and reads this one when it sends.

use sqlx::{SqliteConnection, SqlitePool};
use time::{Date, Duration, OffsetDateTime};

use crate::accounts::EMAIL_LINK_HOURS;
use crate::mail_queue::{self, Queued};
use crate::time::{now, now_utc, stamp};
use crate::{AppState, allowance, db, ids};

/// The wrong codes a claim's page takes before the claim is gone, so the code can't
/// be guessed by whoever holds only the link.
const WRONG_CODES: i64 = 5;

/// What the agent gets back for its human.
pub struct Asked {
    /// The code to type on the link's page, as `xxxx-xxxx`.
    pub code: String,
    pub expires_at: String,
}

/// A code as stored and compared: its letters and digits, lower-cased, with the
/// letters a person may type for a 0 or a 1 read as those, which the codes'
/// alphabet leaves out for that; so spaces, the dash, capitals and a retyped `O`
/// or `l` don't matter.
fn code_hash(code: &str) -> Vec<u8> {
    let typed: String = code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| match c.to_ascii_lowercase() {
            'o' => '0',
            'i' | 'l' => '1',
            other => other,
        })
        .collect();
    ids::token_hash(&typed)
}

/// Records that unclaimed `account_id`'s agent named `email` as its owner's,
/// replacing any claim it had waiting, and queues the link to that address, whether
/// or not another account has it. The caller has checked who asks and counted the
/// request. None, with nothing recorded, when the account isn't unclaimed by now.
pub async fn request(
    state: &AppState,
    account_id: &str,
    email: &str,
) -> Result<Option<Asked>, sqlx::Error> {
    let at = now_utc();
    let (created_at, expires_at) = (stamp(at), stamp(at + Duration::hours(EMAIL_LINK_HOURS)));
    let code = ids::new_claim_code();
    let hash = code_hash(&code);
    let mut tx = db::write_tx(&state.pool).await?;
    // Checked with the write, so a claim confirmed a moment ago leaves no new one.
    let unclaimed = sqlx::query_scalar!(
        r#"SELECT 1 AS "found!: i64" FROM accounts
           WHERE id = ? AND unclaimed AND deleted_at IS NULL"#,
        account_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if unclaimed.is_none() {
        return Ok(None);
    }
    sqlx::query!(
        "INSERT INTO account_claims (account_id, email, code_hash, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (account_id) DO UPDATE
         SET email = excluded.email, code_hash = excluded.code_hash, link_hash = NULL,
             wrong_codes = 0, created_at = excluded.created_at,
             expires_at = excluded.expires_at",
        account_id,
        email,
        hash,
        created_at,
        expires_at
    )
    .execute(&mut *tx)
    .await?;
    mail_queue::queue_in(&mut tx, account_id, &Queued::Claim, at).await?;
    tx.commit().await?;
    mail_queue::wake(state);
    Ok(Some(Asked { code, expires_at }))
}

/// A pending claim, found by its link.
pub struct Claim {
    pub account_id: String,
    /// The entries the account holds, in any state, for the link's page.
    pub entries: i64,
    email: String,
    code_hash: Vec<u8>,
    wrong_codes: i64,
}

/// The claim the emailed link with `secret` leads to, if the link works: unexpired,
/// the last one sent, and its account still unclaimed and not suspended.
pub async fn by_link(pool: &SqlitePool, secret: &str) -> Result<Option<Claim>, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    by_link_in(&mut conn, secret).await
}

async fn by_link_in(
    conn: &mut SqliteConnection,
    secret: &str,
) -> Result<Option<Claim>, sqlx::Error> {
    let hash = ids::token_hash(secret);
    let now = now();
    sqlx::query_as!(
        Claim,
        r#"SELECT c.account_id, c.email, c.code_hash, c.wrong_codes,
                  (SELECT COUNT(*) FROM entries e WHERE e.account_id = c.account_id)
                      AS "entries!: i64"
           FROM account_claims c JOIN accounts a ON a.id = c.account_id
           WHERE c.link_hash = ? AND c.expires_at > ? AND a.unclaimed
             AND a.deleted_at IS NULL AND a.suspended_at IS NULL"#,
        hash,
        now
    )
    .fetch_optional(conn)
    .await
}

/// Why the link's page claimed nothing.
pub enum Refused {
    /// The link is used, expired or replaced, or its claim ended, as it does at the
    /// fifth wrong code.
    Dead,
    /// Not the code the agent was given; counted.
    Wrong,
    /// Another account has the address.
    Taken,
}

/// The claim behind `secret`, if `code` is its code and its address is free. Runs
/// in the caller's transaction, which it must commit: a wrong code is counted, and
/// the fifth deletes the claim.
async fn checked_in(
    conn: &mut SqliteConnection,
    secret: &str,
    code: &str,
) -> Result<Result<Claim, Refused>, sqlx::Error> {
    let Some(claim) = by_link_in(conn, secret).await? else {
        return Ok(Err(Refused::Dead));
    };
    if claim.code_hash != code_hash(code) {
        if claim.wrong_codes + 1 >= WRONG_CODES {
            cancel_in(conn, &claim.account_id).await?;
            return Ok(Err(Refused::Dead));
        }
        sqlx::query!(
            "UPDATE account_claims SET wrong_codes = wrong_codes + 1 WHERE account_id = ?",
            claim.account_id
        )
        .execute(&mut *conn)
        .await?;
        return Ok(Err(Refused::Wrong));
    }
    let taken = sqlx::query_scalar!(
        r#"SELECT 1 AS "found!: i64" FROM accounts WHERE email = ?"#,
        claim.email
    )
    .fetch_optional(&mut *conn)
    .await?;
    if taken.is_some() {
        return Ok(Err(Refused::Taken));
    }
    Ok(Ok(claim))
}

/// Checks `code` for the link's page before the password is hashed, counting a
/// wrong one.
pub async fn check(
    pool: &SqlitePool,
    secret: &str,
    code: &str,
) -> Result<Result<(), Refused>, sqlx::Error> {
    let mut tx = db::write_tx(pool).await?;
    let checked = checked_in(&mut tx, secret, code).await?;
    tx.commit().await?;
    Ok(checked.map(|_| ()))
}

/// Claims the account behind `secret`, in one transaction: it gets the claim's
/// address, verified, since the link reached it, the name and the password; the
/// claim is gone; and its waiting entries move up to the days its new allowance has
/// room on. Its tokens stay, since the code shows the agent that holds one and the
/// person at the inbox work together. Returns the account's ID.
pub async fn confirm(
    pool: &SqlitePool,
    secret: &str,
    code: &str,
    display_name: &str,
    password_hash: &str,
    today: Date,
) -> Result<Result<String, Refused>, sqlx::Error> {
    let mut tx = db::write_tx(pool).await?;
    let claim = match checked_in(&mut tx, secret, code).await? {
        Ok(claim) => claim,
        Err(refused) => {
            tx.commit().await?;
            return Ok(Err(refused));
        }
    };
    let at = now();
    sqlx::query!(
        "UPDATE accounts
         SET email = ?, email_verified_at = ?, claimed_at = ?, password_hash = ?,
             display_name = ?
         WHERE id = ?",
        claim.email,
        at,
        at,
        password_hash,
        display_name,
        claim.account_id
    )
    .execute(&mut *tx)
    .await?;
    cancel_in(&mut tx, &claim.account_id).await?;
    allowance::move_up(&mut tx, &claim.account_id, today).await?;
    tx.commit().await?;
    Ok(Ok(claim.account_id))
}

/// Ends `account_id`'s pending claim, if it has one, and with it its link and its
/// waiting email, in the caller's transaction.
pub async fn cancel_in(conn: &mut SqliteConnection, account_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM account_claims WHERE account_id = ?",
        account_id
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "DELETE FROM outgoing_mail
         WHERE account_id = ? AND dropped_at IS NULL AND kind = 'claim'",
        account_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Deletes the pending claims that expired by `now`, for the hourly tidy.
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let now = stamp(now);
    sqlx::query!("DELETE FROM account_claims WHERE expires_at <= ?", now)
        .execute(pool)
        .await?;
    Ok(())
}

/// A link the mail queue is about to send to the address a claim names.
pub(crate) struct Link {
    pub to: String,
    pub secret: String,
    pub hash: Vec<u8>,
    /// When the link stops working, as the claim's row has it.
    pub expires_at: String,
}

/// Mints the link for `account_id`'s waiting claim, replacing any sent before; none
/// when no claim waits. It goes whether or not another account holds the address,
/// so the email never says which.
pub(crate) async fn mint_link(
    conn: &mut SqliteConnection,
    account_id: &str,
) -> Result<Option<Link>, sqlx::Error> {
    let secret = ids::new_secret();
    let hash = ids::token_hash(&secret);
    let now = now();
    let row = sqlx::query!(
        "UPDATE account_claims SET link_hash = ? WHERE account_id = ? AND expires_at > ?
         RETURNING email, expires_at",
        hash,
        account_id,
        now
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| Link {
        to: row.email,
        secret,
        hash,
        expires_at: row.expires_at,
    }))
}

/// Retires the link with `hash`, whose email never went out; a link minted since
/// stays.
pub(crate) async fn retire_link(pool: &SqlitePool, hash: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE account_claims SET link_hash = NULL WHERE link_hash = ?",
        hash
    )
    .execute(pool)
    .await?;
    Ok(())
}
