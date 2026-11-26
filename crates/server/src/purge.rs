//! The owner's purge, for an erasure request (design.md, Admin; #15): it deletes
//! from the database the named entries with every version of their chain, or an
//! account's personal data, and the attached files no other entry uses. A file
//! purged with an entry removed for a hazard is banned first.
//!
//! Each purge is logged, IDs only, in `purges.log` beside the database, before its
//! transaction commits, and the database records each line it has applied. At every
//! start the server applies the lines the database lacks, so a database file swapped
//! for an older copy on the same disk loses the purged data again. The log lives on
//! that same disk, so a restore of the whole disk, such as Hetzner's, rolls the log
//! back too; only its copy in B2 (#4) will survive one, and until then the owner
//! redoes each purge by hand after such a restore (`ops/restore-production.md`).

use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::AppError;
use crate::file_store::Writer;
use crate::mail_queue::{self, Queued};
use crate::removed_entries::{self, ChainMember};
use crate::removed_files::{self, Removal};
use crate::time::{now, now_utc};
use crate::{AppState, accounts, attachments, db, deletion, series};

#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("can't write the purge log {path}: {error}")]
    WriteLog { path: String, error: std::io::Error },
    #[error("can't read the purge log {path}: {error}")]
    ReadLog { path: String, error: std::io::Error },
    /// The log holds lines that aren't records, so no new purge is logged after them.
    #[error(
        "lines {} of the purge log {path} aren't purge records, so purges are refused until they are fixed or removed",
        .lines.iter().map(usize::to_string).collect::<Vec<_>>().join(", ")
    )]
    Damaged { path: String, lines: Vec<usize> },
}

impl From<PurgeError> for AppError {
    fn from(error: PurgeError) -> AppError {
        match error {
            PurgeError::Database(error) => AppError::Database(error),
            PurgeError::Damaged { .. } => AppError::Conflict(error.to_string()),
            other => {
                tracing::error!(error = %other, "purge");
                AppError::Io(std::io::Error::other("the purge log"))
            }
        }
    }
}

/// One purge, as its line in the log holds it: IDs and hashes, and no content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// When the owner purged, UTC.
    pub at: String,
    /// The entries deleted, every version of each chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<String>,
    /// The account whose personal data was deleted. A GitHub ID the owner typed isn't
    /// logged, since the privacy page promises the bare ID is tied to no account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The entries were removed for a hazard, so their files were banned.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hazard: bool,
    /// The attached files whose record and bytes the purge deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The attached files it banned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub banned: Vec<String>,
}

/// What a purge deletes, before it runs.
#[derive(Default)]
struct Request {
    entries: Vec<String>,
    account: Option<String>,
    github_id: Option<i64>,
    hazard: bool,
    /// Files to delete when no set holds them, besides the entries' own: a logged
    /// purge's, applied again.
    files: Vec<String>,
    /// Hashes to keep banned: a logged purge's, applied again.
    banned: Vec<String>,
}

/// A file one of the versions holds, for the confirm page.
pub struct HeldFile {
    pub sha256: String,
    pub path: String,
    /// An entry outside the chain uses it too, so the purge doesn't delete it; a
    /// hazard's purge still removes and bans it, for every entry.
    pub shared: bool,
}

/// What purging entry `id` would delete.
pub struct ChainPreview {
    /// Every version of its chain, oldest first.
    pub versions: Vec<ChainMember>,
    pub files: Vec<HeldFile>,
    /// The accounts that uploaded a set holding any of the files, which a hazard's
    /// purge puts under the review guard.
    pub uploaders: Vec<String>,
}

impl ChainPreview {
    /// The files another entry uses too.
    pub fn shared(&self) -> impl Iterator<Item = &HeldFile> {
        self.files.iter().filter(|file| file.shared)
    }
}

/// What purging an entry would delete; `None` when there is no such entry.
pub async fn chain_preview(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<ChainPreview>, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let versions = removed_entries::chain_of(&mut connection, id).await?;
    if versions.is_empty() {
        return Ok(None);
    }
    let seqs: BTreeSet<i64> = versions.iter().map(|version| version.seq).collect();
    let mut files: Vec<HeldFile> = Vec::new();
    let mut uploaders = BTreeSet::new();
    for &seq in &seqs {
        let held = sqlx::query!(
            "SELECT af.sha256, af.path FROM entries e
             JOIN attachment_files af ON af.set_seq = e.attachment_set_seq
             WHERE e.seq = ? ORDER BY af.path",
            seq
        )
        .fetch_all(&mut *connection)
        .await?;
        for file in held {
            if files.iter().any(|seen| seen.sha256 == file.sha256) {
                continue;
            }
            let users = sqlx::query_scalar!(
                r#"SELECT e.seq AS "seq!: i64" FROM attachment_files af
                   JOIN entries e ON e.attachment_set_seq = af.set_seq
                   WHERE af.sha256 = ?"#,
                file.sha256
            )
            .fetch_all(&mut *connection)
            .await?;
            // Whom `removed_files::remove_in` guards for a hazard.
            uploaders.extend(
                sqlx::query_scalar!(
                    "SELECT DISTINCT s.account_id FROM attachment_files af
                     JOIN attachment_sets s ON s.seq = af.set_seq WHERE af.sha256 = ?",
                    file.sha256
                )
                .fetch_all(&mut *connection)
                .await?,
            );
            files.push(HeldFile {
                shared: users.iter().any(|user| !seqs.contains(user)),
                sha256: file.sha256,
                path: file.path,
            });
        }
    }
    Ok(Some(ChainPreview {
        versions,
        files,
        uploaders: uploaders.into_iter().collect(),
    }))
}

/// Purges every version of entry `id`'s chain, banning their files first when
/// `hazard`. `None` when there is no such entry.
pub async fn purge_chain(
    state: &AppState,
    id: &str,
    hazard: bool,
) -> Result<Option<Record>, PurgeError> {
    let versions = removed_entries::chain_of(&mut *state.pool.acquire().await?, id).await?;
    if versions.is_empty() {
        return Ok(None);
    }
    let request = Request {
        entries: versions.into_iter().map(|version| version.id).collect(),
        hazard,
        ..Request::default()
    };
    Ok(Some(carry_out(state, &request, Mode::New).await?))
}

/// An account as its purge's confirm page shows it.
pub struct AccountPreview {
    pub id: String,
    /// Its name and address, unless it is deleted.
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub deleted: bool,
    /// The GitHub account linked to it, as ID and login.
    pub github: Option<(i64, String)>,
    /// Whether an exemption record names it, kept after an unlink.
    pub exemption: bool,
    /// Its entries in all, and those of them public.
    pub entries: i64,
    pub public: i64,
}

/// What purging account `id` would delete; `None` when there is no such account.
pub async fn account_preview(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<AccountPreview>, sqlx::Error> {
    let Some(account) = sqlx::query!(
        r#"SELECT id AS "id!", display_name, email,
                  deleted_at IS NOT NULL AS "deleted!: bool",
                  (SELECT COUNT(*) FROM entries e WHERE e.account_id = a.id) AS "entries!: i64",
                  (SELECT COUNT(*) FROM entries e
                   WHERE e.account_id = a.id AND e.status = 'public') AS "public!: i64",
                  EXISTS (SELECT 1 FROM github_exemptions x WHERE x.account_id = a.id)
                      AS "exemption!: bool"
           FROM accounts a WHERE id = ?"#,
        id
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let github = sqlx::query!(
        "SELECT github_id, login FROM github_links WHERE account_id = ?",
        id
    )
    .fetch_optional(pool)
    .await?
    .map(|link| (link.github_id, link.login));
    Ok(Some(AccountPreview {
        display_name: (!account.deleted).then_some(account.display_name),
        email: account.email,
        id: account.id,
        deleted: account.deleted,
        github,
        exemption: account.exemption,
        entries: account.entries,
        public: account.public,
    }))
}

/// Purges account `id`'s personal data: deletes the account, if it isn't deleted
/// yet, as its own deletion does, and with it the GitHub ID kept for the exemption,
/// any pending GitHub sign-in or link, and the exemption record of `github_id`, when
/// no other account holds it. With `entries`, every entry of it goes too, public and
/// removed ones included. `None` when there is no such account.
pub async fn purge_account(
    state: &AppState,
    id: &str,
    github_id: Option<i64>,
    entries: bool,
) -> Result<Option<Record>, PurgeError> {
    let exists = sqlx::query_scalar!(
        r#"SELECT 1 AS "found!: i64" FROM accounts WHERE id = ?"#,
        id
    )
    .fetch_optional(&state.pool)
    .await?;
    if exists.is_none() {
        return Ok(None);
    }
    let entries = if entries {
        sqlx::query_scalar!(
            "SELECT id FROM entries WHERE account_id = ? ORDER BY seq",
            id
        )
        .fetch_all(&state.pool)
        .await?
    } else {
        Vec::new()
    };
    let request = Request {
        entries,
        account: Some(id.to_string()),
        github_id,
        ..Request::default()
    };
    Ok(Some(carry_out(state, &request, Mode::New).await?))
}

/// Applies each purge in the log that this database hasn't, as the server does when
/// it starts, so a database file swapped for an older copy loses that data again. A
/// line that isn't a record is skipped, and every admin is mailed about it once a day;
/// new purges are refused until it is fixed. Returns how many purges it applied.
pub async fn reapply(state: &AppState) -> Result<usize, PurgeError> {
    let log = read_log(&state.purge_log)?;
    if !log.bad_lines.is_empty() {
        tell_owner_of_bad_lines(state, &log.bad_lines).await?;
    }
    let mut applied = 0;
    for logged in &log.records {
        let done = sqlx::query_scalar!(
            r#"SELECT 1 AS "found!: i64" FROM applied_purges WHERE line_sha256 = ?"#,
            logged.line_sha256
        )
        .fetch_optional(&state.pool)
        .await?;
        if done.is_some() {
            continue;
        }
        let record = &logged.record;
        let request = Request {
            entries: record.entries.clone(),
            account: record.account.clone(),
            github_id: None,
            hazard: record.hazard,
            files: record.files.clone(),
            banned: record.banned.clone(),
        };
        carry_out(state, &request, Mode::Again(&logged.line_sha256)).await?;
        applied += 1;
    }
    Ok(applied)
}

/// Mails every admin, once a day, that the purge log has lines it can't read, naming
/// them only by number, since a damaged line's text could be anything.
async fn tell_owner_of_bad_lines(state: &AppState, lines: &[usize]) -> Result<(), sqlx::Error> {
    let numbers = lines
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    tracing::error!(
        path = %state.purge_log.display(),
        lines = %numbers,
        "lines of the purge log aren't purge records: skipped, and purges are refused until they are fixed"
    );
    let mut transaction = db::write_tx(&state.pool).await?;
    if !attachments::first_notice_today(&mut transaction, "purge log damaged").await? {
        return Ok(());
    }
    let notice = Queued::Notice {
        subject: "File Drawer: the purge log is damaged".to_string(),
        body: format!(
            "{} skipped lines {numbers} of {} when it started, since they aren't purge records, so it didn't apply those purges again. It refuses new purges until the lines are fixed or removed.\n",
            state.public_url,
            state.purge_log.display()
        ),
    };
    for admin in accounts::admin_ids(&mut transaction).await? {
        mail_queue::queue_in(&mut transaction, &admin, &notice, now_utc()).await?;
    }
    transaction.commit().await?;
    mail_queue::wake(state);
    Ok(())
}

/// Whether a purge is new, and logged as it commits, or a logged one applied again.
#[derive(Clone, Copy)]
enum Mode<'a> {
    New,
    /// The SHA-256 of its line in the log.
    Again(&'a str),
}

/// Carries out `request` in one transaction, which records that this database applied
/// its line. A new purge is refused while the log holds a line that isn't a record,
/// and appends its own line before the transaction commits, so a purge that committed
/// is always in the log.
async fn carry_out(
    state: &AppState,
    request: &Request,
    mode: Mode<'_>,
) -> Result<Record, PurgeError> {
    let writer = state.files.write().await;
    if matches!(mode, Mode::New) {
        let log = read_log(&state.purge_log)?;
        if !log.bad_lines.is_empty() {
            return Err(PurgeError::Damaged {
                path: state.purge_log.display().to_string(),
                lines: log.bad_lines,
            });
        }
    }
    let mut transaction = db::write_tx(&state.pool).await?;
    let at = now();
    let mut seqs = Vec::new();
    for id in &request.entries {
        let seq = sqlx::query_scalar!(r#"SELECT seq AS "seq!: i64" FROM entries WHERE id = ?"#, id)
            .fetch_optional(&mut *transaction)
            .await?;
        seqs.extend(seq);
    }
    let mut entry_files = BTreeSet::new();
    for &seq in &seqs {
        entry_files.extend(
            sqlx::query_scalar!(
                "SELECT af.sha256 FROM entries e
                 JOIN attachment_files af ON af.set_seq = e.attachment_set_seq
                 WHERE e.seq = ?",
                seq
            )
            .fetch_all(&mut *transaction)
            .await?,
        );
    }
    let mut banned: BTreeSet<String> = request.banned.iter().cloned().collect();
    if request.hazard {
        // Only the files the entries still hold, so a logged purge applied again
        // guards nobody unless a restore brought its entries back.
        for sha256 in &entry_files {
            removed_files::remove_in(&mut transaction, sha256, Removal::Hazard, &at).await?;
            banned.insert(sha256.clone());
        }
    }
    let mut held = entry_files;
    held.extend(request.files.iter().cloned());
    for sha256 in &banned {
        sqlx::query!(
            "INSERT OR IGNORE INTO banned_files (sha256, banned_at) VALUES (?, ?)",
            sha256,
            at
        )
        .execute(&mut *transaction)
        .await?;
    }
    deletion::delete_entries(&mut transaction, &seqs).await?;
    for id in &request.entries {
        sqlx::query!("DELETE FROM deleted_entries WHERE id = ?", id)
            .execute(&mut *transaction)
            .await?;
    }
    let mut files: BTreeSet<String> = BTreeSet::new();
    if let Some(account_id) = &request.account {
        files.extend(
            purge_account_data(&writer, &mut transaction, account_id, request.github_id).await?,
        );
    }
    // The entries may have been all that held a series version back.
    series::mark_readable(&mut transaction).await?;
    files.extend(attachments::delete_lone_files(&writer, &mut transaction).await?);
    // A file the owner removed outlives a deletion, for analysis, but not a purge.
    for sha256 in &held {
        let gone = sqlx::query_scalar!(
            "DELETE FROM files WHERE sha256 = ?1
               AND NOT EXISTS (SELECT 1 FROM attachment_files af WHERE af.sha256 = ?1)
             RETURNING sha256",
            sha256
        )
        .fetch_optional(&mut *transaction)
        .await?;
        files.extend(gone);
    }
    let record = Record {
        at,
        entries: request.entries.clone(),
        account: request.account.clone(),
        hazard: request.hazard,
        files: files.into_iter().collect(),
        banned: banned.into_iter().collect(),
    };
    let line = serde_json::to_string(&record).expect("a record always serializes");
    let line_sha256 = match mode {
        Mode::New => attachments::sha256_hex(&line),
        Mode::Again(line_sha256) => line_sha256.to_string(),
    };
    sqlx::query!(
        "INSERT OR IGNORE INTO applied_purges (line_sha256, applied_at) VALUES (?, ?)",
        line_sha256,
        record.at
    )
    .execute(&mut *transaction)
    .await?;
    if matches!(mode, Mode::New) {
        append(&state.purge_log, &line)?;
    }
    transaction.commit().await?;
    attachments::remove_bytes(&writer, &record.files).await;
    Ok(record)
}

/// Deletes account `account_id`'s personal data in the caller's transaction, and
/// returns the files whose bytes go once it commits. An account deleted already lost
/// the rest at its deletion.
async fn purge_account_data(
    writer: &Writer<'_>,
    transaction: &mut SqliteConnection,
    account_id: &str,
    github_id: Option<i64>,
) -> Result<Vec<String>, sqlx::Error> {
    let Some(deleted) = sqlx::query_scalar!(
        r#"SELECT deleted_at IS NOT NULL AS "deleted!: bool" FROM accounts WHERE id = ?"#,
        account_id
    )
    .fetch_optional(&mut *transaction)
    .await?
    else {
        return Ok(Vec::new());
    };
    let mut github_ids: Vec<i64> = sqlx::query_scalar!(
        "SELECT github_id FROM github_links WHERE account_id = ?1
         UNION SELECT github_id FROM github_exemptions WHERE account_id = ?1",
        account_id
    )
    .fetch_all(&mut *transaction)
    .await?;
    github_ids.extend(github_id);
    // Before the deletion, which keeps the bare ID by clearing the record's account.
    sqlx::query!(
        "DELETE FROM github_exemptions WHERE account_id = ?",
        account_id
    )
    .execute(&mut *transaction)
    .await?;
    for github_id in &github_ids {
        // An ID another account holds is that account's, whatever the owner typed.
        sqlx::query!(
            "DELETE FROM github_exemptions WHERE github_id = ? AND account_id IS NULL",
            github_id
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query!("DELETE FROM github_signups WHERE github_id = ?", github_id)
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query!("DELETE FROM github_starts WHERE account_id = ?", account_id)
        .execute(&mut *transaction)
        .await?;
    if deleted {
        return Ok(Vec::new());
    }
    deletion::delete_in(writer, transaction, account_id).await
}

/// Appends `line`, a record, to the log at `path`, and syncs it to disk, and the
/// folder too when this made the file. A torn last line, which a crash mid-write
/// leaves for a purge that never committed, is cut off first.
fn append(path: &Path, line: &str) -> Result<(), PurgeError> {
    let write = || -> std::io::Result<()> {
        let made = !path.exists();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;
        let mut text = Vec::new();
        file.read_to_end(&mut text)?;
        if text.last().is_some_and(|&last| last != b'\n') {
            let whole = text
                .iter()
                .rposition(|&byte| byte == b'\n')
                .map_or(0, |at| at + 1);
            file.set_len(u64::try_from(whole).expect("a file's length fits in u64"))?;
            file.seek(SeekFrom::End(0))?;
        }
        file.write_all(format!("{line}\n").as_bytes())?;
        file.sync_all()?;
        if made {
            // The new file's name is in its folder only once the folder is synced.
            let folder = path
                .parent()
                .filter(|folder| !folder.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::File::open(folder)?.sync_all()?;
        }
        Ok(())
    };
    write().map_err(|error| PurgeError::WriteLog {
        path: path.display().to_string(),
        error,
    })
}

/// A record read from the log, with the SHA-256 of its line, which the database
/// records once it has applied it.
pub struct Logged {
    pub record: Record,
    pub line_sha256: String,
}

/// What the log holds.
pub struct Log {
    pub records: Vec<Logged>,
    /// The numbers, from 1, of the lines that aren't records, but for a torn last one.
    pub bad_lines: Vec<usize>,
}

/// Every record in the log at `path`; none when there is no log yet. A torn last line
/// is left out, since its purge never committed; any other line that isn't a record
/// is listed in `bad_lines`.
pub fn read_log(path: &Path) -> Result<Log, PurgeError> {
    let mut log = Log {
        records: Vec::new(),
        bad_lines: Vec::new(),
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(log),
        Err(error) => {
            return Err(PurgeError::ReadLog {
                path: path.display().to_string(),
                error,
            });
        }
    };
    let torn = !text.is_empty() && !text.ends_with('\n');
    let lines: Vec<&str> = text.split_terminator('\n').collect();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(record) => log.records.push(Logged {
                record,
                line_sha256: attachments::sha256_hex(line),
            }),
            Err(_) if torn && index + 1 == lines.len() => {
                tracing::warn!(path = %path.display(), "the purge log's last line is torn; its purge never committed");
            }
            Err(_) => log.bad_lines.push(index + 1),
        }
    }
    Ok(log)
}
