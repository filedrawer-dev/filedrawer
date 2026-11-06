//! Seam: the `filedrawer` binary, run as a process.

mod common;

use std::path::PathBuf;
use std::process::{Command, Output};

use axum::http::StatusCode;
use filedrawer_server::{app, db};

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
    let outbox = common::Outbox::default();
    let files_dir = common::files_dir();
    let state = common::state(pool.clone(), &outbox, files_dir.path());
    let router = app(state.clone());
    let test = common::TestApp {
        router,
        state,
        pool,
        account,
        token,
        outbox,
        files_dir,
    };
    let reply = test.post("/entries", common::entry("T", "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[tokio::test]
async fn admin_grant_lets_the_accounts_browser_session_reach_the_admin_pages() {
    let database = scratch_database("grant");
    let pool = db::open(database.to_str().unwrap()).await.unwrap();
    let outbox = common::Outbox::default();
    let files_dir = common::files_dir();
    let state = common::state(pool.clone(), &outbox, files_dir.path());
    let test = common::TestApp {
        router: app(state.clone()),
        state,
        pool,
        account: String::new(),
        token: String::new(),
        outbox,
        files_dir,
    };
    let (cookie, _) = common::browser_account(&test, "owner@example.com").await;
    assert_eq!(
        test.page("/admin", Some(&cookie)).await.status,
        StatusCode::NOT_FOUND
    );
    let page = test.page("/account", Some(&cookie)).await;
    let start = page.body.find("<code>a-").unwrap() + 6;
    let account = &page.body[start..start + 12];
    assert_eq!(
        printed(&filedrawer(
            &database,
            &["admin", "grant", "--account", account],
            &[]
        )),
        account
    );
    assert_eq!(
        test.page("/admin", Some(&cookie)).await.status,
        StatusCode::OK
    );
    let unknown = filedrawer(&database, &["admin", "grant", "--account", "a-nobody"], &[]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("no account a-nobody"));
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
        .env("FILEDRAWER_PUBLIC_URL", "https://filedrawer.dev")
        .env("FILEDRAWER_SMTP_HOST", "smtp.example.com")
        .env("FILEDRAWER_SMTP_USER", "user")
        .env("FILEDRAWER_SMTP_PASSWORD", "not-a-real-password")
        .env(
            "FILEDRAWER_MAIL_FROM",
            "File Drawer <noreply@filedrawer.dev>",
        )
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

#[test]
fn production_without_smtp_settings_refuses_to_start() {
    let database = scratch_database("no-smtp");
    let output = filedrawer(
        &database,
        &["serve"],
        &[
            ("FILEDRAWER_LISTEN", "127.0.0.1:0"),
            ("FILEDRAWER_PUBLIC_URL", "https://filedrawer.dev"),
            ("FILEDRAWER_ENVIRONMENT", "production"),
            ("FILEDRAWER_SCREENER", "pass-all"),
        ],
    );
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("FILEDRAWER_SMTP_HOST"), "{err}");
}
