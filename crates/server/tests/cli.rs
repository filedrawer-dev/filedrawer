//! Seam: the `filedrawer` binary, run as a process.

mod common;

use std::path::PathBuf;
use std::process::{Command, Output};

use axum::http::StatusCode;
use filedrawer_server::{AppState, Screener, app, db};

fn scratch_database(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("filedrawer-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("filedrawer.db")
}

fn filedrawer(database: &PathBuf, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_filedrawer"))
        .args(args)
        .env_clear()
        .env("FILEDRAWER_DATABASE", database)
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

fn printed(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .trim()
        .to_string()
}

#[tokio::test]
async fn admin_commands_make_an_account_and_a_token_the_server_accepts() {
    let database = scratch_database("admin");
    let account = printed(&filedrawer(
        &database,
        &["admin", "create-account", "--display-name", "Ada"],
        &[],
    ));
    assert!(
        account.starts_with("a-") && account.len() == 12,
        "{account}"
    );
    let token = printed(&filedrawer(
        &database,
        &[
            "admin",
            "create-token",
            "--account",
            &account,
            "--name",
            "laptop",
        ],
        &[],
    ));
    assert!(token.starts_with("fdt_") && token.len() == 56, "{token}");

    let pool = db::open(database.to_str().unwrap()).await.unwrap();
    let router = app(AppState {
        pool,
        screener: Screener::PassAll,
    });
    let test = common::TestApp {
        router,
        account,
        token,
    };
    let reply = test.post("/entries", common::entry("T", "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[test]
fn a_token_for_an_unknown_account_is_refused() {
    let database = scratch_database("unknown");
    let output = filedrawer(
        &database,
        &[
            "admin",
            "create-token",
            "--account",
            "a-nobody",
            "--name",
            "x",
        ],
        &[],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no account a-nobody"));
}

#[test]
fn production_refuses_the_pass_all_screener() {
    let database = scratch_database("production");
    let mut child = Command::new(env!("CARGO_BIN_EXE_filedrawer"))
        .arg("serve")
        .env_clear()
        .env("FILEDRAWER_DATABASE", &database)
        .env("FILEDRAWER_LISTEN", "127.0.0.1:0")
        .env("FILEDRAWER_ENVIRONMENT", "production")
        .env("FILEDRAWER_SCREENER", "pass-all")
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // A server that wrongly starts never exits; give it 10 seconds, then fail.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            panic!("production started with the pass-all screener");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("production refuses it"), "{err}");
}

#[test]
fn serve_names_a_missing_setting() {
    let database = scratch_database("missing");
    let output = filedrawer(
        &database,
        &["serve"],
        &[("FILEDRAWER_ENVIRONMENT", "staging")],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("FILEDRAWER_LISTEN is not set"));
}
