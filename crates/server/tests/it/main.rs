#![forbid(unsafe_code)]
//! The server's integration tests.
//! Each file here is a module of this one binary, so `common` compiles once and
//! one test binary links, not one per file (#35, design.md, Stack).

mod accounts;
mod admin_removal;
mod admin_series;
mod admin_suspension;
mod admin_unknown_fields;
mod agent_accounts;
mod allowance;
mod anthropic_client;
mod attachments;
mod batches;
mod body_limits;
mod cites;
mod claims;
mod cli;
mod common;
mod config;
mod daily_email;
mod db;
mod deletion;
mod duplicates;
mod email_change;
mod entries_mine;
mod entry_deletion;
mod error_bodies;
mod fair_queue;
mod feedback;
mod file_store;
mod github_oauth;
mod github_refresh;
mod github_sign_in;
mod hold;
mod http_fetcher;
mod invites;
mod limits;
mod link_checks;
mod mail_queue;
mod migration_rollback;
mod migrations;
mod pages;
mod passwords;
mod provenance;
mod read;
mod reader_warnings;
mod reads;
mod rejected_expiry;
mod removed_files;
mod request_log;
mod review;
mod screening;
mod search_log;
mod search_places;
mod search_terms;
mod series;
mod serve;
mod skill;
mod skill_folders;
mod smoke;
mod status;
mod submit;
mod usability;
mod versions;
mod withdrawal;

/// Cargo builds only the files `main.rs` reaches, so a test file here without a `mod`
/// line above would never run.
#[test]
fn every_file_here_is_a_module() {
    // From the working directory, which cargo and nextest set to the crate's root:
    // CARGO_MANIFEST_DIR names the tree the binary was built in, which may be gone.
    let main = std::fs::read_to_string("tests/it/main.rs").unwrap();
    let mut missing = Vec::new();
    for entry in std::fs::read_dir("tests/it").unwrap() {
        let path = entry.unwrap().path();
        let module = if path.join("mod.rs").is_file() {
            path.file_name()
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            path.file_stem()
        } else {
            None
        };
        let Some(module) = module.and_then(|name| name.to_str()) else {
            continue;
        };
        let line = format!("mod {module};");
        if module != "main" && !main.lines().any(|l| l == line) {
            missing.push(module.to_string());
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "tests/it/main.rs has no `mod` line for {missing:?}"
    );
}

/// A test that reads the repo's files finds them through `common::crate_path`, the tree
/// it runs in. `CARGO_MANIFEST_DIR` read at compile time names the tree it was built
/// in: the commit check builds a copy of what's staged into the worktree's own target/,
/// so a later run in the worktree can read that copy and pass on files it never saw.
#[test]
fn no_test_finds_files_through_the_tree_it_was_built_in() {
    // Built with concat!, so this file doesn't match itself.
    let pattern = concat!("env!(\"CARGO_", "MANIFEST_DIR\")");
    // It copies the sources the binary was built from, on purpose.
    let allowed = ["../cli/tests/it/stale.rs"];
    let mut found = Vec::new();
    let mut dirs = vec![std::path::PathBuf::from("..")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let in_tests = path.components().any(|part| part.as_os_str() == "tests");
            if path.is_dir() {
                dirs.push(path);
            } else if in_tests
                && path.extension().is_some_and(|ext| ext == "rs")
                && !allowed.iter().any(|a| path == std::path::Path::new(a))
                && std::fs::read_to_string(&path).unwrap().contains(pattern)
            {
                found.push(path.display().to_string());
            }
        }
    }
    found.sort();
    assert!(
        found.is_empty(),
        "these read files through the tree they were built in; use common::crate_path: {found:?}"
    );
}
