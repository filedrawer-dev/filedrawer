//! What the process tests share: fixtures in scratch folders, and reading output.

use std::ffi::OsStr;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fixture's path in a scratch directory of its own, which goes, with anything a test
/// wrote beside the file, when this drops.
#[derive(Debug)]
pub struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Deref for Fixture {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<OsStr> for Fixture {
    fn as_ref(&self) -> &OsStr {
        self.path.as_os_str()
    }
}

impl AsRef<Path> for Fixture {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// A fixture written to a fresh scratch directory, from text compiled into the test.
/// Paths from `CARGO_MANIFEST_DIR` would name the temporary tree the commit hook built
/// the test in, which is gone by the next run. Each call gets its own directory, so
/// tests running in parallel never read a file another is rewriting. An unknown name
/// gives a path to no file.
pub fn fixture(name: &str) -> Fixture {
    let dir = tempfile::Builder::new()
        .prefix("cli-tests-")
        .tempdir()
        .unwrap();
    let path = dir.path().join(name);
    let text = match name {
        // The analyzer's example entry, which its own tests build on.
        "example.md" => Some(include_str!(
            "../../../../analyzer/tests/it/fixtures/example.md"
        )),
        "refused.md" => Some(include_str!("../fixtures/refused.md")),
        _ => None,
    };
    if let Some(text) = text {
        std::fs::write(&path, text).unwrap();
    }
    Fixture { _dir: dir, path }
}

/// Runs the `filedrawer` binary with `args`.
pub fn filedrawer<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(args: I) -> Output {
    Command::new(env!("CARGO_BIN_EXE_filedrawer"))
        .args(args)
        .output()
        .expect("filedrawer runs")
}

/// Runs the `entry-check` binary with `args`.
pub fn entry_check<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(args: I) -> Output {
    Command::new(env!("CARGO_BIN_EXE_entry-check"))
        .args(args)
        .output()
        .expect("entry-check runs")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}
