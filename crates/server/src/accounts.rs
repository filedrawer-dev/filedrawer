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

/// Emails are compared trimmed and lower-cased. Only ASCII letters change case, so
/// no other character becomes one an address may hold.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// The characters of an address's local part, besides letters, digits and dots.
const LOCAL_PUNCTUATION: &str = "!#$%&'*+-/=?^_`{|}~";

/// A bare email address, `local@domain` in ASCII and nothing around it: no display
/// name, angle brackets, quotes, comments or address literal. A mail library reads
/// those as other spellings of one mailbox, and the address stored, the one the email
/// limits count and the one the mailer sends to must be the same string. The
/// verification email checks that it exists.
pub fn looks_like_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    let local_part = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || LOCAL_PUNCTUATION.contains(c))
    };
    let label = |label: &str| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    email.len() <= 254
        && local.len() <= 64
        && local.split('.').all(local_part)
        && domain.contains('.')
        && domain.split('.').all(label)
}

fn later(duration: Duration) -> String {
    stamp(now_utc() + duration)
}

/// The result of a signup.
pub enum Signup {
    Created(String),
    /// This account has the email already.
    Taken(String),
    /// The invite code given can't take a signup: unknown, expired, revoked or used up.
    BadInvite,
}

/// Creates an unverified account, or names the account that has the email.
pub async fn sign_up(
    pool: &SqlitePool,
    email: &str,
    password_hash: &str,
    display_name: &str,
    invite: Option<&str>,
) -> Result<Signup, sqlx::Error> {
    // IMMEDIATE, so two signups can't both take a code's last use.
    let mut tx = crate::db::write_tx(pool).await?;
    // The code first, so a bad one answers the same whether or not the address has
    // an account.
    let invite_seq = match invite {
        None => None,
        Some(code) => match crate::invites::usable(&mut *tx, code, now_utc()).await? {
            Some(seq) => Some(seq),
            None => return Ok(Signup::BadInvite),
        },
    };
    let taken = sqlx::query_scalar!("SELECT id FROM accounts WHERE email = ?", email)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(id) = taken {
        return Ok(Signup::Taken(id));
    }
    let id = ids::new_account_id();
    let created_at = now();
    sqlx::query!(
        "INSERT INTO accounts (id, display_name, created_at, email, password_hash, invite_seq)
         VALUES (?, ?, ?, ?, ?, ?)",
        id,
        display_name,
        created_at,
        email,
        password_hash,
        invite_seq
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
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

/// The longest display name, in characters (design.md, Accounts).
pub const DISPLAY_NAME_CHARS: usize = 50;

/// `text` trimmed, if it can be a display name: 1 to [`DISPLAY_NAME_CHARS`]
/// characters on one line that show something, with no invisible character. The name is printed beside the
/// server's own words, on the `author:` line and the card's last line, so it holds
/// nothing a reader can't see.
pub fn display_name(text: &str) -> Option<&str> {
    let text = text.trim();
    analyzer::is_label(text, DISPLAY_NAME_CHARS).then_some(text)
}

/// The prefixes of linked handles on a card's byline, which a name may not hold, so
/// it can't pose as a handle the server checked (design.md, Accounts).
const HANDLE_PREFIXES: [&str; 2] = ["github:", "orcid:"];

/// `text` trimmed, if it can be a display name ([`display_name`], and holding no
/// handle's prefix), or why not, for the signup forms to say.
pub fn display_name_refusal(text: &str) -> Result<&str, String> {
    let Some(name) = display_name(text) else {
        return Err(format!(
            "A display name is 1 to {DISPLAY_NAME_CHARS} characters on one line, with no invisible characters."
        ));
    };
    let lower = name.to_lowercase();
    match HANDLE_PREFIXES
        .iter()
        .find(|prefix| lower.contains(*prefix))
    {
        Some(prefix) => Err(format!(
            "A display name can't hold \"{prefix}\": cards show an account's linked GitHub or ORCID handle that way, so a name holding it would look like one."
        )),
        None => Ok(name),
    }
}

/// A new email link's secret for `account_id`, and its hash, which is all that is
/// stored.
pub async fn new_email_link(
    conn: &mut SqliteConnection,
    account_id: &str,
    purpose: Purpose,
) -> Result<(String, Vec<u8>), sqlx::Error> {
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
    .execute(conn)
    .await?;
    Ok((secret, hash))
}

/// Retires the email link with `hash`, as if used: its email never went out.
pub async fn retire_email_link(pool: &SqlitePool, hash: &[u8]) -> Result<(), sqlx::Error> {
    let now = now();
    sqlx::query!(
        "UPDATE email_tokens SET used_at = ? WHERE hash = ? AND used_at IS NULL",
        now,
        hash
    )
    .execute(pool)
    .await?;
    Ok(())
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

/// Marks the account's address verified, keeping the first time it was.
async fn mark_verified(
    connection: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    let verified_at = now();
    sqlx::query!(
        "UPDATE accounts SET email_verified_at = ? WHERE id = ? AND email_verified_at IS NULL",
        verified_at,
        account_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn end_sessions(
    connection: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM sessions WHERE account_id = ?", account_id)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

/// Verifies the account a verification link was sent for. Its sessions stay: whoever
/// signed up knows the password and could log in again anyway, and a reset is what
/// ends their way in. False when the link is used, expired or unknown.
pub async fn verify_with_link(pool: &SqlitePool, secret: &str) -> Result<bool, sqlx::Error> {
    let mut transaction = crate::db::write_tx(pool).await?;
    let Some(account_id) = spend_link(&mut transaction, secret, Purpose::Verify).await? else {
        return Ok(false);
    };
    mark_verified(&mut transaction, &account_id).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Sets a new password through a reset link, in one transaction: spends every reset
/// link the account has, ends all its sessions, revokes all its tokens, marks the
/// address verified, since the link reached it, and unlinks GitHub. Whoever held the
/// account before keeps no way to act for it, nor their GitHub handle on its cards.
/// False when the link is used, expired or unknown.
pub async fn reset_with_link(
    pool: &SqlitePool,
    secret: &str,
    password_hash: &str,
) -> Result<bool, sqlx::Error> {
    let mut transaction = crate::db::write_tx(pool).await?;
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
    end_sessions(&mut transaction, &account_id).await?;
    sqlx::query!(
        "UPDATE tokens SET revoked_at = ? WHERE account_id = ? AND revoked_at IS NULL",
        used_at,
        account_id
    )
    .execute(&mut *transaction)
    .await?;
    mark_verified(&mut transaction, &account_id).await?;
    // A change of address someone asked for with the old password ends with it.
    crate::email_change::cancel_in(&mut transaction, &account_id).await?;
    // GitHub can list as verified an address its user no longer holds, so whoever
    // reset through it may not be who linked GitHub (the owner's decision,
    // 2026-11-22).
    crate::github_accounts::unlink_in(&mut transaction, &account_id).await?;
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

/// The admin accounts with an address, for notices to the owner.
pub async fn admin_ids(conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM accounts
         WHERE is_admin = 1 AND email IS NOT NULL AND deleted_at IS NULL"
    )
    .fetch_all(conn)
    .await
}

/// Whether the account has a password, which an account that signed up with GitHub
/// may not.
pub async fn has_password(pool: &SqlitePool, account_id: &str) -> Result<bool, sqlx::Error> {
    let hash = sqlx::query_scalar!(
        "SELECT password_hash FROM accounts WHERE id = ?",
        account_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(hash.flatten().is_some())
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

/// Puts `account_id` under the review guard, if it isn't already. Only the owner's
/// calls trip it (design.md, Submission and screening).
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
