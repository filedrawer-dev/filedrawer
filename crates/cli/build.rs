//! Writes the hash of the sources this package is built from, and the folder holding
//! them, to OUT_DIR/source-stamp, which a debug build compares with the sources at each
//! run (src/source_stamp.rs). Outside the repository, as a published crate, the stamp
//! is empty and the check is off.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

#[path = "src/source_stamp.rs"]
mod source_stamp;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    // The workspace root: crates/cli's parent's parent.
    let root = manifest.parent().and_then(Path::parent);
    let stamp = match root {
        Some(root) if source_stamp::has_sources(root) => {
            for path in source_stamp::FOLDERS.iter().chain(&source_stamp::FILES) {
                println!("cargo:rerun-if-changed={}", root.join(path).display());
            }
            format!("{}\n{}\n", source_stamp::hash(root), root.display())
        }
        _ => String::new(),
    };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets it"));
    std::fs::write(out.join("source-stamp"), stamp).expect("OUT_DIR is writable");
}
