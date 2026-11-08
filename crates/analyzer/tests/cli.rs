//! Seam: the `entry-check` binary, run as a process.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fixture written to a fresh scratch directory, from text compiled into the test.
/// Paths from `CARGO_MANIFEST_DIR` would name the temporary tree the commit hook built
/// the test in, which is gone by the next run. Each call gets its own directory, so
/// tests running in parallel never read a file another is rewriting. An unknown name
/// gives a path to no file.
fn fixture(name: &str) -> PathBuf {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("entry-check-tests-{}-{call}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let text = match name {
        "example.md" => include_str!("fixtures/example.md"),
        "refused.md" => include_str!("fixtures/refused.md"),
        _ => return path,
    };
    std::fs::write(&path, text).unwrap();
    path
}

fn entry_check(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_entry-check"))
        .args(args)
        .output()
        .expect("entry-check runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn an_accepted_entry_prints_its_card_and_parsed_primary_row_and_exits_0() {
    let output = entry_check(&[&fixture("example.md")]);
    assert_eq!(output.status.code(), Some(0));
    let out = stdout(&output);
    assert!(out.contains("example.md: accepted, 0 warnings"), "{out}");
    assert!(
        out.contains("  <id>; experiment; null; main; rows: 1 null\n"),
        "{out}"
    );
    assert!(
        out.contains("primary row: model Llama-3-8B-Instruct; condition steered vs unsteered; estimate 0.012; ci_low -0.047; ci_high 0.070; n 520/520; claim null"),
        "{out}"
    );
}

#[test]
fn a_refused_entry_lists_every_reason_and_exits_1() {
    let output = entry_check(&[&fixture("example.md"), &fixture("refused.md")]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert!(out.contains("refused.md: refused"), "{out}");
    assert!(
        out.contains("refused: the required field `title` is missing"),
        "{out}"
    );
    assert!(
        out.contains("refused: the required field `outcome` is missing"),
        "{out}"
    );
    assert!(
        out.contains("refused: the `code` link isn't pinned"),
        "{out}"
    );
    assert!(out.contains("example.md: accepted"), "{out}");
}

#[test]
fn warnings_are_listed_and_point_to_feedback() {
    let example = fixture("example.md");
    let path = example.with_file_name("warned.md");
    let text = std::fs::read_to_string(example)
        .unwrap()
        .replace("stage: main", "stage: final");
    std::fs::write(&path, text).unwrap();
    let output = entry_check(&[&path]);
    assert_eq!(output.status.code(), Some(0));
    let out = stdout(&output);
    assert!(out.contains("warned.md: accepted, 1 warning\n"), "{out}");
    assert!(
        out.contains("warning: `stage: final` is not a value the server knows"),
        "{out}"
    );
    assert!(out.contains("POST /feedback"), "{out}");
}

#[test]
fn a_file_that_cant_be_read_exits_2() {
    let output = entry_check(&[&fixture("example.md"), &fixture("no-such-file.md")]);
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(err.contains("no-such-file.md"), "{err}");
    assert!(stdout(&output).contains("example.md: accepted"));
}

#[test]
fn no_arguments_print_usage_and_exit_2() {
    let output = entry_check(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("usage: entry-check FILE...")
    );
}

#[test]
fn help_prints_usage_and_exits_0() {
    let output = entry_check(&[Path::new("-h")]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("usage: entry-check FILE..."));
}

#[cfg(unix)]
#[test]
fn a_file_name_that_isnt_utf8_is_reported_not_a_panic() {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"\xff.md");
    let output = entry_check(&[Path::new(name)]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("can't read"));
}

#[test]
fn a_series_cite_the_server_cant_read_makes_entry_check_refuse() {
    let path = fixture("example.md");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replacen("tags:", "series: ser_madeUp1@1\ntags:", 1),
    )
    .unwrap();
    let output = entry_check(&[&path]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert!(
        out.contains("refused: `series: ser_madeUp1@1` should be `<id>@<version>`"),
        "{out}"
    );
}
