//! What the server keeps for GitHub sign-in (design.md, Accounts; #119): sign-ins
//! under way, signups waiting for their form, each account's linked GitHub account,
//! and which account each GitHub ID first exempted from the new-account limit.

use sqlx::{SqliteConnection, SqlitePool};
use time::{Duration, OffsetDateTime};

use crate::github_login::Identity;
use crate::ids;
use crate::time::{now, now_utc, stamp};

/// How long a sign-in under way and a signup waiting for its form last: GitHub's
/// codes last 10 minutes.
pub const START_MINUTES: i64 = 10;

/// The most sign-ins one client may have under way, and the most in all, so a flood
/// of starts can't fill the table.
pub const STARTS_PER_CLIENT: i64 = 20;
pub const STARTS_IN_ALL: i64 = 10_000;

/// How old a GitHub account must be to exempt its first account from the
/// new-account limit (design.md, Limits and abuse).
pub const EXEMPT_AFTER_DAYS: i64 = 90;

/// What a sign-in started for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    SignIn,
    /// Linking GitHub to this account, whose session started it.
    Link(String),
}

/// A sign-in under way, as its callback finds it.
pub struct Start {
    pub purpose: Purpose,
    pub verifier: String,
    pub next: Option<String>,
}

/// A started sign-in's state, sent to GitHub, and its PKCE verifier.
pub struct Started {
    pub state: String,
    pub verifier: String,
}

/// Records a sign-in under way for the browser holding `browser_secret`, from the
/// client with `client_key`. None when that client, or every client, has too many
/// under way.
pub async fn start(
    pool: &SqlitePool,
    browser_secret: &str,
    client_key: &str,
    purpose: &Purpose,
    next: Option<&str>,
) -> Result<Option<Started>, sqlx::Error> {
    let mut tx = crate::db::write_tx(pool).await?;
    let now = now();
    let client_hash = ids::token_hash(client_key);
    let live = sqlx::query!(
        r#"SELECT count(*) AS "in_all!: i64",
                  coalesce(sum(client_hash = ?), 0) AS "this_client!: i64"
           FROM github_starts WHERE expires_at > ?"#,
        client_hash,
        now
    )
    .fetch_one(&mut *tx)
    .await?;
    if live.this_client >= STARTS_PER_CLIENT || live.in_all >= STARTS_IN_ALL {
        return Ok(None);
    }
    let state = ids::new_secret();
    let verifier = ids::new_secret();
    let state_hash = ids::token_hash(&state);
    let browser_hash = ids::token_hash(browser_secret);
    let (purpose_text, account_id) = match purpose {
        Purpose::SignIn => ("sign_in", None),
        Purpose::Link(account) => ("link", Some(account.as_str())),
    };
    let expires_at = stamp(now_utc() + Duration::minutes(START_MINUTES));
    sqlx::query!(
        "INSERT INTO github_starts
           (state_hash, browser_hash, verifier, purpose, account_id, next, client_hash,
            created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        state_hash,
        browser_hash,
        verifier,
        purpose_text,
        account_id,
        next,
        client_hash,
        now,
        expires_at
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(Started { state, verifier }))
}

/// Spends the sign-in under way with `state`, if the browser holding
/// `browser_secret` started it and it hasn't expired. A state another browser
/// started stays, so a link that carries it can't spend it.
pub async fn take_start(
    pool: &SqlitePool,
    state: &str,
    browser_secret: &str,
) -> Result<Option<Start>, sqlx::Error> {
    let state_hash = ids::token_hash(state);
    let browser_hash = ids::token_hash(browser_secret);
    let now = now();
    let row = sqlx::query!(
        "DELETE FROM github_starts
         WHERE state_hash = ? AND browser_hash = ? AND expires_at > ?
         RETURNING purpose, account_id, verifier, next",
        state_hash,
        browser_hash,
        now
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|row| {
        let purpose = match (row.purpose.as_str(), row.account_id) {
            ("sign_in", _) => Purpose::SignIn,
            ("link", Some(account)) => Purpose::Link(account),
            _ => return None,
        };
        Some(Start {
            purpose,
            verifier: row.verifier,
            next: row.next,
        })
    }))
}

/// The live account `github_id` is linked to.
pub async fn account_of(pool: &SqlitePool, github_id: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT l.account_id FROM github_links l JOIN accounts a ON a.id = l.account_id
         WHERE l.github_id = ? AND a.deleted_at IS NULL",
        github_id
    )
    .fetch_optional(pool)
    .await
}

/// Keeps the linked account's login as GitHub just gave it, at a sign-in.
pub async fn note_login(pool: &SqlitePool, identity: &Identity) -> Result<(), sqlx::Error> {
    let now = now();
    sqlx::query!(
        "UPDATE github_links SET login = ?, refreshed_at = ?, hidden = 0 WHERE github_id = ?",
        identity.login,
        now,
        identity.github_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// The result of linking GitHub to an account.
#[derive(Debug, PartialEq, Eq)]
pub enum Linking {
    Done,
    /// Another account has this GitHub account.
    OtherAccount,
    /// This account has another GitHub account linked.
    HasAnother,
}

/// The live account `github_id` is linked to, in the caller's transaction, which
/// first deletes a link a deleted account kept, as an older binary's deletion
/// leaves them (crates/server/migration-rollback.txt, 0038).
async fn live_holder(
    conn: &mut SqliteConnection,
    github_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query!(
        "DELETE FROM github_links WHERE github_id = ?
           AND account_id IN (SELECT id FROM accounts WHERE deleted_at IS NOT NULL)",
        github_id
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query_scalar!(
        "SELECT account_id FROM github_links WHERE github_id = ?",
        github_id
    )
    .fetch_optional(&mut *conn)
    .await
}

/// Inserts the link and the exemption record (kept only for the first account
/// that ever linked the ID) in the caller's transaction.
async fn insert_link(
    conn: &mut SqliteConnection,
    account_id: &str,
    identity: &Identity,
) -> Result<(), sqlx::Error> {
    let now = now();
    sqlx::query!(
        "INSERT INTO github_links
           (account_id, github_id, login, github_created_at, linked_at, refreshed_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        account_id,
        identity.github_id,
        identity.login,
        identity.created_at,
        now,
        now
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "INSERT INTO github_exemptions (github_id, account_id) VALUES (?, ?)
         ON CONFLICT (github_id) DO NOTHING",
        identity.github_id,
        account_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Links `identity` to `account_id`, whose session finished the sign-in. The
/// account's address counts as verified when GitHub lists it as a verified one of
/// theirs, since the person holds both the session and the inbox.
pub async fn link(
    pool: &SqlitePool,
    account_id: &str,
    identity: &Identity,
) -> Result<Linking, sqlx::Error> {
    let mut tx = crate::db::write_tx(pool).await?;
    let holder = live_holder(&mut tx, identity.github_id).await?;
    match holder {
        Some(holder) if holder == account_id => {
            tx.rollback().await?;
            note_login(pool, identity).await?;
            return Ok(Linking::Done);
        }
        Some(_) => return Ok(Linking::OtherAccount),
        None => {}
    }
    let has = sqlx::query_scalar!(
        "SELECT github_id FROM github_links WHERE account_id = ?",
        account_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if has.is_some() {
        return Ok(Linking::HasAnother);
    }
    insert_link(&mut tx, account_id, identity).await?;
    let email = sqlx::query_scalar!("SELECT email FROM accounts WHERE id = ?", account_id)
        .fetch_one(&mut *tx)
        .await?;
    if email.is_some_and(|email| identity.emails.contains(&email)) {
        let at = now();
        sqlx::query!(
            "UPDATE accounts SET email_verified_at = ? WHERE id = ? AND email_verified_at IS NULL",
            at,
            account_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Linking::Done)
}

/// The account's linked GitHub login, as stored.
pub async fn login_of(pool: &SqlitePool, account_id: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT login FROM github_links WHERE account_id = ?",
        account_id
    )
    .fetch_optional(pool)
    .await
}

/// Removes the account's link. False when it had none.
pub async fn unlink(pool: &SqlitePool, account_id: &str) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!("DELETE FROM github_links WHERE account_id = ?", account_id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

/// Removes a deleted account's link and clears it from its exemption record, in the
/// deletion's transaction. The record keeps the bare GitHub ID, so the ID can't
/// exempt a second account (design.md, Accounts).
pub async fn forget_account(
    conn: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM github_links WHERE account_id = ?", account_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query!(
        "UPDATE github_exemptions SET account_id = NULL WHERE account_id = ?",
        account_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Whether `account_id` skips the new-account limit for its GitHub account: it is
/// linked to one at least [`EXEMPT_AFTER_DAYS`] old, and was the first account that
/// one ever linked.
pub async fn exempt(
    conn: &mut SqliteConnection,
    account_id: &str,
    at: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    let old_enough = stamp(at - Duration::days(EXEMPT_AFTER_DAYS));
    let found = sqlx::query_scalar!(
        "SELECT 1 FROM github_links l
         JOIN github_exemptions e ON e.github_id = l.github_id AND e.account_id = l.account_id
         WHERE l.account_id = ? AND l.github_created_at IS NOT NULL
           AND l.github_created_at <= ?",
        account_id,
        old_enough
    )
    .fetch_optional(conn)
    .await?;
    Ok(found.is_some())
}

/// Whether `identity`'s GitHub account is too young, or of unknown age, to skip the
/// per-IP signup limit (design.md, Limits and abuse).
pub fn young(identity: &Identity, at: OffsetDateTime) -> bool {
    let old_enough = stamp(at - Duration::days(EXEMPT_AFTER_DAYS));
    identity
        .created_at
        .as_ref()
        .is_none_or(|created| *created > old_enough)
}

/// A GitHub sign-in waiting for its finish-signup form.
pub struct Pending {
    pub identity: Identity,
    pub next: Option<String>,
}

/// Keeps `identity` for its finish-signup form and returns the key the browser
/// holds.
pub async fn hold_signup(
    pool: &SqlitePool,
    identity: &Identity,
    next: Option<&str>,
) -> Result<String, sqlx::Error> {
    let key = ids::new_secret();
    let key_hash = ids::token_hash(&key);
    let emails = serde_json::to_string(&identity.emails).expect("strings serialize");
    let now = now();
    let expires_at = stamp(now_utc() + Duration::minutes(START_MINUTES));
    sqlx::query!(
        "INSERT INTO github_signups
           (key_hash, github_id, login, name, github_created_at, emails, next, created_at,
            expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        key_hash,
        identity.github_id,
        identity.login,
        identity.name,
        identity.created_at,
        emails,
        next,
        now,
        expires_at
    )
    .execute(pool)
    .await?;
    Ok(key)
}

/// The signup waiting under `key`, if it hasn't expired.
pub async fn pending(pool: &SqlitePool, key: &str) -> Result<Option<Pending>, sqlx::Error> {
    read_pending(&mut *pool.acquire().await?, key).await
}

async fn read_pending(
    conn: &mut SqliteConnection,
    key: &str,
) -> Result<Option<Pending>, sqlx::Error> {
    let key_hash = ids::token_hash(key);
    let now = now();
    let row = sqlx::query!(
        "SELECT github_id, login, name, github_created_at, emails, next FROM github_signups
         WHERE key_hash = ? AND expires_at > ?",
        key_hash,
        now
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| Pending {
        identity: Identity {
            github_id: row.github_id,
            login: row.login,
            name: row.name,
            created_at: row.github_created_at,
            emails: serde_json::from_str(&row.emails).unwrap_or_default(),
        },
        next: row.next,
    }))
}

/// The result of finishing a GitHub signup.
pub enum Finished {
    /// The new account, verified and with no password.
    Created(String),
    /// This account has the address already, so nothing was made.
    Taken(String),
    /// Another account linked the GitHub account meanwhile.
    GithubTaken,
    /// The invite code given can't take a signup.
    BadInvite,
    /// The waiting signup is gone: used or expired.
    Gone,
}

/// Makes the account a waiting signup holds, with `email` (one GitHub listed) and
/// `display_name`, and links it, in one transaction that spends the signup.
pub async fn finish_signup(
    pool: &SqlitePool,
    key: &str,
    email: &str,
    display_name: &str,
    invite: Option<&str>,
) -> Result<Finished, sqlx::Error> {
    let mut tx = crate::db::write_tx(pool).await?;
    let key_hash = ids::token_hash(key);
    let now = now();
    let Some(Pending { identity, .. }) = read_pending(&mut tx, key).await? else {
        return Ok(Finished::Gone);
    };
    let invite_seq = match invite {
        None => None,
        Some(code) => match crate::invites::usable(&mut *tx, code, now_utc()).await? {
            Some(seq) => Some(seq),
            None => return Ok(Finished::BadInvite),
        },
    };
    sqlx::query!("DELETE FROM github_signups WHERE key_hash = ?", key_hash)
        .execute(&mut *tx)
        .await?;
    let taken = sqlx::query_scalar!("SELECT id FROM accounts WHERE email = ?", email)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(id) = taken {
        tx.commit().await?;
        return Ok(Finished::Taken(id));
    }
    if live_holder(&mut tx, identity.github_id).await?.is_some() {
        tx.commit().await?;
        return Ok(Finished::GithubTaken);
    }
    let id = ids::new_account_id();
    sqlx::query!(
        "INSERT INTO accounts
           (id, display_name, created_at, email, email_verified_at, invite_seq)
         VALUES (?, ?, ?, ?, ?, ?)",
        id,
        display_name,
        now,
        email,
        now,
        invite_seq
    )
    .execute(&mut *tx)
    .await?;
    insert_link(&mut tx, &id, &identity).await?;
    tx.commit().await?;
    Ok(Finished::Created(id))
}

/// Deletes sign-ins and signups that expired, and the links of accounts an older
/// binary deleted without them (crates/server/migration-rollback.txt, 0038), for the
/// hourly tidy.
pub async fn tidy(pool: &SqlitePool, at: OffsetDateTime) -> Result<(), sqlx::Error> {
    let now = stamp(at);
    sqlx::query!("DELETE FROM github_starts WHERE expires_at <= ?", now)
        .execute(pool)
        .await?;
    sqlx::query!("DELETE FROM github_signups WHERE expires_at <= ?", now)
        .execute(pool)
        .await?;
    sqlx::query!(
        "DELETE FROM github_links
         WHERE account_id IN (SELECT id FROM accounts WHERE deleted_at IS NOT NULL)"
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "UPDATE github_exemptions SET account_id = NULL
         WHERE account_id IN (SELECT id FROM accounts WHERE deleted_at IS NOT NULL)"
    )
    .execute(pool)
    .await?;
    Ok(())
}
