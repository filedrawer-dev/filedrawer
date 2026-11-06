//! Attached files' bytes on disk (design.md, Attached code): each file once, at
//! `<root>/<first two hex digits>/<sha256>`. The database holds each file's record.
//!
//! A file is written to `<root>/tmp/`, synced and renamed into place, so a reader
//! never sees part of one. The bytes go in before their record commits, and come out
//! after its deletion commits; [`FileStore::write`]'s lock spans both, so an upload of
//! a file can't lose its bytes to a deletion of the same file running at once.

use std::io;
use std::path::{Path, PathBuf};

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

fn is_sha256(name: &str) -> bool {
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
    /// Stores `text` as file `sha256`, which must be its SHA-256 in hex. A file the
    /// store already holds is left as it is: the same hash means the same bytes.
    pub async fn put(&self, sha256: &str, text: &str) -> io::Result<()> {
        let path = self.store.path(sha256)?;
        if tokio::fs::try_exists(&path).await? {
            return Ok(());
        }
        let dir = path.parent().expect("a file's path has its directory");
        tokio::fs::create_dir_all(dir).await?;
        let tmp = self.store.root.join(TMP).join(sha256);
        let mut file = tokio::fs::File::create(&tmp).await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, text.as_bytes()).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, &path).await?;
        // The rename, and the directory if it is new, last only once their
        // directories are synced.
        sync_dir(dir).await?;
        sync_dir(&self.store.root).await
    }

    /// Deletes file `sha256`, if the store holds it.
    pub async fn remove(&self, sha256: &str) -> io::Result<()> {
        match tokio::fs::remove_file(self.store.path(sha256)?).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

async fn sync_dir(dir: &Path) -> io::Result<()> {
    tokio::fs::File::open(dir).await?.sync_all().await
}
