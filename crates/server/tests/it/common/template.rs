//! Fresh in-memory databases loaded from one migrated template, so a test skips
//! running every migration: in a debug build that took 90 ms a database, and loading
//! the template takes 3 (#138).
//!
//! nextest runs each test as a process of its own, so the template is a file in
//! `CARGO_TARGET_TMPDIR`, named for this test binary's build: a new migration, a
//! change to `db::open` or an sqlx upgrade all rebuild the binary, and so the
//! template.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use filedrawer_server::db;

/// What every template's file name starts with.
const PREFIX: &str = "filedrawer-template-";

/// A fresh in-memory database, migrated, with nothing in it.
pub async fn fresh_pool() -> sqlx::SqlitePool {
    let path = template_path();
    if let Ok(image) = std::fs::read(&path)
        && let Ok(pool) = db::open_image(&image).await
    {
        return pool;
    }
    // Missing, or removed or damaged since: made again, and then it must load, so a
    // broken `open_image` fails the tests rather than costing them the migrations.
    let image = make_template(&path).await;
    db::open_image(&image)
        .await
        .expect("a template just made loads")
}

/// This build's template: the test binary's size and modification time.
pub fn template_path() -> PathBuf {
    let binary = std::env::current_exe().expect("a test binary knows its path");
    let metadata = std::fs::metadata(&binary).expect("the running test binary exists");
    let modified = metadata
        .modified()
        .expect("Linux records modification times")
        .duration_since(UNIX_EPOCH)
        .expect("the binary was built after 1970")
        .as_nanos();
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{PREFIX}{}-{modified}.db", metadata.len()))
}

/// Makes the template at `path` from a database `db::open` migrated, and returns it.
/// It's written to a file of its own name and renamed into place, so a test in
/// another process never reads half of one. Older builds' templates are removed.
async fn make_template(path: &Path) -> Vec<u8> {
    let pool = db::open(":memory:").await.unwrap();
    let image = pool
        .acquire()
        .await
        .unwrap()
        .serialize(None)
        .await
        .unwrap()
        .to_vec();
    pool.close().await;
    let dir = path.parent().expect("the template's path is in a folder");
    remove_other_templates(dir, path);
    let mut file = tempfile::NamedTempFile::new_in(dir).unwrap();
    file.write_all(&image).unwrap();
    file.persist(path).unwrap();
    image
}

/// Removes every template in `dir` but `keep`, best effort: one another process is
/// reading stays readable until it closes it, and one it then finds gone it makes again.
fn remove_other_templates(dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_template = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(PREFIX) && name.ends_with(".db"));
        if is_template && path != keep {
            let _ = std::fs::remove_file(path);
        }
    }
}
