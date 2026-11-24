//! The sources the `filedrawer` package's binaries are built from, hashed, so a debug
//! build older than them refuses to check entries (design.md, Distribution).
//! `build.rs` hashes them into the build, and `check.rs` hashes them again at each run,
//! or, with FILEDRAWER_SOURCE_ROOT set (the tests), hashes the sources in that folder.

use std::path::Path;

/// The folders whose `.rs` files the binaries are built from, relative to the root.
pub const FOLDERS: [&str; 2] = ["crates/analyzer/src", "crates/cli/src"];
/// The other files they are built from, relative to the root.
pub const FILES: [&str; 4] = [
    "Cargo.lock",
    "crates/analyzer/Cargo.toml",
    "crates/cli/Cargo.toml",
    "crates/cli/build.rs",
];

/// Whether `root` holds the sources, as the repository does and a published crate
/// doesn't.
pub fn has_sources(root: &Path) -> bool {
    FOLDERS.iter().all(|folder| root.join(folder).is_dir())
}

/// The sources under `root`, hashed as 16 hex digits: each regular `.rs` file in the
/// folders and each of the files, by its path relative to the root and its bytes, in
/// path order. Links are never followed, and a file that can't be read hashes as a
/// fixed marker, so a broken tree gives a hash, never a panic.
pub fn hash(root: &Path) -> String {
    let mut paths: Vec<String> = FILES.iter().map(|file| (*file).to_string()).collect();
    for folder in FOLDERS {
        collect(root, folder, &mut paths);
    }
    paths.sort();
    let mut state = Fnv::new();
    for path in &paths {
        state.feed(path.as_bytes());
        state.feed(&[0]);
        match std::fs::read(root.join(path)) {
            Ok(bytes) => {
                state.feed(&bytes.len().to_le_bytes());
                state.feed(&bytes);
            }
            Err(_) => state.feed(b"\xffunreadable"),
        }
    }
    format!("{:016x}", state.0)
}

/// Adds the regular `.rs` files under `root/relative`, by their paths relative to root.
fn collect(root: &Path, relative: &str, paths: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(root.join(relative)) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let path = format!("{relative}/{name}");
        if kind.is_dir() {
            collect(root, &path, paths);
        } else if kind.is_file() && name.ends_with(".rs") {
            paths.push(path);
        }
    }
}

/// FNV-1a, 64 bits: the standard library's hashers may change between releases.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn feed(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}
