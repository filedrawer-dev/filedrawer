#![forbid(unsafe_code)]
//! The server's integration tests.
//! Each file here is a module of this one binary, so `common` compiles once and
//! one test binary links, not one per file (#35, design.md, Stack).

mod accounts;
mod admin_removal;
mod admin_series;
mod admin_suspension;
mod allowance;
mod attachments;
mod batches;
mod cites;
mod cli;
mod common;
mod config;
mod daily_email;
mod deletion;
mod embargo;
mod embargo_paused;
mod entries_mine;
mod entry_deletion;
mod error_bodies;
mod fair_queue;
mod feedback;
mod hold;
mod http_fetcher;
mod invites;
mod limits;
mod link_checks;
mod migration_rollback;
mod migrations;
mod pages;
mod provenance;
mod read;
mod reader_warnings;
mod removed_files;
mod request_log;
mod review;
mod screening;
mod search_log;
mod series;
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
