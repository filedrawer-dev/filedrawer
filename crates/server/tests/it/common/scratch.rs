//! Paths in folders of their own, for tests that hand a file or folder to a binary or
//! reopen a database file.

use std::ops::Deref;
use std::path::{Path, PathBuf};

/// A path, not yet made, in a new folder named for `name`. The folder goes, with all
/// a test put in it, when this drops, even when the test panics: tests that built such
/// paths by hand left thousands of folders in /tmp.
#[derive(Debug)]
pub struct Scratch {
    dir: tempfile::TempDir,
    path: PathBuf,
}

impl Scratch {
    /// `file` in a new folder under the system's temporary folder.
    pub fn new(name: &str, file: &str) -> Self {
        Self::new_in(&std::env::temp_dir(), name, file)
    }

    /// `file` in a new folder under `parent`.
    pub fn new_in(parent: &Path, name: &str, file: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("filedrawer-{name}-"))
            .tempdir_in(parent)
            .unwrap();
        let path = dir.path().join(file);
        Scratch { dir, path }
    }

    /// `filedrawer.db` in a new folder under the system's temporary folder.
    pub fn database(name: &str) -> Self {
        Self::new(name, "filedrawer.db")
    }

    /// The folder holding the path.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}
