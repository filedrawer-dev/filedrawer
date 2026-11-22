//! What the owner does from a shell on the server: create an account, give it a
//! token, make it the admin, and revoke every account's secrets after a leak.

use sqlx::SqlitePool;

use crate::ids;
use crate::time::now;

#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    #[error("a display name is 1 to 50 characters")]
    DisplayName,
    #[error("no account {0}")]
    NoAccount(String),
    #[error("this database has no accounts: is FILEDRAWER_DATABASE the server's?")]
    NoAccounts,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// What `revoke_all` does beyond ending sessions, revoking tokens and spending email
/// links.
#[derive(Debug, Default, Clone, Copy)]
pub struct Revoke {
    /// Also clear every password.
    pub passwords: bool,
    /// Change the database; otherwise only count.
    pub apply: bool,
}

/// After a leak: ends every session, revokes every token and spends every unused
/// email link, and with `passwords` clears every password, in one transaction, for
/// every account. Without `apply` it changes nothing. Returns what it ended, or
/// would end.
pub async fn revoke_all(pool: &SqlitePool, revoke: Revoke) -> Result<Revoked, AdminError> {
    let at = now();
    let mut tx = crate::db::write_tx(pool).await?;
    let sessions = sqlx::query_scalar!("SELECT COUNT(*) FROM sessions")
        .fetch_one(&mut *tx)
        .await?;
    let tokens = sqlx::query_scalar!("SELECT COUNT(*) FROM tokens WHERE revoked_at IS NULL")
        .fetch_one(&mut *tx)
        .await?;
    let links = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM email_tokens WHERE used_at IS NULL AND expires_at > ?",
        at
    )
    .fetch_one(&mut *tx)
    .await?;
    let passwords =
        sqlx::query_scalar!("SELECT COUNT(*) FROM accounts WHERE password_hash IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    // A mistyped path opens a new, empty database, where revoking nothing would
    // read as done.
    let accounts = sqlx::query_scalar!("SELECT COUNT(*) FROM accounts")
        .fetch_one(&mut *tx)
        .await?;
    if accounts == 0 {
        return Err(AdminError::NoAccounts);
    }
    if revoke.apply {
        sqlx::query!("DELETE FROM sessions")
            .execute(&mut *tx)
            .await?;
        sqlx::query!(
            "UPDATE tokens SET revoked_at = ? WHERE revoked_at IS NULL",
            at
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE email_tokens SET used_at = ? WHERE used_at IS NULL",
            at
        )
        .execute(&mut *tx)
        .await?;
        if revoke.passwords {
            sqlx::query!("UPDATE accounts SET password_hash = NULL")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
    }
    Ok(Revoked {
        applied: revoke.apply,
        sessions,
        tokens,
        links,
        passwords: revoke.passwords.then_some(passwords),
    })
}

/// What `revoke_all` ended, or would have without `apply`.
#[derive(Debug)]
pub struct Revoked {
    pub applied: bool,
    pub sessions: i64,
    pub tokens: i64,
    /// Unused, unexpired email links.
    pub links: i64,
    /// Passwords cleared; none when they weren't asked for.
    pub passwords: Option<i64>,
}

/// Creates an account and returns its ID.
pub async fn create_account(pool: &SqlitePool, display_name: &str) -> Result<String, AdminError> {
    let length = display_name.chars().count();
    if length == 0 || length > 50 {
        return Err(AdminError::DisplayName);
    }
    let id = ids::new_account_id();
    let created_at = now();
    // The owner made it, so it counts as verified; it has no email to verify.
    sqlx::query!(
        "INSERT INTO accounts (id, display_name, created_at, email_verified_at)
         VALUES (?, ?, ?, ?)",
        id,
        display_name,
        created_at,
        created_at
    )
    .execute(pool)
    .await?;
    Ok(id)
}

/// Makes `account_id` the admin: its browser sessions reach the admin pages. Returns
/// the account's ID.
pub async fn grant(pool: &SqlitePool, account_id: &str) -> Result<String, AdminError> {
    let done = sqlx::query!(
        "UPDATE accounts SET is_admin = 1 WHERE id = ? AND deleted_at IS NULL",
        account_id
    )
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(AdminError::NoAccount(account_id.to_string()));
    }
    Ok(account_id.to_string())
}

/// Creates a token for `account_id` and returns it. Only its hash is stored, so this
/// is the one time anyone sees it.
pub async fn create_token(
    pool: &SqlitePool,
    account_id: &str,
    name: &str,
) -> Result<String, AdminError> {
    let exists = sqlx::query_scalar!(
        "SELECT 1 AS found FROM accounts WHERE id = ? AND deleted_at IS NULL",
        account_id
    )
    .fetch_optional(pool)
    .await?;
    if exists.is_none() {
        return Err(AdminError::NoAccount(account_id.to_string()));
    }
    let token = ids::new_token();
    let id = ids::new_id();
    let hash = ids::token_hash(&token);
    let created_at = now();
    sqlx::query!(
        "INSERT INTO tokens (id, account_id, name, hash, created_at) VALUES (?, ?, ?, ?, ?)",
        id,
        account_id,
        name,
        hash,
        created_at
    )
    .execute(pool)
    .await?;
    Ok(token)
}
