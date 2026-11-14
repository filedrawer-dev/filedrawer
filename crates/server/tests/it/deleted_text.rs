//! Seam: the router on a database file, and `db::trim_log`: once an entry is
//! deleted and the log emptied, its text is in none of the database's files
//! (design.md, Admin; #65).

use crate::common;

use std::path::Path;

use axum::http::StatusCode;
use common::{TestApp, entry};
use filedrawer_server::{admin, app, db};

/// A word no other row holds.
const MARKER: &str = "zqvjxmarkerwkq";

/// A test app whose database is the file `filedrawer.db` in `dir`.
async fn app_on_disk(dir: &Path) -> TestApp {
    let pool = db::open(dir.join("filedrawer.db").to_str().unwrap())
        .await
        .unwrap();
    let account = admin::create_account(&pool, "Ada").await.unwrap();
    let token = admin::create_token(&pool, &account, "laptop")
        .await
        .unwrap();
    let outbox = common::Outbox::default();
    let files_dir = common::files_dir();
    let state = common::state(pool.clone(), &outbox, files_dir.path());
    TestApp {
        router: app(state.clone()),
        state,
        pool,
        account,
        token,
        outbox,
        files_dir,
        web: Default::default(),
    }
}

/// The files in `dir` that hold `text`: the database, its log and its index.
fn files_holding(dir: &Path, text: &str) -> Vec<String> {
    let mut holding = Vec::new();
    for file in std::fs::read_dir(dir).unwrap() {
        let file = file.unwrap();
        let bytes = std::fs::read(file.path()).unwrap();
        if bytes
            .windows(text.len())
            .any(|window| window == text.as_bytes())
        {
            holding.push(file.file_name().to_string_lossy().into_owned());
        }
    }
    holding.sort();
    holding
}

#[tokio::test]
async fn a_deleted_entrys_text_is_in_none_of_the_databases_files() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_on_disk(dir.path()).await;
    let id = app
        .submit_held(&entry(&format!("Steering {MARKER}"), ""))
        .await;
    assert_ne!(files_holding(dir.path(), MARKER), Vec::<String>::new());

    let deleted = app.delete_as(&format!("/e/{id}"), &app.token, false).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert!(db::trim_log(&app.pool).await.unwrap());

    assert_eq!(files_holding(dir.path(), MARKER), Vec::<String>::new());
}
