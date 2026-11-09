//! Seam: the `filedrawer` binary, run as a process.

use crate::common;

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
        web: Default::default(),
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
        web: Default::default(),
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

/// Made-up hidden text, 48 bytes, and its base64 (made with `base64 -w0`).
const HIDDEN: &str = "made-up instruction for agents reading this file";
const HIDDEN_B64: &str = "bWFkZS11cCBpbnN0cnVjdGlvbiBmb3IgYWdlbnRzIHJlYWRpbmcgdGhpcyBmaWxl";
/// HIDDEN in base64 four times over, one layer past the limit (`base64 -w0`, four times).
const HIDDEN_B64_X4: &str = "V1d4a1IyRXhjRlJOVkVacVVUQktkMWx0TlU5TlIwNTFWbTF3YTFJeWVESlpiV3hEWWxkSmVsTlhaRnBXTWxKeldXMDFVMlZyYkVsVGJYaGFWakZLZDFsdE1XcGFNbEpJWVVoQ2FtVlZTblJaVm1RMFlrRTlQUT09";

/// Runs `filedrawer unpack` on files holding `texts`, written to a scratch folder,
/// and returns the output and each file's path.
fn unpack(name: &str, texts: &[&[u8]]) -> (Output, Vec<String>) {
    let dir = scratch_database(name).parent().unwrap().to_path_buf();
    let paths: Vec<String> = texts
        .iter()
        .enumerate()
        .map(|(n, text)| {
            let path = dir.join(format!("file{n}.py"));
            std::fs::write(&path, text).unwrap();
            path.display().to_string()
        })
        .collect();
    let output = Command::new(env!("CARGO_BIN_EXE_filedrawer"))
        .arg("unpack")
        .args(&paths)
        .env_clear()
        .output()
        .unwrap();
    (output, paths)
}

#[test]
fn unpack_counts_what_each_file_hides_without_printing_it() {
    let hiding = format!("payload = '{HIDDEN_B64}'\n");
    let (output, paths) = unpack(
        "unpack",
        &[hiding.as_bytes(), b"def plain():\n    return 1\n"],
    );
    let out = printed(&output);
    assert_eq!(
        out,
        format!(
            "{}: 1 part, {} bytes hidden\n{}: nothing hidden",
            paths[0],
            HIDDEN.len(),
            paths[1]
        )
    );
    assert!(!out.contains("made-up"), "{out}");
    assert!(!out.contains(&HIDDEN_B64[..12]), "{out}");
}

#[test]
fn unpack_names_the_limit_a_file_passes() {
    let deep = format!("payload = '{HIDDEN_B64_X4}'\n");
    let (output, paths) = unpack("unpack-deep", &[deep.as_bytes()]);
    assert_eq!(
        printed(&output),
        format!(
            "{}: past a limit: it is encoded or compressed more than 3 layers deep",
            paths[0]
        )
    );
}

#[test]
fn unpack_refuses_a_file_that_isnt_utf8() {
    let (output, paths) = unpack("unpack-binary", &[b"ok\n", &[0xff, 0xfe, 0x00]]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&paths[1]), "{stderr}");
    assert!(output.stdout.is_empty());
}

/// A zip in base64 holding one member, `SECRET-MEMBER.txt`, compressed with LZMA, which
/// the server doesn't open (made with Python's zipfile).
const LZMA_ZIP_B64: &str = "UEsDBD8AAgAOAGg6Z138mvKQMwAAACAAAAARAAAAU0VDUkVULU1FTUJFUi50eHQJBAUAXQAAgAAANphIvkw7tP5V27opYNUFzHI/jpWng0iRp2iwavIjMgRrdJn///ffDABQSwECPwM/AAIADgBoOmdd/JrykDMAAAAgAAAAEQAAAAAAAAAAAAAAgAEAAAAAU0VDUkVULU1FTUJFUi50eHRQSwUGAAAAAAEAAQA/AAAAYgAAAAAA";

#[test]
fn unpack_names_nothing_from_inside_what_it_wont_open() {
    let file = format!("payload = '{LZMA_ZIP_B64}'\n");
    let (output, paths) = unpack("unpack-unopened", &[file.as_bytes()]);
    assert_eq!(
        printed(&output),
        format!(
            "{}: past a limit: it holds something the server doesn't open",
            paths[0]
        )
    );
}
