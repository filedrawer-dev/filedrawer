//! Checks File Drawer entries before submission, offline, with the analyzer the server
//! uses: `filedrawer check` and `entry-check`, one code under two names.

use std::ffi::OsString;
use std::process::ExitCode;

use filedrawer_analyzer::{Analysis, CardContext, analyze, render_card};

/// What `-h` prints after the usage line.
const ABOUT: &str = "\
Reads each entry the way the File Drawer server does (docs/entry-format.md) and prints
whether the server would accept it, the reasons it would refuse it, its card as
the analyzer renders it, the primary Results row as the analyzer read it, and every
warning. Nothing is fetched: link hosts and pins are checked by the URL's shape, and
the card shows placeholders such as <id> and <author> for what only the server knows.

Exit status: 0 when every entry would be accepted (warnings allowed), 1 when any would
be refused, 2 when a file can't be read or no file is given";

/// What `-h` adds to the exit statuses in a debug build, which checks its own age
/// (`stale`).
#[cfg(debug_assertions)]
const DEBUG_EXIT: &str = ", 3 when this debug build is
older than the sources it was built from, which a rebuild fixes";
#[cfg(not(debug_assertions))]
const DEBUG_EXIT: &str = "";

/// Checks the entries at `paths`, printing as `command` (`entry-check` or
/// `filedrawer check`), and returns the exit status `-h` describes.
pub fn run(command: &str, paths: &[OsString]) -> ExitCode {
    let usage = format!("usage: {command} FILE...\n\n{ABOUT}{DEBUG_EXIT}.");
    if paths.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{usage}");
        return ExitCode::SUCCESS;
    }
    #[cfg(debug_assertions)]
    if let Some(refusal) = stale(command) {
        eprintln!("{refusal}");
        return ExitCode::from(3);
    }
    if paths.is_empty() {
        eprintln!("{usage}");
        return ExitCode::from(2);
    }
    let mut refused = false;
    let mut unreadable = false;
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            println!();
        }
        let name = path.to_string_lossy();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let analysis = analyze(&text);
                refused |= analysis.refused();
                print!("{}", report(&name, &analysis));
            }
            Err(error) => {
                eprintln!("{command}: can't read {name}: {error}");
                unreadable = true;
            }
        }
    }
    match (unreadable, refused) {
        (true, _) => ExitCode::from(2),
        (false, true) => ExitCode::from(1),
        (false, false) => ExitCode::SUCCESS,
    }
}

/// Why this debug build mustn't check entries, when the sources it was built from have
/// changed since: an older analyzer would refuse or accept entries the current one
/// doesn't (in one session, 111 of 428 entries read as refused that weren't).
/// FILEDRAWER_SOURCE_ROOT, for the tests, names the sources to compare with instead.
#[cfg(debug_assertions)]
fn stale(command: &str) -> Option<String> {
    stale_against(
        command,
        include_str!(concat!(env!("OUT_DIR"), "/source-stamp")),
        std::env::var_os("FILEDRAWER_SOURCE_ROOT"),
        std::env::current_exe().ok().as_deref(),
    )
}

/// `stale`, given the build's stamp (the sources' hash, then the folder they were in;
/// empty outside the repository), FILEDRAWER_SOURCE_ROOT's value and the binary's path.
#[cfg(debug_assertions)]
fn stale_against(
    command: &str,
    stamp: &str,
    given: Option<OsString>,
    exe: Option<&std::path::Path>,
) -> Option<String> {
    use crate::source_stamp::{has_sources, hash};
    use std::path::{Path, PathBuf};

    let (built, root) = stamp.trim_end().split_once('\n')?;
    let root = match given {
        // The tests' copy: with no sources there, refuse, so it can't turn the check off.
        Some(given) => PathBuf::from(given),
        None if has_sources(Path::new(root)) => PathBuf::from(root),
        None => root_beside(exe?)?,
    };
    if !has_sources(&root) {
        return Some(format!(
            "{command}: FILEDRAWER_SOURCE_ROOT names {}, which holds none of its sources; \
             unset it.",
            root.display()
        ));
    }
    if hash(&root) == built {
        return None;
    }
    Some(format!(
        "{command}: this debug build is older than the sources in {}; rebuild it with \
         `cargo build -p filedrawer` (when that changes nothing, `touch crates/cli/build.rs` \
         first), or run `cargo run -q --bin entry-check -- FILE...`, which rebuilds first.",
        root.display()
    ))
}

/// For a stamp naming a tree that's gone (it moved, or another machine built it at
/// another path): the sources beside the binary at `exe`, when it sits in their
/// target/debug/.
#[cfg(debug_assertions)]
fn root_beside(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let debug = exe.parent()?;
    let target = debug.parent()?;
    let beside = target.parent()?;
    let in_place = debug.file_name()? == "debug" && target.file_name()? == "target";
    (in_place && crate::source_stamp::has_sources(beside)).then(|| beside.to_path_buf())
}

/// The report for one entry: verdict, refusals, card, primary row, warnings.
fn report(path: &str, analysis: &Analysis) -> String {
    let warnings = match analysis.warnings.len() {
        1 => "1 warning".to_string(),
        n => format!("{n} warnings"),
    };
    let verdict = if analysis.refused() {
        "refused"
    } else {
        "accepted"
    };
    let mut out = format!("{path}: {verdict}, {warnings}\n");
    for refusal in &analysis.refusals {
        out.push_str(&format!("refused: {refusal}\n"));
    }
    out.push_str("card:\n");
    for line in render_card(analysis, &CardContext::default()).lines() {
        out.push_str(&format!("  {line}\n"));
    }
    out.push_str(&format!(
        "primary row: {}\n",
        analysis.primary_row_summary()
    ));
    for warning in &analysis.warnings {
        out.push_str(&format!("warning: {warning}\n"));
    }
    out
}

/// Which sources a debug build compares itself with when FILEDRAWER_SOURCE_ROOT is
/// unset. The stamp and the binary's path come in as arguments here: the process tests
/// (tests/it/stale.rs) can't remove the tree their binaries were stamped with.
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{root_beside, stale_against};
    use crate::source_stamp::{FOLDERS, hash};
    use std::path::{Path, PathBuf};

    /// A tree holding the sources' folders, its one source file reading `text`.
    fn tree(text: &str) -> tempfile::TempDir {
        let tree = tempfile::tempdir().unwrap();
        for folder in FOLDERS {
            std::fs::create_dir_all(tree.path().join(folder)).unwrap();
        }
        std::fs::write(tree.path().join("crates/cli/src/main.rs"), text).unwrap();
        tree
    }

    /// The stamp of a build of the sources in `built`, made in `root`.
    fn stamp(built: &Path, root: &Path) -> String {
        format!("{}\n{}\n", hash(built), root.display())
    }

    fn exe_in(root: &Path) -> PathBuf {
        root.join("target/debug/entry-check")
    }

    #[test]
    fn root_beside_finds_the_sources_whose_target_debug_holds_the_binary() {
        let sources = tree("");
        assert_eq!(
            root_beside(&exe_in(sources.path())).as_deref(),
            Some(sources.path())
        );

        let bare = tempfile::tempdir().unwrap();
        assert_eq!(root_beside(&exe_in(bare.path())), None);
        let release = sources.path().join("target/release/entry-check");
        assert_eq!(root_beside(&release), None);
        let not_target = sources.path().join("build/debug/entry-check");
        assert_eq!(root_beside(&not_target), None);
    }

    #[test]
    fn a_stamped_tree_holding_the_sources_is_the_one_compared() {
        let stamped = tree("");
        let beside = tree("// edited\n");
        let stamp = stamp(stamped.path(), stamped.path());
        let exe = exe_in(beside.path());
        assert_eq!(stale_against("entry-check", &stamp, None, Some(&exe)), None);
    }

    #[test]
    fn with_the_stamped_tree_gone_the_sources_beside_the_binary_are_compared() {
        let gone = tempfile::tempdir().unwrap().path().join("moved");
        let built = tree("");
        let beside = tree("// edited\n");
        let exe = exe_in(beside.path());
        let refusal = stale_against("entry-check", &stamp(built.path(), &gone), None, Some(&exe))
            .expect("a build older than the sources beside it refuses");
        assert!(
            refusal.contains(&format!(
                "older than the sources in {}",
                beside.path().display()
            )),
            "{refusal}"
        );

        let current = tree("");
        let exe = exe_in(current.path());
        let stamp = stamp(built.path(), &gone);
        assert_eq!(stale_against("entry-check", &stamp, None, Some(&exe)), None);
    }

    #[test]
    fn a_build_with_no_stamp_or_no_sources_to_find_checks_entries() {
        // A published crate's build, whose stamp is empty.
        let beside = tree("");
        let exe = exe_in(beside.path());
        assert_eq!(stale_against("entry-check", "", None, Some(&exe)), None);
        // A stamped tree gone, and a binary that isn't in a tree's target/debug/.
        let gone = tempfile::tempdir().unwrap().path().join("moved");
        let stamp = stamp(beside.path(), &gone);
        let elsewhere = tempfile::tempdir().unwrap().path().join("entry-check");
        assert_eq!(
            stale_against("entry-check", &stamp, None, Some(&elsewhere)),
            None
        );
        assert_eq!(stale_against("entry-check", &stamp, None, None), None);
    }
}
