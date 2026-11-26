//! Attached code (design.md, Attached code; the owner's decisions, 2026-11-06): an
//! account uploads a set of UTF-8 text files once, each naming its origin, and its
//! entries cite the set as `attachments: as-...`. Files are stored once by SHA-256.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};

use crate::file_store::Writer;
use crate::{AppState, ids, removed_files, time::now};

/// The most bytes one set may store in all (design.md: 1 MiB per entry): its files'
/// text, their origins and paths, and the repos they name.
pub const MAX_SET_BYTES: usize = 1_048_576;
/// The most files one set may hold, and the most repos it may declare.
pub const MAX_SET_FILES: usize = 1000;
/// The most bytes one file's origin may hold.
pub const MAX_ORIGIN_BYTES: usize = 1024;
/// The most bytes one repo's URL may hold.
pub const MAX_REPO_URL_BYTES: usize = 1024;
/// The most bytes one set's origins and the repos they name, names and URLs, may
/// hold in all: the author's text an entry's page shows for its set.
pub const MAX_SET_ORIGIN_BYTES: usize = 262_144;
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
    pub text: String,
    pub sha256: String,
}

/// A set read from a request, ready to store.
#[derive(Debug, Clone)]
pub struct NewSet {
    pub files: Vec<NewFile>,
    /// The repos its files' origins name, each with its URL or `no remote`.
    pub repos: BTreeMap<String, String>,
}

impl NewSet {
    /// The bytes of its origins and of its repos' names and URLs.
    fn origin_bytes(&self) -> usize {
        let origins: usize = self.files.iter().map(|file| file.origin.len()).sum();
        let repos: usize = self
            .repos
            .iter()
            .map(|(name, url)| name.len() + url.len())
            .sum();
        origins + repos
    }

    /// The bytes it stores besides its files' text: each file's origin and path, and
    /// each repo's name and URL.
    pub fn metadata_bytes(&self) -> usize {
        let paths: usize = self.files.iter().map(|file| file.path.len()).sum();
        self.origin_bytes() + paths
    }

    /// The bytes it stores in all, which count toward [`MAX_SET_BYTES`] and its
    /// account's day.
    pub fn bytes(&self) -> usize {
        let text: usize = self.files.iter().map(|file| file.text.len()).sum();
        self.metadata_bytes() + text
    }
}

/// Why a set is refused before anything is stored.
#[derive(Debug)]
pub enum Invalid {
    /// Over a cap on the whole set: which, with what the set holds.
    OverCap(String),
    /// Every problem found.
    Problems(Vec<String>),
}

/// The most characters of the sender's own text a reason repeats.
const SHOWN_CHARS: usize = 80;

/// The start of the sender's `text`, for a reason.
pub fn shown(text: &str) -> String {
    let mut start: String = text.chars().take(SHOWN_CHARS).collect();
    if start.len() < text.len() {
        start.push('…');
    }
    start
}

fn over_set_bytes(bytes: usize) -> Invalid {
    Invalid::OverCap(format!(
        "the set holds {bytes} bytes, counting its files' text, origins, paths and repo URLs; the cap is {MAX_SET_BYTES} bytes"
    ))
}

/// Reads `set` into files to store, or every reason it can't be.
pub fn read(set: &SetRequest) -> Result<NewSet, Invalid> {
    if set.files.len() > MAX_SET_FILES {
        return Err(Invalid::OverCap(format!(
            "the set holds {} files; the cap is {MAX_SET_FILES} files",
            set.files.len()
        )));
    }
    if set.repos.len() > MAX_SET_FILES {
        return Err(Invalid::OverCap(format!(
            "the set declares {} repos; the cap is {MAX_SET_FILES} repos",
            set.repos.len()
        )));
    }
    let text_bytes: usize = set.files.iter().map(|file| file.text.len()).sum();
    if text_bytes > MAX_SET_BYTES {
        return Err(over_set_bytes(text_bytes));
    }
    let mut problems = Vec::new();
    for (name, url) in &set.repos {
        if url.len() > MAX_REPO_URL_BYTES {
            problems.push(format!(
                "repo `{}` has a URL of {} bytes; the cap is {MAX_REPO_URL_BYTES} bytes",
                shown(name),
                url.len()
            ));
        } else if !(is_http_url(url) || url == "no remote") {
            problems.push(format!(
                "repo `{}` must be an http(s) URL or `no remote`, not `{}`",
                shown(name),
                shown(url)
            ));
        }
    }
    if set.files.is_empty() {
        problems.push("a set needs at least one file in `files`".into());
    }
    let mut paths = HashSet::new();
    let mut files = Vec::new();
    let mut repos = BTreeMap::new();
    for file in &set.files {
        if file.origin.len() > MAX_ORIGIN_BYTES {
            problems.push(format!(
                "an origin of {} bytes, starting `{}`, is over the cap of {MAX_ORIGIN_BYTES} bytes",
                file.origin.len(),
                shown(&file.origin)
            ));
            continue;
        }
        let Some((repo, path)) = split_origin(&file.origin) else {
            problems.push(format!(
                "origin `{}` must be `<repo>@<commit>:<path>`, with a 7- to 40-character commit hash, or `author:<path>`",
                shown(&file.origin)
            ));
            continue;
        };
        if let Repo::Named(name) = repo {
            let Some(url) = set.repos.get(name) else {
                problems.push(format!(
                    "origin `{}` names repo `{}`, which `repos` doesn't declare",
                    shown(&file.origin),
                    shown(name)
                ));
                continue;
            };
            // Once for the set, however many files name it.
            if !repos.contains_key(name) {
                repos.insert(name.to_string(), url.clone());
            }
        }
        if !paths.insert(path) {
            problems.push(format!("two files have the path {}", shown(path)));
        }
        if file
            .text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        {
            problems.push(format!(
                "{} holds control characters; attachments are text files",
                shown(path)
            ));
        }
        files.push(NewFile {
            path: path.to_string(),
            origin: file.origin.clone(),
            text: file.text.clone(),
            sha256: sha256_hex(&file.text),
        });
    }
    if !problems.is_empty() {
        return Err(Invalid::Problems(problems));
    }
    let set = NewSet { files, repos };
    let origin_bytes = set.origin_bytes();
    if origin_bytes > MAX_SET_ORIGIN_BYTES {
        return Err(Invalid::OverCap(format!(
            "the set's origins and repo URLs hold {origin_bytes} bytes; the cap is {MAX_SET_ORIGIN_BYTES} bytes"
        )));
    }
    let bytes = set.bytes();
    if bytes > MAX_SET_BYTES {
        return Err(over_set_bytes(bytes));
    }
    Ok(set)
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
    /// The site can store no more.
    Full(Limit),
    /// These paths hold files the owner removed as a hazard, which no set may hold
    /// again, under any path or account (design.md, Attached code).
    Banned(Vec<String>),
}

/// What stops the site storing more attachments.
#[derive(Debug, Clone, Copy)]
pub enum Limit {
    /// The site's storage cap is reached.
    StoreCap,
    /// Storing the files would leave the disk under its floor.
    DiskFloor,
}

/// Why storing or deleting attached files failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("attached files' bytes: {0}")]
    Files(#[from] std::io::Error),
}

/// A size as the database holds it. A set is at most 1 MiB by `read`, so its sizes
/// fit.
fn stored_size(bytes: usize) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

/// Stores `set` as a new set of `account_id`'s, within the account's day, the
/// site's storage cap (the bytes of unique files and of every set's origins, paths
/// and repos) and the disk's floor. New files' bytes go into the file store before
/// the set's records commit, and before the database's write lock is taken, so the
/// lock is held only for the records.
pub async fn store(state: &AppState, account_id: &str, set: &NewSet) -> Result<Stored, StoreError> {
    let pool = &state.pool;
    let bytes = stored_size(set.bytes());
    let metadata_bytes = stored_size(set.metadata_bytes());
    let at = now();
    let today = format!("{}%", &at[..10]);
    // Every upload, and everything that deletes a file's record, holds this lock in
    // the one server process, so what the checks below read of sets and files stays
    // true until this set's records commit, without the database's write lock. A
    // ban takes no such lock, so the transaction checks for one again.
    let writer = state.files.write().await;
    let mut reader = pool.acquire().await?;
    let spent = sqlx::query_scalar!(
        r#"SELECT COALESCE(SUM(bytes), 0) AS "spent!: i64" FROM attachment_sets
           WHERE account_id = ? AND created_at LIKE ?"#,
        account_id,
        today
    )
    .fetch_one(&mut *reader)
    .await?;
    if spent + bytes > ACCOUNT_DAILY_BYTES {
        return Ok(Stored::DailyCap);
    }
    let banned = banned_paths(&mut reader, &set.files).await?;
    if !banned.is_empty() {
        return Ok(Stored::Banned(banned));
    }
    let mut new = Vec::new();
    let mut seen = HashSet::new();
    for file in &set.files {
        let stored =
            sqlx::query_scalar!("SELECT 1 AS found FROM files WHERE sha256 = ?", file.sha256)
                .fetch_optional(&mut *reader)
                .await?;
        if stored.is_none() && seen.insert(file.sha256.as_str()) {
            new.push(file);
        }
    }
    let new_bytes: i64 = new.iter().map(|file| stored_size(file.text.len())).sum();
    let added = new_bytes + metadata_bytes;
    let held = sqlx::query_scalar!(
        r#"SELECT (SELECT COALESCE(SUM(bytes), 0) FROM files)
                + (SELECT COALESCE(SUM(metadata_bytes), 0) FROM attachment_sets
                   WHERE deleted_at IS NULL) AS "held!: i64""#
    )
    .fetch_one(&mut *reader)
    .await?;
    drop(reader);
    if held + added > state.attachment_store_cap_bytes {
        return Ok(Stored::Full(Limit::StoreCap));
    }
    // Non-negative, as a sum of sizes.
    let added = u64::try_from(added).unwrap_or(u64::MAX);
    if state.files.free_bytes()?.saturating_sub(added) < state.attachment_disk_floor_bytes {
        return Ok(Stored::Full(Limit::DiskFloor));
    }
    let texts: Vec<(&str, &str)> = new
        .iter()
        .map(|file| (file.sha256.as_str(), file.text.as_str()))
        .collect();
    let unrecorded = || -> Vec<String> { new.iter().map(|file| file.sha256.clone()).collect() };
    if let Err(error) = writer.put_all(&texts).await {
        remove_bytes(&writer, &unrecorded()).await;
        return Err(error.into());
    }
    // After an error here the records may have committed all the same, so the bytes
    // stay, and `sweep` removes them if no record holds them.
    let stored = record(pool, account_id, set, bytes, metadata_bytes, &at).await?;
    if !matches!(stored, Stored::Saved { .. }) {
        // Refused in the transaction: no record holds the bytes just written.
        remove_bytes(&writer, &unrecorded()).await;
    }
    Ok(stored)
}

/// The paths of `files` that hold a banned file.
async fn banned_paths(
    conn: &mut SqliteConnection,
    files: &[NewFile],
) -> Result<Vec<String>, sqlx::Error> {
    let mut banned = Vec::new();
    for file in files {
        if removed_files::is_banned(conn, &file.sha256).await? {
            banned.push(file.path.clone());
        }
    }
    Ok(banned)
}

/// Commits the records of `set`, whose new files' bytes are in the store: the set,
/// its repos, and its files.
async fn record(
    pool: &SqlitePool,
    account_id: &str,
    set: &NewSet,
    bytes: i64,
    metadata_bytes: i64,
    at: &str,
) -> Result<Stored, StoreError> {
    let mut transaction = crate::db::write_tx(pool).await?;
    // Checked in this transaction, so no ban lands between the check and the
    // inserts below.
    let banned = banned_paths(&mut transaction, &set.files).await?;
    if !banned.is_empty() {
        return Ok(Stored::Banned(banned));
    }
    let id = format!("as-{}", ids::new_id());
    let set_seq = sqlx::query_scalar!(
        r#"INSERT INTO attachment_sets (id, account_id, bytes, metadata_bytes, created_at)
           VALUES (?, ?, ?, ?, ?)
           RETURNING seq AS "seq!: i64""#,
        id,
        account_id,
        bytes,
        metadata_bytes,
        at
    )
    .fetch_one(&mut *transaction)
    .await?;
    for (name, url) in &set.repos {
        sqlx::query!(
            "INSERT INTO attachment_repos (set_seq, name, url) VALUES (?, ?, ?)",
            set_seq,
            name,
            url
        )
        .execute(&mut *transaction)
        .await?;
    }
    for file in &set.files {
        let file_bytes = stored_size(file.text.len());
        sqlx::query!(
            "INSERT OR IGNORE INTO files (sha256, bytes, created_at) VALUES (?, ?, ?)",
            file.sha256,
            file_bytes,
            at
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query!(
            "INSERT INTO attachment_files (set_seq, path, origin, sha256) VALUES (?, ?, ?, ?)",
            set_seq,
            file.path,
            file.origin,
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
        r#"SELECT seq AS "seq!: i64" FROM attachment_sets
           WHERE id = ? AND account_id = ? AND deleted_at IS NULL"#,
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
    /// The URL, or `no remote`, of the repo its origin names, when the set
    /// declared it.
    pub repo_url: Option<String>,
    pub sha256: String,
    pub bytes: i64,
    /// The archive's owner removed it, so it isn't served.
    pub removed: bool,
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
    let mut files = sqlx::query_as!(
        SetFile,
        r#"SELECT af.path, af.origin, af.repo_url, af.sha256, f.bytes,
                  f.removed_at IS NOT NULL AS "removed!: bool"
           FROM attachment_files af JOIN files f ON f.sha256 = af.sha256
           WHERE af.set_seq = ? ORDER BY af.path"#,
        set.seq
    )
    .fetch_all(pool)
    .await?;
    let repos: HashMap<String, String> = sqlx::query!(
        "SELECT name, url FROM attachment_repos WHERE set_seq = ?",
        set.seq
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|repo| (repo.name, repo.url))
    .collect();
    // A file's own row holds its repo's URL only in the sets stored before each
    // set's repos were stored once.
    for file in files.iter_mut().filter(|file| file.repo_url.is_none()) {
        if let Some((Repo::Named(name), _)) = split_origin(&file.origin) {
            file.repo_url = repos.get(name).cloned();
        }
    }
    Ok(Some(EntrySet { id: set.id, files }))
}

/// What the screen and its reasons call a set's origins and repos.
pub const ORIGINS: &str = "the attached files' origins";

/// The origins of the files in the set entry `seq` cites, one a line, and the URL of
/// each repo they name, as the entry's page shows them, under a line saying what
/// they are, for the screen (design.md, Attached code); `None` when it cites no set.
/// A file's contents are screened once by hash, but its origin is the set's, so an
/// origin such as `author:<an instruction to readers>` needs a screen of its own.
pub async fn origins_text(pool: &SqlitePool, seq: i64) -> Result<Option<String>, sqlx::Error> {
    let Some(set) = entry_set(pool, seq).await? else {
        return Ok(None);
    };
    let mut repos = BTreeMap::new();
    let mut text = format!("--- {ORIGINS}, listed by the server ---\n");
    for file in &set.files {
        text.push_str(&file.origin);
        text.push('\n');
        if let (Some((Repo::Named(name), _)), Some(url)) =
            (split_origin(&file.origin), &file.repo_url)
        {
            repos.insert(name, url);
        }
    }
    if !repos.is_empty() {
        text.push_str("--- the repos they name ---\n");
        for (name, url) in repos {
            text.push_str(&format!("{name}: {url}\n"));
        }
    }
    Ok(Some(text))
}

/// Whether a public entry's set holds file `sha256`, and the owner hasn't removed it,
/// so the files host may serve it. A banned file was removed when it was banned, and
/// can't be uploaded again, so this covers bans too.
pub async fn is_public_file(pool: &SqlitePool, sha256: &str) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar!(
        "SELECT 1 AS found FROM attachment_files af
         JOIN entries e ON e.attachment_set_seq = af.set_seq
         JOIN files f ON f.sha256 = af.sha256
         WHERE af.sha256 = ? AND e.status = 'public' AND f.removed_at IS NULL
         LIMIT 1",
        sha256
    )
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Deletes the sets no entry cites, with their files' places in them and, by the
/// schema's cascade, their repos: only
/// `account_id`'s when given, and only those uploaded before `uploaded_before`, a
/// stored time, when given. The files' records go with [`delete_lone_files`].
pub async fn delete_uncited_sets(
    transaction: &mut SqliteConnection,
    account_id: Option<&str>,
    uploaded_before: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM attachment_files WHERE set_seq IN (
             SELECT s.seq FROM attachment_sets s
             WHERE (?1 IS NULL OR s.account_id = ?1) AND (?2 IS NULL OR s.created_at < ?2)
                 AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.attachment_set_seq = s.seq))",
        account_id,
        uploaded_before
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!(
        "DELETE FROM attachment_sets
         WHERE (?1 IS NULL OR account_id = ?1) AND (?2 IS NULL OR created_at < ?2)
             AND NOT EXISTS (
                 SELECT 1 FROM entries e WHERE e.attachment_set_seq = attachment_sets.seq)",
        account_id,
        uploaded_before
    )
    .execute(&mut *transaction)
    .await?;
    Ok(())
}

/// Deletes set `seq`'s files and repos from it when no entry cites it, as when the entries
/// that cited it are deleted, and marks it deleted at `at`: no entry may cite it
/// again, and its row stays until uncited sets expire, so its bytes still count
/// toward its account's day. The files' records go with [`delete_lone_files`].
pub async fn delete_uncited_set(
    transaction: &mut SqliteConnection,
    seq: i64,
    at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM attachment_files WHERE set_seq = ?1
           AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.attachment_set_seq = ?1)",
        seq
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!(
        "DELETE FROM attachment_repos WHERE set_seq = ?1
           AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.attachment_set_seq = ?1)",
        seq
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!(
        "UPDATE attachment_sets SET deleted_at = ?2 WHERE seq = ?1
           AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.attachment_set_seq = ?1)",
        seq,
        at
    )
    .execute(&mut *transaction)
    .await?;
    Ok(())
}

/// Deletes the record of every file no set holds, and returns their hashes. Their
/// bytes stay on disk until [`remove_bytes`] or [`sweep`] removes them, after the
/// transaction commits; `_lock` proves the store's lock is held until then. A file
/// the owner removed stays, record and bytes, for analysis, as an entry the owner
/// removed outlives its account (design.md, Admin), and so its removal holds when
/// the same bytes are attached again.
pub async fn delete_lone_files(
    _lock: &Writer<'_>,
    transaction: &mut SqliteConnection,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "DELETE FROM files WHERE removed_at IS NULL AND NOT EXISTS (
             SELECT 1 FROM attachment_files af WHERE af.sha256 = files.sha256)
         RETURNING sha256"
    )
    .fetch_all(transaction)
    .await
}

/// Deletes the sets no entry cites that were uploaded before `uploaded_before`, a
/// stored time, and the records of the files no set holds then. Their bytes are
/// left to [`sweep`], which the caller runs next under the same lock.
pub async fn expire_uncited_sets(
    pool: &SqlitePool,
    lock: &Writer<'_>,
    uploaded_before: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = crate::db::write_tx(pool).await?;
    delete_uncited_sets(&mut transaction, None, Some(uploaded_before)).await?;
    delete_lone_files(lock, &mut transaction).await?;
    transaction.commit().await
}

/// Removes the bytes on disk that no file's record holds, such as those a crash
/// left between writing a file and committing its record, and any half-written file.
pub async fn sweep(pool: &SqlitePool, writer: &Writer<'_>) -> Result<(), StoreError> {
    let recorded: HashSet<String> = sqlx::query_scalar!("SELECT sha256 FROM files")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    let unrecorded: Vec<String> = writer
        .stored()
        .await?
        .into_iter()
        .filter(|sha256| !recorded.contains(sha256))
        .collect();
    remove_bytes(writer, &unrecorded).await;
    Ok(())
}

/// Removes the bytes of files `hashes`, whose records are gone. A file that can't
/// be removed is logged, and the next sweep tries again.
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
/// in the [`FileStore`](crate::file_store::FileStore).
pub struct EntryFile {
    pub sha256: String,
    pub path: String,
    pub bytes: i64,
    pub verdict: Option<String>,
    pub reason: Option<String>,
    /// The archive's owner removed it, so it isn't served.
    pub removed: bool,
    /// The owner removed it as a hazard, so no entry citing it passes screening.
    pub banned: bool,
}

/// The files of the set entry `seq` cites, by path; none when it cites no set.
pub async fn entry_files(pool: &SqlitePool, seq: i64) -> Result<Vec<EntryFile>, sqlx::Error> {
    sqlx::query_as!(
        EntryFile,
        r#"SELECT af.sha256, af.path, f.bytes, f.screen_verdict AS verdict, f.screen_reason AS reason,
                  f.removed_at IS NOT NULL AS "removed!: bool",
                  EXISTS (SELECT 1 FROM banned_files b WHERE b.sha256 = af.sha256) AS "banned!: bool"
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
pub async fn first_notice_today(
    conn: &mut SqliteConnection,
    kind: &str,
) -> Result<bool, sqlx::Error> {
    let day = now()[..10].to_string();
    let done = sqlx::query!(
        "INSERT OR IGNORE INTO owner_notices (kind, day) VALUES (?, ?)",
        kind,
        day
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}
