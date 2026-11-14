//! What the process tests share: fixtures in scratch folders, and reading output.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fixture written to a fresh scratch directory, from text compiled into the test.
/// Paths from `CARGO_MANIFEST_DIR` would name the temporary tree the commit hook built
/// the test in, which is gone by the next run. Each call gets its own directory, so
/// tests running in parallel never read a file another is rewriting. An unknown name
/// gives a path to no file.
pub fn fixture(name: &str) -> PathBuf {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cli-tests-{}-{call}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let text = match name {
        // The analyzer's example entry, which its own tests build on.
        "example.md" => include_str!("../../../../analyzer/tests/it/fixtures/example.md"),
        "refused.md" => include_str!("../fixtures/refused.md"),
        _ => return path,
    };
    std::fs::write(&path, text).unwrap();
    path
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
