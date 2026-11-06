//! Attached code (design.md, Attached code; the owner's decisions, 2026-11-06): an
//! account uploads a set of UTF-8 text files once, each naming its origin, and its
//! entries cite the set as `attachments: as-...`. Files are stored once by SHA-256.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::file_store::Writer;
use crate::{AppState, ids, time::now};

/// The most bytes one set's files may hold in all (design.md: 1 MiB per entry).
pub const MAX_SET_BYTES: usize = 1_048_576;
/// The most bytes an account may upload in a UTC day (design.md: 10 MB).
pub const ACCOUNT_DAILY_BYTES: i64 = 10_000_000;

/// A set as an agent sends it.
#[derive(Debug, Deserialize)]
pub struct SetRequest {
    /// Short repo names the origins may use, each with its URL or `no remote`.
    #[serde(default)]
    pub repos: BTreeMap<String, String>,
    pub files: Vec<FileRequest>,
}

#[derive(Debug, Deserialize)]
pub struct FileRequest {
    pub origin: String,
    pub text: String,
}

/// A file read from a request, ready to store.
#[derive(Debug, Clone)]
pub struct NewFile {
    pub path: String,
    pub origin: String,
    /// The URL, or `no remote`, of a repo the set named.
    pub repo_url: Option<String>,
    pub text: String,
    pub sha256: String,
}

/// Why a set is refused before anything is stored.
#[derive(Debug)]
pub enum Invalid {
    /// Over [`MAX_SET_BYTES`]: the bytes it holds.
    TooLarge(usize),
    /// Every problem found.
    Problems(Vec<String>),
}

/// Reads `set` into files to store, or every reason it can't be.
pub fn read(set: &SetRequest) -> Result<Vec<NewFile>, Invalid> {
    let bytes: usize = set.files.iter().map(|file| file.text.len()).sum();
    if bytes > MAX_SET_BYTES {
        return Err(Invalid::TooLarge(bytes));
    }
    let mut problems = Vec::new();
    for (name, url) in &set.repos {
        if !(is_http_url(url) || url == "no remote") {
            problems.push(format!(
                "repo `{name}` must be an http(s) URL or `no remote`, not `{url}`"
            ));
        }
    }
    if set.files.is_empty() {
        problems.push("a set needs at least one file in `files`".into());
    }
    let mut paths = HashSet::new();
    let mut files = Vec::new();
    for file in &set.files {
        let Some((repo, path)) = split_origin(&file.origin) else {
            problems.push(format!(
                "origin `{}` must be `<repo>@<commit>:<path>`, with a 7- to 40-character commit hash, or `author:<path>`",
                file.origin
            ));
            continue;
        };
        let repo_url = match repo {
            Repo::Author | Repo::Url => None,
            Repo::Named(name) => match set.repos.get(name) {
                Some(url) => Some(url.clone()),
                None => {
                    problems.push(format!(
                        "origin `{}` names repo `{name}`, which `repos` doesn't declare",
                        file.origin
                    ));
                    continue;
                }
            },
        };
        if !paths.insert(path.to_string()) {
            problems.push(format!("two files have the path {path}"));
        }
        if file
            .text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        {
            problems.push(format!(
                "{path} holds control characters; attachments are text files"
            ));
        }
        files.push(NewFile {
            path: path.to_string(),
            origin: file.origin.clone(),
            repo_url,
            text: file.text.clone(),
            sha256: sha256_hex(&file.text),
        });
    }
    if problems.is_empty() {
        Ok(files)
    } else {
        Err(Invalid::Problems(problems))
    }
}

enum Repo<'a> {
    Author,
    Url,
    Named(&'a str),
}

/// The repo and path of an origin: `author:<path>`, or `<repo>@<commit>:<path>`
/// where `<repo>` is a URL or a declared name.
fn split_origin(origin: &str) -> Option<(Repo<'_>, &str)> {
    if let Some(path) = origin.strip_prefix("author:") {
        return (!path.is_empty()).then_some((Repo::Author, path));
    }
    // The `@` that a commit hash and a `:` follow; a URL may hold other `@`s and `:`s.
    origin.match_indices('@').find_map(|(at, _)| {
        let rest = &origin[at + 1..];
        let colon = rest.find(':')?;
        let commit = &rest[..colon];
        let path = &rest[colon + 1..];
        let repo = &origin[..at];
        let hash =
            (7..=40).contains(&commit.len()) && commit.chars().all(|c| c.is_ascii_hexdigit());
        if !hash || path.is_empty() || repo.is_empty() {
            return None;
        }
        let repo = if is_http_url(repo) {
            Repo::Url
        } else if repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            Repo::Named(repo)
        } else {
            return None;
        };
        Some((repo, path))
    })
}

fn is_http_url(text: &str) -> bool {
    (text.starts_with("https://") || text.starts_with("http://"))
        && !text.contains(char::is_whitespace)
}

/// The SHA-256 of `text`, in lowercase hex.
pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What storing a set came to.
#[derive(Debug)]
pub enum Stored {
    Saved {
        id: String,
        bytes: i64,
    },
    /// The account's day is spent.
    DailyCap,
    /// The site's storage cap is reached.
    StoreFull,
    /// Storing the files would leave the disk under its floor.
    DiskLow,
}

/// Why storing or deleting attached files failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("attached files' bytes: {0}")]
    Files(#[from] std::io::Error),
}

/// Stores `files` as a new set of `account_id`'s, within the account's day, the
/// site's storage cap (bytes of unique files) and the disk's floor. New files' bytes
/// go into the file store before the set's records commit.
pub async fn store(
    state: &AppState,
    account_id: &str,
    files: &[NewFile],
) -> Result<Stored, StoreError> {
    let pool = &state.pool;
    // A set is at most 1 MiB by `read`, so these sizes fit an i64.
    let size = |file: &NewFile| i64::try_from(file.text.len()).unwrap_or(i64::MAX);
    let bytes: i64 = files.iter().map(size).sum();
    let today = format!("{}%", &now()[..10]);
    let writer = state.files.write().await;
    let mut transaction = pool.begin().await?;
    let spent = sqlx::query_scalar!(
        r#"SELECT COALESCE(SUM(bytes), 0) AS "spent!: i64" FROM attachment_sets
           WHERE account_id = ? AND created_at LIKE ?"#,
        account_id,
        today
    )
    .fetch_one(&mut *transaction)
    .await?;
    if spent + bytes > ACCOUNT_DAILY_BYTES {
        return Ok(Stored::DailyCap);
    }
    let mut new_bytes = 0i64;
    let mut seen = HashSet::new();
    for file in files {
        let stored =
            sqlx::query_scalar!("SELECT 1 AS found FROM files WHERE sha256 = ?", file.sha256)
                .fetch_optional(&mut *transaction)
                .await?;
        if stored.is_none() && seen.insert(file.sha256.clone()) {
            new_bytes += size(file);
        }
    }
    let held = sqlx::query_scalar!(r#"SELECT COALESCE(SUM(bytes), 0) AS "held!: i64" FROM files"#)
        .fetch_one(&mut *transaction)
        .await?;
    if new_bytes > 0 && held + new_bytes > state.attachment_store_cap_bytes {
        return Ok(Stored::StoreFull);
    }
    // Non-negative, as a sum of sizes.
    let new_bytes = u64::try_from(new_bytes).unwrap_or(u64::MAX);
    if new_bytes > 0
        && state.files.free_bytes()?.saturating_sub(new_bytes) < state.attachment_disk_floor_bytes
    {
        return Ok(Stored::DiskLow);
    }
    let at = now();
    let id = format!("as-{}", ids::new_id());
    let set_seq = sqlx::query_scalar!(
        r#"INSERT INTO attachment_sets (id, account_id, bytes, created_at) VALUES (?, ?, ?, ?)
           RETURNING seq AS "seq!: i64""#,
        id,
        account_id,
        bytes,
        at
    )
    .fetch_one(&mut *transaction)
    .await?;
    for file in files {
        writer.put(&file.sha256, &file.text).await?;
        let file_bytes = size(file);
        sqlx::query!(
            "INSERT OR IGNORE INTO files (sha256, bytes, created_at) VALUES (?, ?, ?)",
            file.sha256,
            file_bytes,
            at
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query!(
            "INSERT INTO attachment_files (set_seq, path, origin, repo_url, sha256)
             VALUES (?, ?, ?, ?, ?)",
            set_seq,
            file.path,
            file.origin,
            file.repo_url,
            file.sha256
        )
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Stored::Saved { id, bytes })
}

/// The row of set `id`, if `account_id` uploaded it.
pub async fn owned_set(
    pool: &SqlitePool,
    account_id: &str,
    id: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM attachment_sets WHERE id = ? AND account_id = ?"#,
        id,
        account_id
    )
    .fetch_optional(pool)
    .await
}

/// A file in an entry's set, as its page lists it.
pub struct SetFile {
    pub path: String,
    pub origin: String,
    pub repo_url: Option<String>,
    pub sha256: String,
    pub bytes: i64,
}

/// The set an entry cites, as its page lists it.
pub struct EntrySet {
    pub id: String,
    /// By path.
    pub files: Vec<SetFile>,
}

/// The set entry `seq` cites, if any.
pub async fn entry_set(pool: &SqlitePool, seq: i64) -> Result<Option<EntrySet>, sqlx::Error> {
    let Some(set) = sqlx::query!(
        r#"SELECT s.seq AS "seq!: i64", s.id FROM entries e
           JOIN attachment_sets s ON s.seq = e.attachment_set_seq WHERE e.seq = ?"#,
        seq
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let files = sqlx::query_as!(
        SetFile,
        r#"SELECT af.path, af.origin, af.repo_url, af.sha256, f.bytes
           FROM attachment_files af JOIN files f ON f.sha256 = af.sha256
           WHERE af.set_seq = ? ORDER BY af.path"#,
        set.seq
    )
    .fetch_all(pool)
    .await?;
    Ok(Some(EntrySet { id: set.id, files }))
}

/// Whether a public entry's set holds file `sha256`, so the files host may serve it.
pub async fn is_public_file(pool: &SqlitePool, sha256: &str) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar!(
        "SELECT 1 AS found FROM attachment_files af
         JOIN entries e ON e.attachment_set_seq = af.set_seq
         WHERE af.sha256 = ? AND e.status = 'public' LIMIT 1",
        sha256
    )
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Deletes the record of every file no set holds, and returns their hashes, for
/// [`remove_bytes`] once the transaction commits.
pub async fn delete_lone_files(
    transaction: &mut sqlx::SqliteConnection,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "DELETE FROM files WHERE NOT EXISTS (
             SELECT 1 FROM attachment_files af WHERE af.sha256 = files.sha256)
         RETURNING sha256"
    )
    .fetch_all(transaction)
    .await
}

/// Removes the bytes of files whose records are deleted and committed. A file that
/// can't be removed is logged, and stays on disk with no record.
pub async fn remove_bytes(writer: &Writer<'_>, hashes: &[String]) {
    for sha256 in hashes {
        if let Err(error) = writer.remove(sha256).await {
            tracing::error!(%sha256, %error, "removing an attached file's bytes failed");
        }
    }
}

/// The largest chunk of a file one screening call reads: well under Haiku's
/// long-prompt price step (design.md, Attached code).
pub const CHUNK_BYTES: usize = 256 * 1024;

/// A file of an entry's set, with the screen's verdict on it so far. Its bytes are
/// in the [`FileStore`].
pub struct EntryFile {
    pub sha256: String,
    pub path: String,
    pub verdict: Option<String>,
    pub reason: Option<String>,
}

/// The files of the set entry `seq` cites, by path; none when it cites no set.
pub async fn entry_files(pool: &SqlitePool, seq: i64) -> Result<Vec<EntryFile>, sqlx::Error> {
    sqlx::query_as!(
        EntryFile,
        r#"SELECT af.sha256, af.path, f.screen_verdict AS verdict, f.screen_reason AS reason
           FROM entries e
           JOIN attachment_files af ON af.set_seq = e.attachment_set_seq
           JOIN files f ON f.sha256 = af.sha256
           WHERE e.seq = ? ORDER BY af.path"#,
        seq
    )
    .fetch_all(pool)
    .await
}

/// Stores the screen's verdict on file `sha256`, kept for every set that holds it:
/// `pass`, `reject` or `review`, with a reason for the last two.
pub async fn store_file_verdict(
    pool: &SqlitePool,
    sha256: &str,
    verdict: &str,
    reason: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE files SET screen_verdict = ?, screen_reason = ?
         WHERE sha256 = ? AND screen_verdict IS NULL",
        verdict,
        reason,
        sha256
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Passes every file of entry `seq`'s set that has no verdict: the owner read them
/// on the review page and published the entry, so later entries citing them aren't
/// sent to review again for the same files.
pub async fn pass_unjudged_files(pool: &SqlitePool, seq: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE files SET screen_verdict = 'pass'
         WHERE screen_verdict IS NULL AND sha256 IN (
             SELECT af.sha256 FROM entries e
             JOIN attachment_files af ON af.set_seq = e.attachment_set_seq
             WHERE e.seq = ?)",
        seq
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// `text` cut into chunks of at most [`CHUNK_BYTES`], at line ends where it can be
/// and always at character boundaries.
pub fn chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut rest = text;
    while rest.len() > CHUNK_BYTES {
        let mut cut = CHUNK_BYTES;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(line_end) = rest[..cut].rfind('\n') {
            cut = line_end + 1;
        }
        chunks.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    if !rest.is_empty() || chunks.is_empty() {
        chunks.push(rest);
    }
    chunks
}

/// Records that the owner is told of `kind` today. False if they already were.
pub async fn first_notice_today(pool: &SqlitePool, kind: &str) -> Result<bool, sqlx::Error> {
    let day = now()[..10].to_string();
    let done = sqlx::query!(
        "INSERT OR IGNORE INTO owner_notices (kind, day) VALUES (?, ?)",
        kind,
        day
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}
