//! Seam: the `filedrawer` binary, run as a process. `filedrawer check` is
//! `entry-check` under another name, so these tests check that it is the same, and
//! entry_check.rs checks what it does.

use std::ffi::OsStr;

use crate::common::{entry_check, filedrawer, fixture, stderr, stdout};

#[test]
fn help_says_what_file_drawer_is_and_lists_check() {
    for flag in ["-h", "--help"] {
        let output = filedrawer([flag]);
        assert_eq!(output.status.code(), Some(0));
        let out = stdout(&output);
        assert!(out.starts_with("usage: filedrawer COMMAND"), "{out}");
        assert!(out.contains("https://filedrawer.dev"), "{out}");
        assert!(out.contains("\n  check FILE..."), "{out}");
    }
}

#[test]
fn no_command_prints_help_to_stderr_and_exits_2() {
    let output = filedrawer::<[&str; 0], &str>([]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).starts_with("usage: filedrawer COMMAND"));
    assert!(stdout(&output).is_empty());
}

#[test]
fn an_unknown_command_is_named_and_exits_2() {
    let output = filedrawer(["search", "steering"]);
    assert_eq!(output.status.code(), Some(2));
    let err = stderr(&output);
    assert!(err.starts_with("filedrawer: no command `search`"), "{err}");
    assert!(err.contains("usage: filedrawer COMMAND"), "{err}");
}

#[test]
fn version_prints_the_package_version() {
    let output = filedrawer(["--version"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        format!("filedrawer {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn check_prints_and_exits_as_entry_check_does() {
    let example = fixture("example.md");
    let refused = fixture("refused.md");
    let missing = fixture("no-such-file.md");
    for files in [
        vec![&example],
        vec![&example, &refused],
        vec![&example, &missing],
    ] {
        let ours = filedrawer(
            std::iter::once(OsStr::new("check")).chain(files.iter().map(|p| p.as_os_str())),
        );
        let theirs = entry_check(files.iter().map(|p| p.as_os_str()));
        assert_eq!(ours.status.code(), theirs.status.code(), "{files:?}");
        assert_eq!(stdout(&ours), stdout(&theirs), "{files:?}");
        // Each names itself in an error.
        assert_eq!(
            stderr(&ours),
            stderr(&theirs).replace("entry-check:", "filedrawer check:"),
            "{files:?}"
        );
    }
}

#[test]
fn check_help_names_the_subcommand() {
    let output = filedrawer(["check", "-h"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).starts_with("usage: filedrawer check FILE..."));
    let output = filedrawer(["check"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).starts_with("usage: filedrawer check FILE..."));
}
