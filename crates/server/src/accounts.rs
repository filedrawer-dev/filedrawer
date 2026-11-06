//! Accounts with email and password, their sessions, their email links and their
//! tokens (design.md, Accounts).

use sqlx::{SqliteConnection, SqlitePool};
use time::Duration;

use crate::{ids, time::now, time::now_utc, time::stamp};

/// How long a login lasts.
pub const SESSION_DAYS: i64 = 30;
/// How long an emailed link works.
pub const EMAIL_LINK_HOURS: i64 = 24;

/// What an email link is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Verify,
    Reset,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Purpose::Verify => "verify",
            Purpose::Reset => "reset",
        }
    }
}

/// Emails are compared trimmed and lower-cased.
pub fn normalise_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// A plausible email address: one `@`, something on each side, a dot in the domain,
/// and no spaces. The verification email checks the rest.
pub fn looks_like_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.contains(char::is_whitespace)
        && !domain.contains('@')
        && email.len() <= 254
}

fn later(duration: Duration) -> String {
    stamp(now_utc() + duration)
}

/// The result of a signup.
pub enum Signup {
    Created(String),
    /// Another account has the email already.
    Taken,
}

/// Creates an unverified account, or names the account that has the email.
pub async fn sign_up(
    pool: &SqlitePool,
    email: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<Signup, sqlx::Error> {
    let taken = sqlx::query_scalar!("SELECT 1 AS found FROM accounts WHERE email = ?", email)
        .fetch_optional(pool)
        .await?;
    if taken.is_some() {
        return Ok(Signup::Taken);
    }
    let id = ids::new_account_id();
    let created_at = now();
    sqlx::query!(
        "INSERT INTO accounts (id, display_name, created_at, email, password_hash)
         VALUES (?, ?, ?, ?, ?)",
        id,
        display_name,
        created_at,
        email,
        password_hash
    )
    .execute(pool)
    .await?;
    Ok(Signup::Created(id))
}

/// The account with this email, as login needs it.
pub struct Login {
    pub account_id: String,
    pub password_hash: Option<String>,
}

pub async fn login_for_email(pool: &SqlitePool, email: &str) -> Result<Option<Login>, sqlx::Error> {
    sqlx::query_as!(
        Login,
        "SELECT id AS \"account_id!\", password_hash FROM accounts WHERE email = ?",
        email
    )
    .fetch_optional(pool)
    .await
}

/// `text` trimmed, if it is 1 to `max` characters on one line: for display names and
/// token names.
pub fn one_line_name(text: &str, max: usize) -> Option<&str> {
    let text = text.trim();
    let length = text.chars().count();
    (length >= 1 && length <= max && !text.contains(char::is_control)).then_some(text)
}

/// A new email link's secret for `account_id`. Only its hash is stored.
pub async fn new_email_link(
    pool: &SqlitePool,
    account_id: &str,
    purpose: Purpose,
) -> Result<String, sqlx::Error> {
    let secret = ids::new_secret();
    let hash = ids::token_hash(&secret);
    let created_at = now();
    let expires_at = later(Duration::hours(EMAIL_LINK_HOURS));
    let purpose = purpose.as_str();
    sqlx::query!(
        "INSERT INTO email_tokens (hash, account_id, purpose, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?)",
        hash,
        account_id,
        purpose,
        created_at,
        expires_at
    )
    .execute(pool)
    .await?;
    Ok(secret)
}

/// Whether an email link would work, without using it: for the page the link opens.
pub async fn email_link_works(
    pool: &SqlitePool,
    secret: &str,
    purpose: Purpose,
) -> Result<bool, sqlx::Error> {
    let hash = ids::token_hash(secret);
    let now = now();
    let purpose = purpose.as_str();
    let found = sqlx::query_scalar!(
        "SELECT 1 AS found FROM email_tokens
         WHERE hash = ? AND purpose = ? AND used_at IS NULL AND expires_at > ?",
        hash,
        purpose,
        now
    )
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Spends an email link and returns its account, if the link is unused, unexpired and
/// for `purpose`.
async fn spend_link(
    connection: &mut SqliteConnection,
    secret: &str,
    purpose: Purpose,
) -> Result<Option<String>, sqlx::Error> {
    let hash = ids::token_hash(secret);
    let used_at = now();
    let purpose = purpose.as_str();
    sqlx::query_scalar!(
        "UPDATE email_tokens SET used_at = ?
         WHERE hash = ? AND purpose = ? AND used_at IS NULL AND expires_at > ?
         RETURNING account_id",
        used_at,
        hash,
        purpose,
        used_at
    )
    .fetch_optional(&mut *connection)
    .await
}

async fn mark_verified(
    connection: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    let verified_at = now();
    sqlx::query!(
        "UPDATE accounts SET email_verified_at = COALESCE(email_verified_at, ?) WHERE id = ?",
        verified_at,
        account_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// Verifies the account a verification link was sent for. False when the link is
/// used, expired or unknown.
pub async fn verify_with_link(pool: &SqlitePool, secret: &str) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let Some(account_id) = spend_link(&mut transaction, secret, Purpose::Verify).await? else {
        return Ok(false);
    };
    mark_verified(&mut transaction, &account_id).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Sets a new password through a reset link, in one transaction: spends every reset
/// link the account has, ends all its sessions, and marks the address verified, since
/// the link reached it. False when the link is used, expired or unknown.
pub async fn reset_with_link(
    pool: &SqlitePool,
    secret: &str,
    password_hash: &str,
) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let Some(account_id) = spend_link(&mut transaction, secret, Purpose::Reset).await? else {
        return Ok(false);
    };
    let used_at = now();
    sqlx::query!(
        "UPDATE email_tokens SET used_at = ?
         WHERE account_id = ? AND purpose = 'reset' AND used_at IS NULL",
        used_at,
        account_id
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!(
        "UPDATE accounts SET password_hash = ? WHERE id = ?",
        password_hash,
        account_id
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!("DELETE FROM sessions WHERE account_id = ?", account_id)
        .execute(&mut *transaction)
        .await?;
    mark_verified(&mut transaction, &account_id).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Starts a session and returns the cookie's secret.
pub async fn new_session(pool: &SqlitePool, account_id: &str) -> Result<String, sqlx::Error> {
    let secret = ids::new_secret();
    let hash = ids::token_hash(&secret);
    let created_at = now();
    let expires_at = later(Duration::days(SESSION_DAYS));
    sqlx::query!(
        "INSERT INTO sessions (hash, account_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
        hash,
        account_id,
        created_at,
        expires_at
    )
    .execute(pool)
    .await?;
    Ok(secret)
}

/// The account a session cookie's secret belongs to, if the session is current.
pub async fn session_account(
    pool: &SqlitePool,
    secret: &str,
) -> Result<Option<String>, sqlx::Error> {
    let hash = ids::token_hash(secret);
    let now = now();
    sqlx::query_scalar!(
        "SELECT s.account_id FROM sessions s JOIN accounts a ON a.id = s.account_id
         WHERE s.hash = ? AND s.expires_at > ? AND a.deleted_at IS NULL",
        hash,
        now
    )
    .fetch_optional(pool)
    .await
}

/// Whether `account_id` is the admin (design.md, Admin).
pub async fn is_admin(pool: &SqlitePool, account_id: &str) -> Result<bool, sqlx::Error> {
    let admin = sqlx::query_scalar!(
        r#"SELECT is_admin AS "is_admin: bool" FROM accounts
           WHERE id = ? AND deleted_at IS NULL"#,
        account_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(admin == Some(true))
}

/// An account under the review guard.
pub struct Guarded {
    pub id: String,
    pub display_name: Option<String>,
    pub since: String,
}

/// Every account under the review guard, longest guarded first.
pub async fn guarded(pool: &SqlitePool) -> Result<Vec<Guarded>, sqlx::Error> {
    sqlx::query_as!(
        Guarded,
        r#"SELECT a.id, w.display_name, a.review_guard_at AS "since!"
           FROM accounts a JOIN authors w ON w.id = a.id
           WHERE a.review_guard_at IS NOT NULL ORDER BY a.review_guard_at, a.id"#
    )
    .fetch_all(pool)
    .await
}

/// Puts `account_id` under the review guard, if it isn't already: the owner rejected
/// or held one of its entries, or confirmed a refused series (the owner's decision,
/// 2026-11-06).
pub async fn guard(
    connection: &mut sqlx::SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    let at = now();
    sqlx::query!(
        "UPDATE accounts SET review_guard_at = ? WHERE id = ? AND review_guard_at IS NULL",
        at,
        account_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// Whether `account_id` is under the review guard.
pub async fn is_guarded(pool: &SqlitePool, account_id: &str) -> Result<bool, sqlx::Error> {
    let since = sqlx::query_scalar!(
        "SELECT review_guard_at FROM accounts WHERE id = ?",
        account_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(since.flatten().is_some())
}

/// Lifts the review guard from `account_id`. False if it wasn't guarded.
pub async fn clear_guard(pool: &SqlitePool, account_id: &str) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE accounts SET review_guard_at = NULL
         WHERE id = ? AND review_guard_at IS NOT NULL",
        account_id
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

pub async fn end_session(pool: &SqlitePool, secret: &str) -> Result<(), sqlx::Error> {
    let hash = ids::token_hash(secret);
    sqlx::query!("DELETE FROM sessions WHERE hash = ?", hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// What the account page shows.
pub struct AccountView {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub verified: bool,
    pub tokens: Vec<TokenView>,
}

pub struct TokenView {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

pub async fn view(pool: &SqlitePool, account_id: &str) -> Result<AccountView, sqlx::Error> {
    let account = sqlx::query!(
        "SELECT id AS \"id!\", display_name, email, email_verified_at FROM accounts WHERE id = ?",
        account_id
    )
    .fetch_one(pool)
    .await?;
    let tokens = sqlx::query_as!(
        TokenView,
        "SELECT id, name, created_at, revoked_at FROM tokens WHERE account_id = ?
         ORDER BY created_at DESC",
        account_id
    )
    .fetch_all(pool)
    .await?;
    Ok(AccountView {
        id: account.id,
        display_name: account.display_name,
        email: account.email,
        verified: account.email_verified_at.is_some(),
        tokens,
    })
}

/// Revokes one of the account's own tokens. Returns whether it found one.
pub async fn revoke_token(
    pool: &SqlitePool,
    account_id: &str,
    token_id: &str,
) -> Result<bool, sqlx::Error> {
    let revoked_at = now();
    let done = sqlx::query!(
        "UPDATE tokens SET revoked_at = ? WHERE id = ? AND account_id = ? AND revoked_at IS NULL",
        revoked_at,
        token_id,
        account_id
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}
