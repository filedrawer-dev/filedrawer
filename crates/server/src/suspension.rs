//! The owner suspending an account (design.md, Admin; #43): it can't submit, and all
//! its entries and the series it made become removed (#57). Lifting the suspension
//! lets it write again and restores nothing; the owner restores each entry and series
//! from its admin page (the owner's decisions, 2026-11-09).

use sqlx::{SqliteConnection, SqlitePool};

use crate::error::AppError;
use crate::respond::CONTACT;
use crate::{db, removed_entries, removed_series, time::now};

/// An account as its admin page shows it.
pub struct OwnersAccount {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    /// When the owner suspended it, while it is suspended.
    pub suspended_at: Option<String>,
}

/// One of its entries, newest first on that page.
pub struct OwnersEntry {
    pub id: String,
    pub status: String,
    pub submitted_at: String,
}

/// Account `account_id`, unless it is deleted, and every entry of it, newest first.
pub async fn owners_account(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Option<(OwnersAccount, Vec<OwnersEntry>)>, sqlx::Error> {
    let Some(account) = sqlx::query_as!(
        OwnersAccount,
        r#"SELECT id AS "id!", display_name, email, suspended_at FROM accounts
           WHERE id = ? AND deleted_at IS NULL"#,
        account_id
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let entries = sqlx::query_as!(
        OwnersEntry,
        "SELECT id, status, submitted_at FROM entries WHERE account_id = ?
         ORDER BY submitted_at DESC, seq DESC",
        account_id
    )
    .fetch_all(pool)
    .await?;
    Ok(Some((account, entries)))
}

/// What suspending or lifting did.
#[derive(Debug, PartialEq, Eq)]
pub enum Changed {
    Done,
    NoAccount,
    /// The account was already in the state asked for.
    Already,
}

/// What a suspended account's writes are refused with.
pub fn refusal() -> AppError {
    AppError::Forbidden(format!(
        "this account is suspended, so it can't submit or change anything; write to {CONTACT}"
    ))
}

/// Refuses a write by `account_id` while it is suspended, for a route that a browser
/// session reaches, which has no token check. (A token's writes are refused in
/// `auth.rs`, and an entry insert by the `entries_not_from_suspended` trigger.)
pub async fn refuse_if_suspended(pool: &SqlitePool, account_id: &str) -> Result<(), AppError> {
    match suspended(&mut *pool.acquire().await?, account_id).await? {
        Some(true) => Err(refusal()),
        _ => Ok(()),
    }
}

/// Whether account `account_id` is suspended; `None` when there is no such account,
/// or it is deleted.
async fn suspended(
    connection: &mut SqliteConnection,
    account_id: &str,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT suspended_at IS NOT NULL AS "suspended!: bool" FROM accounts
           WHERE id = ? AND deleted_at IS NULL"#,
        account_id
    )
    .fetch_optional(connection)
    .await
}

/// Suspends `account_id` and removes every entry of it and every series it made. The
/// caller places the entries that waited on those series.
pub async fn suspend(pool: &SqlitePool, account_id: &str) -> Result<Changed, sqlx::Error> {
    let mut transaction = db::write_tx(pool).await?;
    let at = now();
    let changed = set(&mut transaction, account_id, Some(&at)).await?;
    if changed == Changed::Done {
        removed_entries::remove_all_of(&mut transaction, account_id).await?;
        removed_series::remove_all_of(&mut transaction, account_id).await?;
    }
    transaction.commit().await?;
    Ok(changed)
}

/// Lifts `account_id`'s suspension. Its entries and series stay removed.
pub async fn lift(pool: &SqlitePool, account_id: &str) -> Result<Changed, sqlx::Error> {
    let mut transaction = db::write_tx(pool).await?;
    let changed = set(&mut transaction, account_id, None).await?;
    transaction.commit().await?;
    Ok(changed)
}

/// Sets `suspended_at` to `at`, if it isn't set that way already.
async fn set(
    connection: &mut SqliteConnection,
    account_id: &str,
    at: Option<&str>,
) -> Result<Changed, sqlx::Error> {
    let Some(suspended) = suspended(&mut *connection, account_id).await? else {
        return Ok(Changed::NoAccount);
    };
    if suspended == at.is_some() {
        return Ok(Changed::Already);
    }
    sqlx::query!(
        "UPDATE accounts SET suspended_at = ? WHERE id = ?",
        at,
        account_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(Changed::Done)
}
