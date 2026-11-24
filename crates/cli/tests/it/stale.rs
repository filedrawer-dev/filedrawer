//! Seam: the `entry-check` and `filedrawer` binaries, run as processes against a copy
//! of the sources they were built from, named by FILEDRAWER_SOURCE_ROOT: a debug build
//! older than its sources refuses to check entries (design.md, Distribution).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::common::{fixture, stderr};

/// The sources the binaries are built from, relative to the repository's root.
const SOURCES: [&str; 6] = [
    "Cargo.lock",
    "crates/analyzer/Cargo.toml",
    "crates/analyzer/src",
    "crates/cli/Cargo.toml",
    "crates/cli/build.rs",
    "crates/cli/src",
];

/// A copy of the sources in a scratch folder, which goes when this drops. They come
/// from the tree the tests were built in, which a run right after its build still has.
fn copy_of_sources() -> tempfile::TempDir {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(Path::parent).unwrap();
    let copy = tempfile::Builder::new()
        .prefix("cli-sources-")
        .tempdir()
        .unwrap();
    for source in SOURCES {
        copy_path(&root.join(source), &copy.path().join(source));
    }
    copy
}

fn copy_path(from: &Path, to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    if from.is_dir() {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            copy_path(&entry.path(), &to.join(entry.file_name()));
        }
    } else {
        std::fs::copy(from, to).unwrap();
    }
}

fn run_against<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(
    binary: &str,
    root: &Path,
    args: I,
) -> Output {
    Command::new(binary)
        .args(args)
        .env("FILEDRAWER_SOURCE_ROOT", root)
        .output()
        .unwrap()
}

fn entry_check_against(root: &Path) -> Output {
    run_against(
        env!("CARGO_BIN_EXE_entry-check"),
        root,
        [&fixture("example.md")],
    )
}

fn append(path: PathBuf, text: &str) {
    let mut contents = std::fs::read_to_string(&path).unwrap();
    contents.push_str(text);
    std::fs::write(path, contents).unwrap();
}

#[test]
fn a_build_of_the_sources_as_they_are_checks_entries() {
    let sources = copy_of_sources();
    let output = entry_check_against(sources.path());
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn an_edited_source_makes_the_build_refuse_naming_the_rebuild() {
    let sources = copy_of_sources();
    append(
        sources.path().join("crates/analyzer/src/lib.rs"),
        "\n// edited\n",
    );
    let output = entry_check_against(sources.path());
    assert_eq!(output.status.code(), Some(3));
    let err = stderr(&output);
    assert!(err.starts_with("entry-check: "), "{err}");
    assert!(
        err.contains(&format!(
            "older than the sources in {}",
            sources.path().display()
        )),
        "{err}"
    );
    assert!(err.contains("cargo build -p filedrawer"), "{err}");
    assert!(err.contains("touch crates/cli/build.rs"), "{err}");
    assert_eq!(err.lines().count(), 1, "{err}");
}

#[test]
fn an_added_renamed_or_lock_changed_source_makes_the_build_refuse() {
    let added = copy_of_sources();
    std::fs::write(added.path().join("crates/cli/src/new.rs"), "// new\n").unwrap();
    assert_eq!(entry_check_against(added.path()).status.code(), Some(3));

    let renamed = copy_of_sources();
    let src = renamed.path().join("crates/analyzer/src");
    std::fs::rename(src.join("card.rs"), src.join("card2.rs")).unwrap();
    assert_eq!(entry_check_against(renamed.path()).status.code(), Some(3));

    let locked = copy_of_sources();
    append(locked.path().join("Cargo.lock"), "\n");
    assert_eq!(entry_check_against(locked.path()).status.code(), Some(3));
}

#[test]
fn files_cargo_doesnt_compile_dont_count() {
    let sources = copy_of_sources();
    let src = sources.path().join("crates/analyzer/src");
    std::fs::write(src.join(".card.rs.swp"), "swap").unwrap();
    std::fs::write(src.join("notes.txt"), "notes").unwrap();
    std::os::unix::fs::symlink(src.join("card.rs"), src.join("linked.rs")).unwrap();
    let output = entry_check_against(sources.path());
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn a_root_without_the_sources_refuses_rather_than_skipping_the_check() {
    let empty = tempfile::tempdir().unwrap();
    let output = entry_check_against(empty.path());
    assert_eq!(output.status.code(), Some(3));
    let err = stderr(&output);
    assert!(
        err.contains(&format!(
            "{}, which holds none of its sources",
            empty.path().display()
        )),
        "{err}"
    );
}

#[test]
fn filedrawer_check_refuses_under_its_own_name_and_its_help_still_answers() {
    let sources = copy_of_sources();
    append(
        sources.path().join("crates/cli/src/check.rs"),
        "\n// edited\n",
    );
    let binary = env!("CARGO_BIN_EXE_filedrawer");
    let check = run_against(
        binary,
        sources.path(),
        [OsStr::new("check"), fixture("example.md").as_ref()],
    );
    assert_eq!(check.status.code(), Some(3));
    assert!(
        stderr(&check).starts_with("filedrawer check: "),
        "{}",
        stderr(&check)
    );
    let help = run_against(binary, sources.path(), ["-h"]);
    assert_eq!(help.status.code(), Some(0));
    let check_help = run_against(binary, sources.path(), ["check", "-h"]);
    assert_eq!(check_help.status.code(), Some(0));
}

#[test]
fn help_names_the_stale_build_exit_status() {
    let sources = copy_of_sources();
    let output = run_against(env!("CARGO_BIN_EXE_entry-check"), sources.path(), ["-h"]);
    let out = String::from_utf8(output.stdout).unwrap();
    let words = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        words.contains("no file is given, 3 when this debug build is older"),
        "{out}"
    );
}
