//! Seam: the `filedrawer-server` binary, run as a process.

use crate::common;
use crate::common::scratch::Scratch;

use std::path::Path;
use std::process::{Command, Output};

use axum::http::StatusCode;
use filedrawer_server::{app, db};

fn scratch_database(name: &str) -> Scratch {
    Scratch::database(&format!("cli-{name}"))
}

fn server(database: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_filedrawer-server"))
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
    let account = printed(&server(
        &database,
        &["admin", "create-account", "--display-name", "Ada"],
        &[],
    ));
    assert!(
        account.starts_with("a-") && account.len() == 12,
        "{account}"
    );
    let token = printed(&server(
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
    let account = common::account_id(&test, &cookie).await;
    assert_eq!(
        printed(&server(
            &database,
            &["admin", "grant", "--account", &account],
            &[]
        )),
        account
    );
    assert_eq!(
        test.page("/admin", Some(&cookie)).await.status,
        StatusCode::OK
    );
    let unknown = server(&database, &["admin", "grant", "--account", "a-nobody"], &[]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("no account a-nobody"));
}

#[test]
fn a_token_for_an_unknown_account_is_refused() {
    let database = scratch_database("unknown");
    let output = server(
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_filedrawer-server"))
        .arg("serve")
        .env_clear()
        .env("FILEDRAWER_DATABASE", &*database)
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
    let output = server(
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
    let output = server(
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

/// Settings for `check-outbound` that call none of the internet's hosts.
const OFFLINE: &[(&str, &str)] = &[
    ("FILEDRAWER_LISTEN", "127.0.0.1:0"),
    ("FILEDRAWER_PUBLIC_URL", "https://staging.example"),
    ("FILEDRAWER_ENVIRONMENT", "staging"),
    ("FILEDRAWER_SCREENER", "pass-all"),
    ("FILEDRAWER_LINK_CHECKS", "off"),
];

#[test]
fn check_outbound_with_nothing_to_call_says_so_and_never_opens_the_database() {
    let database = scratch_database("outbound-none");
    std::fs::remove_dir_all(database.dir()).unwrap();
    let output = server(&database, &["check-outbound"], OFFLINE);
    let out = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "skipped SMTP (no SMTP settings)",
            "skipped api.anthropic.com (FILEDRAWER_SCREENER isn't anthropic)",
            "skipped api.github.com (FILEDRAWER_LINK_CHECKS=off)",
            "skipped hc-ping.com (no FILEDRAWER_DAILY_EMAIL_PING)",
        ]
    );
    assert!(!database.dir().exists());
}

#[test]
fn check_outbound_names_the_host_it_cant_reach_and_exits_3() {
    let database = scratch_database("outbound-smtp");
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let mut env = OFFLINE.to_vec();
    env.extend([
        ("FILEDRAWER_SMTP_HOST", "127.0.0.1"),
        ("FILEDRAWER_SMTP_PORT", closed.as_str()),
        ("FILEDRAWER_SMTP_USER", "u"),
        ("FILEDRAWER_SMTP_PASSWORD", "hunter2"),
        ("FILEDRAWER_MAIL_FROM", "x@example.com"),
    ]);
    let output = server(&database, &["check-outbound"], &env);
    let out = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(3), "{out}");
    let first = out.lines().next().unwrap();
    assert!(
        first.starts_with(&format!("can't reach 127.0.0.1:{closed}: ")),
        "{out}"
    );
    assert!(!out.contains("hunter2"), "{out}");
}

#[test]
fn check_outbound_with_settings_it_cant_read_exits_2() {
    let database = scratch_database("outbound-bad");
    let output = server(
        &database,
        &["check-outbound"],
        &[("FILEDRAWER_ENVIRONMENT", "staging")],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("FILEDRAWER_LISTEN is not set"));
}

/// Made-up hidden text, 48 bytes, and its base64 (made with `base64 -w0`).
const HIDDEN: &str = "made-up instruction for agents reading this file";
const HIDDEN_B64: &str = "bWFkZS11cCBpbnN0cnVjdGlvbiBmb3IgYWdlbnRzIHJlYWRpbmcgdGhpcyBmaWxl";
/// HIDDEN in base64 four times over, one layer past the limit (`base64 -w0`, four times).
const HIDDEN_B64_X4: &str = "V1d4a1IyRXhjRlJOVkVacVVUQktkMWx0TlU5TlIwNTFWbTF3YTFJeWVESlpiV3hEWWxkSmVsTlhaRnBXTWxKeldXMDFVMlZyYkVsVGJYaGFWakZLZDFsdE1XcGFNbEpJWVVoQ2FtVlZTblJaVm1RMFlrRTlQUT09";

/// Runs `filedrawer-server unpack` on files holding `texts`, written to a scratch folder,
/// and returns the output and each file's path.
fn unpack(name: &str, texts: &[&[u8]]) -> (Output, Vec<String>) {
    let scratch = scratch_database(name);
    let dir = scratch.dir();
    let paths: Vec<String> = texts
        .iter()
        .enumerate()
        .map(|(n, text)| {
            let path = dir.join(format!("file{n}.py"));
            std::fs::write(&path, text).unwrap();
            path.display().to_string()
        })
        .collect();
    let output = Command::new(env!("CARGO_BIN_EXE_filedrawer-server"))
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

/// Every whole `FILEDRAWER_*` name in `text`.
fn setting_names(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .filter(|word| word.starts_with("FILEDRAWER_") && word.len() > "FILEDRAWER_".len())
        .collect()
}

/// The settings module's source, which names every setting `serve` reads.
const CONFIG_SOURCE: &str = include_str!("../../src/config.rs");

#[test]
fn help_names_every_setting_serve_reads() {
    let output = Command::new(env!("CARGO_BIN_EXE_filedrawer-server"))
        .arg("-h")
        .env_clear()
        .output()
        .unwrap();
    let help = printed(&output);
    let named = setting_names(&help);
    let mut missing: Vec<&str> = setting_names(CONFIG_SOURCE)
        .into_iter()
        .filter(|name| !named.contains(name))
        .collect();
    missing.sort_unstable();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "`filedrawer-server -h` doesn't name {missing:?}, which config.rs reads"
    );
}

#[test]
fn help_names_the_commit_the_binary_was_built_from() {
    let output = Command::new(env!("CARGO_BIN_EXE_filedrawer-server"))
        .arg("-h")
        .env_clear()
        .output()
        .unwrap();
    let help = printed(&output);
    // ops/deploy-production finds a binary's commit in this form, in the file itself.
    // What the build was given (CI's FILEDRAWER_COMMIT), read here rather than
    // through the binary's constant.
    let built_from = option_env!("FILEDRAWER_COMMIT")
        .filter(|commit| !commit.is_empty())
        .unwrap_or("unknown");
    let line = format!("filedrawer-commit {built_from}");
    assert!(
        help.lines()
            .any(|l| l.trim_end_matches('.').ends_with(&line)),
        "-h doesn't end a line with {line:?}:\n{help}"
    );
}

/// A router over the scratch database `database`, as the server would serve it.
async fn app_over(database: &Path) -> common::TestApp {
    let pool = db::open(database.to_str().unwrap()).await.unwrap();
    let outbox = common::Outbox::default();
    let files_dir = common::files_dir();
    let state = common::state(pool.clone(), &outbox, files_dir.path());
    common::TestApp {
        router: app(state.clone()),
        state,
        pool,
        account: String::new(),
        token: String::new(),
        outbox,
        files_dir,
        web: Default::default(),
    }
}

/// Grace's account with a session, a token and a reset link she hasn't used: what a
/// leak could hand someone.
async fn graces_secrets(test: &common::TestApp) -> (String, String, String) {
    let (cookie, token) = common::browser_account(test, "grace@example.com").await;
    test.form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let link = test.link_in_last_email_to("grace@example.com").await;
    (cookie, token, link)
}

/// Whether each of Grace's secrets still works: the session, the token, the link.
async fn still_work(test: &common::TestApp, secrets: &(String, String, String)) -> [bool; 3] {
    let (cookie, token, link) = secrets;
    [
        test.page("/account", Some(cookie)).await.status == StatusCode::OK,
        test.get_with_token("/entries/mine", token).await.status == StatusCode::OK,
        test.page(link, None).await.status == StatusCode::OK,
    ]
}

async fn logs_in(test: &common::TestApp, password: &str) -> common::Reply {
    test.form(
        "/login",
        &[("email", "grace@example.com"), ("password", password)],
        None,
    )
    .await
}

#[tokio::test]
async fn revoke_all_without_yes_says_what_it_would_end_and_changes_nothing() {
    let database = scratch_database("revoke-dry");
    let test = app_over(&database).await;
    let secrets = graces_secrets(&test).await;
    let said = printed(&server(&database, &["admin", "revoke-all"], &[]));
    for count in ["1 session", "1 token", "1 email link", "--yes"] {
        assert!(said.contains(count), "{said}");
    }
    assert!(!said.contains("password"), "{said}");
    assert_eq!(still_work(&test, &secrets).await, [true; 3]);
}

#[tokio::test]
async fn revoke_all_ends_every_session_token_and_email_link_and_keeps_passwords() {
    let database = scratch_database("revoke");
    let test = app_over(&database).await;
    let secrets = graces_secrets(&test).await;
    let said = printed(&server(&database, &["admin", "revoke-all", "--yes"], &[]));
    assert!(said.contains("1 session"), "{said}");
    assert_eq!(still_work(&test, &secrets).await, [false; 3]);
    assert_eq!(
        logs_in(&test, common::PASSWORD).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn revoke_all_passwords_leaves_each_account_a_reset_and_nothing_else() {
    let database = scratch_database("revoke-passwords");
    let test = app_over(&database).await;
    let secrets = graces_secrets(&test).await;
    let said = printed(&server(
        &database,
        &["admin", "revoke-all", "--passwords", "--yes"],
        &[],
    ));
    assert!(said.contains("1 password"), "{said}");
    assert_eq!(still_work(&test, &secrets).await, [false; 3]);
    // The old password gets login's usual answer, as a wrong one does.
    let old = logs_in(&test, common::PASSWORD).await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    assert!(
        old.body.contains("The email or the password is wrong."),
        "{}",
        old.body
    );
    test.form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let token = test
        .link_in_last_email_to("grace@example.com")
        .await
        .replace("/reset/confirm?token=", "");
    let reset = test
        .form(
            "/reset/confirm",
            &[("token", &token), ("password", "a new passphrase")],
            None,
        )
        .await;
    assert_eq!(reset.status, StatusCode::OK, "{}", reset.body);
    assert_eq!(
        logs_in(&test, "a new passphrase").await.status,
        StatusCode::SEE_OTHER
    );
}

#[test]
fn help_names_revoke_all() {
    let database = scratch_database("revoke-help");
    let help = printed(&server(&database, &["-h"], &[]));
    assert!(
        help.contains("admin revoke-all [--passwords] [--yes]"),
        "{help}"
    );
}

#[tokio::test]
async fn revoke_all_spends_an_unused_verification_link() {
    let database = scratch_database("revoke-verify");
    let test = app_over(&database).await;
    test.form(
        "/signup",
        &[
            ("email", "ada@example.com"),
            ("password", common::PASSWORD),
            ("display_name", "Ada"),
        ],
        None,
    )
    .await;
    let link = test.link_in_last_email_to("ada@example.com").await;
    printed(&server(&database, &["admin", "revoke-all", "--yes"], &[]));
    let page = test.page(&link, None).await;
    assert!(page.body.contains("used or expired"), "{}", page.body);
}

#[tokio::test]
async fn revoke_all_passwords_without_yes_clears_no_password() {
    let database = scratch_database("revoke-passwords-dry");
    let test = app_over(&database).await;
    graces_secrets(&test).await;
    let said = printed(&server(
        &database,
        &["admin", "revoke-all", "--passwords"],
        &[],
    ));
    assert!(said.contains("1 password"), "{said}");
    assert_eq!(
        logs_in(&test, common::PASSWORD).await.status,
        StatusCode::SEE_OTHER
    );
}

/// A mistyped FILEDRAWER_DATABASE opens a new, empty database, where revoking
/// nothing would read as done.
#[test]
fn revoke_all_refuses_a_database_with_no_accounts() {
    let database = scratch_database("revoke-empty");
    let output = server(&database, &["admin", "revoke-all", "--yes"], &[]);
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("FILEDRAWER_DATABASE"), "{said}");
}

#[test]
fn revoke_all_refuses_a_flag_it_doesnt_know_or_one_given_twice() {
    let database = scratch_database("revoke-flags");
    for flags in [&["--force"][..], &["--yes", "--yes"]] {
        let args: Vec<&str> = ["admin", "revoke-all"]
            .iter()
            .chain(flags)
            .copied()
            .collect();
        let output = server(&database, &args, &[]);
        assert_eq!(output.status.code(), Some(2), "{flags:?}");
    }
}
