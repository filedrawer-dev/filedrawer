#![forbid(unsafe_code)]
//! The analyzer's integration tests.
//! Each file here is a module of this one binary, so `common` compiles once and
//! one test binary links, not one per file (#35, design.md, Stack).

mod card;
mod cli;
mod common;
mod encoded;
mod front_matter;
mod results;

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
