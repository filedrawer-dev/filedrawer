//! What the owner does from a shell on the server: create an account, give it a
//! token, and make it the admin.

use sqlx::SqlitePool;

use crate::ids;
use crate::time::now;

#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    #[error("a display name is 1 to 50 characters")]
    DisplayName,
    #[error("no account {0}")]
    NoAccount(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
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
