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
