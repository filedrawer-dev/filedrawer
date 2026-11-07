//! The owner removing an attached file (design.md, Attached code; the owner's
//! decisions, 2026-11-06): a removed file is served for no entry, and its bytes and
//! record stay for analysis. One removed as a hazard is also banned: refused at
//! upload and when cited, rejected in screening, and its uploaders guarded.

use sqlx::{SqliteConnection, SqlitePool};

use crate::{accounts, time::now};

/// What every refusal and rejection that names a banned file says of it.
pub const REMOVED_BY_OWNER: &str = "removed by the archive's owner";

/// Whether file `sha256` is banned, in the caller's transaction, as an upload checks
/// before it stores the file.
pub async fn is_banned(conn: &mut SqliteConnection, sha256: &str) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar!(
        "SELECT 1 AS found FROM banned_files WHERE sha256 = ?",
        sha256
    )
    .fetch_optional(conn)
    .await?;
    Ok(found.is_some())
}

/// The paths in set `set_seq` whose files are banned, by path.
pub async fn banned_paths(pool: &SqlitePool, set_seq: i64) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT af.path FROM attachment_files af
         JOIN banned_files b ON b.sha256 = af.sha256
         WHERE af.set_seq = ? ORDER BY af.path",
        set_seq
    )
    .fetch_all(pool)
    .await
}

/// Where a stored file stands with the owner. A ban implies a removal.
pub enum FileState {
    Kept,
    /// Removed at the time held.
    Removed(String),
    /// Removed as a hazard at the time held, and banned.
    Banned(String),
}

impl FileState {
    pub fn is_banned(&self) -> bool {
        matches!(self, FileState::Banned(_))
    }
}

/// A stored file, as the owner's page for it shows it.
pub struct OwnersFile {
    pub sha256: String,
    pub bytes: i64,
    /// The screen's verdict, kept as it was through a removal.
    pub verdict: Option<String>,
    pub reason: Option<String>,
    pub state: FileState,
}

/// File `sha256`'s record, if the server holds one.
pub async fn owners_file(
    pool: &SqlitePool,
    sha256: &str,
) -> Result<Option<OwnersFile>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT f.sha256 AS "sha256!", f.bytes, f.screen_verdict AS verdict,
                  f.screen_reason AS reason, f.removed_at, b.banned_at AS "banned_at?"
           FROM files f LEFT JOIN banned_files b ON b.sha256 = f.sha256
           WHERE f.sha256 = ?"#,
        sha256
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| OwnersFile {
        state: match (row.banned_at, row.removed_at) {
            (Some(at), _) => FileState::Banned(at),
            (None, Some(at)) => FileState::Removed(at),
            (None, None) => FileState::Kept,
        },
        sha256: row.sha256,
        bytes: row.bytes,
        verdict: row.verdict,
        reason: row.reason,
    }))
}

/// One place a file is used: a set that holds it, under a path, and an entry citing
/// that set, if any.
pub struct FileUse {
    pub set_id: String,
    pub account_id: String,
    pub path: String,
    pub entry_id: Option<String>,
    pub entry_status: Option<String>,
}

/// Every set holding file `sha256`, with each entry citing it, oldest set first.
pub async fn file_uses(pool: &SqlitePool, sha256: &str) -> Result<Vec<FileUse>, sqlx::Error> {
    sqlx::query_as!(
        FileUse,
        r#"SELECT s.id AS set_id, s.account_id, af.path,
                  e.id AS "entry_id?", e.status AS "entry_status?"
           FROM attachment_files af
           JOIN attachment_sets s ON s.seq = af.set_seq
           LEFT JOIN entries e ON e.attachment_set_seq = s.seq
           WHERE af.sha256 = ? ORDER BY s.seq, e.seq"#,
        sha256
    )
    .fetch_all(pool)
    .await
}

/// How the owner removes a file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// It is served for no entry; its bytes and record stay.
    Plain,
    /// As `Plain`, and its hash is banned, and every account that uploaded a set
    /// holding it goes under the review guard.
    Hazard,
}

/// Removes file `sha256` as `removal` says. False when the server holds no such file.
pub async fn remove(
    pool: &SqlitePool,
    sha256: &str,
    removal: Removal,
) -> Result<bool, sqlx::Error> {
    let at = now();
    let mut transaction = pool.begin().await?;
    let done = sqlx::query!(
        "UPDATE files SET removed_at = COALESCE(removed_at, ?) WHERE sha256 = ?",
        at,
        sha256
    )
    .execute(&mut *transaction)
    .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    if removal == Removal::Hazard {
        sqlx::query!(
            "INSERT OR IGNORE INTO banned_files (sha256, banned_at) VALUES (?, ?)",
            sha256,
            at
        )
        .execute(&mut *transaction)
        .await?;
        let uploaders = sqlx::query_scalar!(
            "SELECT DISTINCT s.account_id FROM attachment_files af
             JOIN attachment_sets s ON s.seq = af.set_seq WHERE af.sha256 = ?",
            sha256
        )
        .fetch_all(&mut *transaction)
        .await?;
        for account_id in uploaders {
            accounts::guard(&mut transaction, &account_id).await?;
        }
    }
    transaction.commit().await?;
    Ok(true)
}
