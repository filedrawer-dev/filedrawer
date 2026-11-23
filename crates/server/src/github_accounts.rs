//! GitHub sign-in's state (design.md, Accounts; #119): the sign-ins under way, which
//! the browser's cookie holds, and what the server keeps: signups waiting for their
//! form, each account's linked GitHub account, and which account each GitHub ID
//! first exempted from the new-account limit.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};
use time::{Duration, OffsetDateTime};

use crate::github_login::Identity;
use crate::ids;
use crate::time::{now, now_utc, stamp};

/// How long a sign-in under way and a signup waiting for its form last: GitHub's
/// codes last 10 minutes.
pub const START_MINUTES: i64 = 10;

/// How many sign-ins under way a browser's cookie keeps, so a few tabs each finish
/// their own; a start past that drops the oldest.
pub const STARTS_KEPT: usize = 3;

/// The most a start cookie's value may take, under the 4096 bytes a browser keeps
/// of a cookie's name and value, past which it drops the whole cookie: a start that
/// would pass it drops its own `next`, then the oldest starts.
pub const START_COOKIE_BYTES: usize = 3800;

/// How old a GitHub account must be to exempt its first account from the
/// new-account limit (design.md, Limits and abuse).
pub const EXEMPT_AFTER_DAYS: i64 = 90;

/// What a sign-in started for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    SignIn,
    /// Linking GitHub to the account of the session this tag is of, so a cookie
    /// planted in another browser can't link to that browser's account.
    Link(SessionTag),
    /// Confirming that the session this tag is of, which asked to change its
    /// account's address to `new_email`, holds the GitHub account linked to it, for
    /// an account with no password to confirm with (design.md, Accounts). Tagged
    /// like a link, so a planted cookie can't send another browser's account's
    /// change to an address of the planter's.
    ChangeEmail {
        session: SessionTag,
        new_email: String,
    },
}

/// What a link start keeps of the session that started it: a hash of its secret,
/// apart from the hash the sessions table keeps, which only that session's browser
/// can match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTag(String);

impl SessionTag {
    pub fn of(session_secret: &str) -> Self {
        let hash = Sha256::digest(format!("filedrawer github link {session_secret}"));
        SessionTag(URL_SAFE_NO_PAD.encode(hash))
    }
}

/// A round trip to GitHub under way, as its callback finds it.
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

/// A start as the cookie keeps it.
#[derive(Serialize, Deserialize)]
struct Kept {
    verifier: String,
    /// A link's or a change of address's [`SessionTag`]; none for a sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<String>,
    /// A change of address's new address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    new_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next: Option<String>,
    /// When it expires, in Unix seconds.
    expires: i64,
}

/// A start's state: a hash of its verifier, apart from PKCE's challenge.
fn state_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(format!(
        "filedrawer github state {verifier}"
    )))
}

/// The starts a cookie's value keeps, as base64url of their JSON, so a `next` page
/// can't break the cookie's syntax; none for a garbled one.
fn decode(cookie: &str) -> Vec<Kept> {
    URL_SAFE_NO_PAD
        .decode(cookie)
        .ok()
        .and_then(|json| serde_json::from_slice(&json).ok())
        .unwrap_or_default()
}

fn encode(kept: &[Kept]) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(kept).expect("starts serialize"))
}

/// The browser's start cookie with a new start for `purpose` added, and the new
/// start's state and verifier. The cookie keeps no state of its own: a start's
/// state is a hash of its verifier, so a cookie written around a state someone saw,
/// in a callback link, has no verifier that fits it. A garbled cookie, or none,
/// starts a fresh one; expired starts drop out.
pub fn add_start(
    cookie: Option<&str>,
    purpose: Purpose,
    next: Option<&str>,
    now: OffsetDateTime,
) -> (Started, String) {
    let now = now.unix_timestamp();
    let mut kept = cookie.map(decode).unwrap_or_default();
    kept.retain(|start| start.expires > now);
    let verifier = ids::new_secret();
    let (session, new_email) = match purpose {
        Purpose::SignIn => (None, None),
        Purpose::Link(SessionTag(tag)) => (Some(tag), None),
        Purpose::ChangeEmail {
            session: SessionTag(tag),
            new_email,
        } => (Some(tag), Some(new_email)),
    };
    kept.push(Kept {
        verifier: verifier.clone(),
        session,
        new_email,
        next: next.map(str::to_string),
        expires: now + START_MINUTES * 60,
    });
    kept.drain(..kept.len().saturating_sub(STARTS_KEPT));
    let mut value = encode(&kept);
    if value.len() > START_COOKIE_BYTES
        && let Some(newest) = kept.last_mut()
    {
        newest.next = None;
        value = encode(&kept);
    }
    while value.len() > START_COOKIE_BYTES && kept.len() > 1 {
        kept.remove(0);
        value = encode(&kept);
    }
    let started = Started {
        state: state_of(&verifier),
        verifier,
    };
    (started, value)
}

/// The live start in `cookie` whose state is `state`.
pub fn find_start(cookie: &str, state: &str, now: OffsetDateTime) -> Option<Start> {
    let now = now.unix_timestamp();
    let start = decode(cookie)
        .into_iter()
        .find(|start| start.expires > now && state_of(&start.verifier) == state)?;
    let purpose = match (start.session, start.new_email) {
        (None, None) => Purpose::SignIn,
        (Some(tag), None) => Purpose::Link(SessionTag(tag)),
        (Some(tag), Some(new_email)) => Purpose::ChangeEmail {
            session: SessionTag(tag),
            new_email,
        },
        (None, Some(_)) => return None,
    };
    Some(Start {
        purpose,
        verifier: start.verifier,
        next: start.next,
    })
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
    unlink_in(&mut *pool.acquire().await?, account_id).await
}

/// [`unlink`] in the caller's transaction, such as a password reset's. The
/// exemption record stays, so the GitHub ID exempts no other account.
pub async fn unlink_in(conn: &mut SqliteConnection, account_id: &str) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!("DELETE FROM github_links WHERE account_id = ?", account_id)
        .execute(conn)
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
    unlink_in(&mut *conn, account_id).await?;
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

/// How the hourly refresh went.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    pub asked: usize,
    /// Whether it stopped early, for GitHub's limits or a failure to answer.
    pub stopped: bool,
}

/// Asks GitHub about each linked account that has an entry, in any state, longest
/// unconfirmed first, so cards show its current login (the owner's decisions,
/// 2026-11-22). A 304 or a 200 confirms the login; a 404 hides the handle until a
/// later answer; anything else stops the run, and the next hour goes on.
pub async fn refresh_logins(
    state: &crate::AppState,
    at: OffsetDateTime,
) -> Result<Refreshed, sqlx::Error> {
    let mut done = Refreshed::default();
    let links = sqlx::query!(
        "SELECT l.github_id, l.etag FROM github_links l
         WHERE EXISTS (SELECT 1 FROM entries e WHERE e.account_id = l.account_id)
         ORDER BY l.refreshed_at, l.github_id"
    )
    .fetch_all(&state.pool)
    .await?;
    for link in links {
        match refresh_one(state, link.github_id, link.etag.as_deref(), at).await? {
            Some(()) => done.asked += 1,
            None => {
                done.stopped = true;
                break;
            }
        }
    }
    Ok(done)
}

/// Refreshes `account_id`'s login as an entry of it publishes, so its first card
/// shows a login GitHub just confirmed, unless GitHub confirmed it within the hour.
/// A failure is logged: the hourly refresh comes next.
pub async fn refresh_at_publish(state: &crate::AppState, account_id: &str) {
    let hour_ago = stamp(now_utc() - Duration::hours(1));
    let link = sqlx::query!(
        "SELECT github_id, etag FROM github_links WHERE account_id = ? AND refreshed_at <= ?",
        account_id,
        hour_ago
    )
    .fetch_optional(&state.pool)
    .await;
    let refreshed = match link {
        Ok(Some(link)) => refresh_one(state, link.github_id, link.etag.as_deref(), now_utc())
            .await
            .map(|_| ()),
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = refreshed {
        tracing::warn!(%error, "refreshing a GitHub login at publish failed");
    }
}

/// Asks GitHub about one linked account and keeps what it says. None when the
/// refresh must stop for now ([`crate::link_checks::UserAnswer::Wait`]).
async fn refresh_one(
    state: &crate::AppState,
    github_id: i64,
    etag: Option<&str>,
    at: OffsetDateTime,
) -> Result<Option<()>, sqlx::Error> {
    use crate::link_checks::UserAnswer;
    let now = stamp(at);
    match crate::link_checks::github_user(state, github_id, etag).await {
        UserAnswer::Unchanged => {
            sqlx::query!(
                "UPDATE github_links SET refreshed_at = ?, hidden = 0 WHERE github_id = ?",
                now,
                github_id
            )
            .execute(&state.pool)
            .await?;
        }
        UserAnswer::Changed { login, etag } => {
            sqlx::query!(
                "UPDATE github_links SET login = ?, etag = ?, refreshed_at = ?, hidden = 0
                 WHERE github_id = ?",
                login,
                etag,
                now,
                github_id
            )
            .execute(&state.pool)
            .await?;
        }
        UserAnswer::NotFound => {
            sqlx::query!(
                "UPDATE github_links SET hidden = 1 WHERE github_id = ?",
                github_id
            )
            .execute(&state.pool)
            .await?;
        }
        UserAnswer::Unreadable => {}
        UserAnswer::Wait => return Ok(None),
    }
    Ok(Some(()))
}

/// Deletes signups that expired, the expired round trips an older binary kept in
/// `github_starts`, with the address a change of address's start holds, before the
/// browser's cookie held them (#127), and the links of accounts an older binary
/// deleted without them (crates/server/migration-rollback.txt, 0038), for the hourly
/// tidy.
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
