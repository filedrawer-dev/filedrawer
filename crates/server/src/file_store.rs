//! Attached files' bytes on disk (design.md, Attached code): each file once, at
//! `<root>/<first two hex digits>/<sha256>`. The database holds each file's record.
//!
//! A file is written to `<root>/tmp/`, synced and renamed into place, so a reader
//! never sees part of one; the files of one upload are synced together. The bytes go
//! in before their record commits, and come out
//! after its deletion commits; [`FileStore::write`]'s lock spans both, so an upload of
//! a file can't lose its bytes to a deletion of the same file running at once.
//! Whoever takes that lock and the database's write lock takes this one first.

use std::io;
use std::path::PathBuf;

use tokio::sync::{Mutex, MutexGuard};

/// The directory of attached files.
#[derive(Debug)]
pub struct FileStore {
    root: PathBuf,
    writing: Mutex<()>,
}

impl FileStore {
    /// The store at `root`, created if missing.
    pub fn open(root: PathBuf) -> io::Result<FileStore> {
        std::fs::create_dir_all(root.join(TMP))?;
        Ok(FileStore {
            root,
            writing: Mutex::new(()),
        })
    }

    /// The text of file `sha256`, or `None` when the store doesn't hold it.
    pub async fn read(&self, sha256: &str) -> io::Result<Option<String>> {
        match tokio::fs::read_to_string(self.path(sha256)?).await {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The bytes free on the store's filesystem for an unprivileged writer.
    pub fn free_bytes(&self) -> io::Result<u64> {
        let stats = rustix::fs::statvfs(&self.root)?;
        Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
    }

    /// The right to change the store, held while a change and its records commit.
    pub async fn write(&self) -> Writer<'_> {
        Writer {
            store: self,
            _held: self.writing.lock().await,
        }
    }

    /// Where file `sha256` is kept. Refuses anything but a SHA-256 in lower-case hex,
    /// so no name can reach outside the store.
    fn path(&self, sha256: &str) -> io::Result<PathBuf> {
        if !is_sha256(sha256) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`{sha256}` isn't a SHA-256 in lower-case hex"),
            ));
        }
        Ok(self.root.join(&sha256[..2]).join(sha256))
    }
}

/// Where a write keeps a file until it is complete.
const TMP: &str = "tmp";

/// Whether `name` is a SHA-256 in lower-case hex, as the store names files.
pub fn is_sha256(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Changes to the store, made while holding its lock.
pub struct Writer<'a> {
    store: &'a FileStore,
    _held: MutexGuard<'a, ()>,
}

impl Writer<'_> {
    /// Stores each `(sha256, text)` as file `sha256`, which must be the text's
    /// SHA-256 in hex and appear once. A file the store already holds is replaced by
    /// the same bytes. Nothing is written when a name isn't a SHA-256.
    ///
    /// The files are synced together, once before the renames and once after, so
    /// the time under the lock grows with their bytes and not with a sync for each.
    pub async fn put_all(&self, files: &[(&str, &str)]) -> io::Result<()> {
        let mut moves = Vec::with_capacity(files.len());
        for (sha256, _) in files {
            let tmp = self.store.root.join(TMP).join(sha256);
            moves.push((tmp, self.store.path(sha256)?));
        }
        if files.is_empty() {
            return Ok(());
        }
        for ((tmp, _), (_, text)) in moves.iter().zip(files) {
            tokio::fs::write(tmp, text).await?;
        }
        self.sync().await?;
        for (tmp, path) in &moves {
            let dir = path.parent().expect("a file's path has its directory");
            tokio::fs::create_dir_all(dir).await?;
            tokio::fs::rename(tmp, path).await?;
        }
        // The renames, and any new directory, last only once they are synced too.
        self.sync().await
    }

    /// Writes everything the store's filesystem holds in memory to disk: file
    /// contents, renames and new directories.
    async fn sync(&self) -> io::Result<()> {
        let root = tokio::fs::File::open(&self.store.root)
            .await?
            .into_std()
            .await;
        tokio::task::spawn_blocking(move || rustix::fs::syncfs(&root)).await??;
        Ok(())
    }

    /// Deletes file `sha256`, if the store holds it.
    pub async fn remove(&self, sha256: &str) -> io::Result<()> {
        match tokio::fs::remove_file(self.store.path(sha256)?).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// The hash of every file the store holds. Clears any temporary file a write
    /// left behind, which, with the lock held, no write is still making; one that
    /// can't be removed is logged and left.
    pub async fn stored(&self) -> io::Result<Vec<String>> {
        let mut hashes = Vec::new();
        let mut dirs = tokio::fs::read_dir(&self.store.root).await?;
        while let Some(dir) = dirs.next_entry().await? {
            let tmp = dir.file_name() == TMP;
            if !dir.file_type().await?.is_dir() {
                continue;
            }
            let mut files = tokio::fs::read_dir(dir.path()).await?;
            while let Some(file) = files.next_entry().await? {
                if tmp {
                    if let Err(error) = tokio::fs::remove_file(file.path()).await {
                        tracing::error!(path = %file.path().display(), %error, "removing a half-written attached file failed");
                    }
                } else if let Some(name) = file.file_name().to_str().filter(|name| is_sha256(name))
                {
                    hashes.push(name.to_string());
                }
            }
        }
        hashes.sort();
        Ok(hashes)
    }
}
