//! Seam: `ops/smoke-staging`, run as a process against a real `filedrawer serve` on
//! loopback. CI runs the script against staging after each deploy, so a change to the
//! server's replies that would break it there fails here first.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ops/smoke-staging");

/// A `filedrawer serve` on a free loopback port, stopped when dropped.
struct Server {
    child: Child,
    url: String,
    database: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(dir) = self.database.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A new database file in a folder of its own.
fn scratch_database(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("filedrawer-smoke-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("filedrawer.db")
}

/// A free loopback port: one the system just handed out and nobody holds.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn serve(database: PathBuf) -> Server {
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let child = Command::new(env!("CARGO_BIN_EXE_filedrawer"))
        .arg("serve")
        .env_clear()
        .env("FILEDRAWER_DATABASE", &database)
        .env("FILEDRAWER_LISTEN", format!("127.0.0.1:{port}"))
        .env("FILEDRAWER_ENVIRONMENT", "dev")
        .env("FILEDRAWER_SCREENER", "pass-all")
        .env("FILEDRAWER_PUBLIC_URL", &url)
        .env("RUST_LOG", "error")
        .spawn()
        .unwrap();
    Server {
        child,
        url,
        database,
    }
}

/// Runs a `filedrawer admin` command on `database` and returns what it printed.
fn admin(database: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_filedrawer"))
        .arg("admin")
        .args(args)
        .env_clear()
        .env("FILEDRAWER_DATABASE", database)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// A server with an account, and that account's token. The account is made before the
/// server starts, since both would run the migrations on a new file.
fn server_and_token(name: &str) -> (Server, String) {
    let database = scratch_database(name);
    let account = admin(&database, &["create-account", "--display-name", "Smoke"]);
    let token = admin(
        &database,
        &["create-token", "--account", &account, "--name", "smoke"],
    );
    (serve(database), token)
}

/// The script with `args`, none of its settings inherited, and the staging key if
/// given. A BASH_ENV file, as in the sandbox, could set the settings again.
fn script(args: &[&str], key: Option<&str>) -> Command {
    let mut command = Command::new(SCRIPT);
    command
        .args(args)
        .env_remove("BASH_ENV")
        .env_remove("FILEDRAWER_STAGING_KEY")
        .env_remove("FILEDRAWER_STAGING_API_KEY")
        .env_remove("FILEDRAWER_STAGING_SCREEN_API_KEY");
    if let Some(key) = key {
        command.env("FILEDRAWER_STAGING_KEY", key);
    }
    command
}

fn smoke(args: &[&str], key: Option<&str>, token: Option<&str>) -> Output {
    let mut command = script(args, key);
    if let Some(token) = token {
        command.env("FILEDRAWER_STAGING_API_KEY", token);
    }
    command.output().unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn every_check_passes_against_this_server() {
    let (server, token) = server_and_token("pass");
    let output = smoke(&["--wait", "30", &server.url], Some("unused"), Some(&token));
    let printed = text(&output);
    assert!(output.status.success(), "{printed}");
    assert!(!printed.contains("FAIL"), "{printed}");
    assert!(printed.contains(" 0 failed"), "{printed}");
    assert!(
        printed
            .lines()
            .filter(|line| line.starts_with("PASS "))
            .count()
            >= 18,
        "{printed}"
    );
}

#[test]
fn a_failed_check_fails_the_run() {
    let (server, _) = server_and_token("fail");
    let output = smoke(
        &["--wait", "30", &server.url],
        Some("unused"),
        Some("fdt_not-a-token"),
    );
    let printed = text(&output);
    assert!(!output.status.success(), "{printed}");
    assert!(printed.contains("FAIL submit"), "{printed}");
}

#[test]
fn a_server_that_never_answers_fails_after_the_wait() {
    let url = format!("http://127.0.0.1:{}", free_port());
    let output = smoke(&["--wait", "1", &url], Some("k"), Some("t"));
    let printed = text(&output);
    assert!(!output.status.success(), "{printed}");
    assert!(printed.contains("/skill.md"), "{printed}");
}

#[test]
fn it_names_a_missing_key_without_running() {
    let output = smoke(&["http://127.0.0.1:9"], None, Some("t"));
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains("FILEDRAWER_STAGING_KEY"));
    let output = smoke(&["http://127.0.0.1:9"], Some("k"), None);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains("FILEDRAWER_STAGING_API_KEY"));
}

#[test]
fn h_says_what_it_does() {
    let output = smoke(&["-h"], None, None);
    assert!(output.status.success(), "{}", text(&output));
    let usage = String::from_utf8_lossy(&output.stdout);
    assert!(usage.contains("usage: ops/smoke-staging"), "{usage}");
    assert!(usage.contains("--wait"), "{usage}");
}

/// Runs `ops/smoke-staging screen`, with its settings files looked for in an empty
/// folder so a real token on this machine can't be read.
fn screen(args: &[&str], key: Option<&str>, token: Option<&str>) -> Output {
    let empty = scratch_database("screen-settings");
    let empty = empty.parent().unwrap();
    let mut command = script(&[&["screen"], args].concat(), key);
    command.env("FILEDRAWER_OPS_DIR", empty);
    if let Some(token) = token {
        command.env("FILEDRAWER_STAGING_SCREEN_API_KEY", token);
    }
    let output = command.output().unwrap();
    let _ = std::fs::remove_dir_all(empty);
    output
}

#[test]
fn screen_against_a_server_that_doesnt_screen_fails_on_the_off_topic_entry() {
    let (server, token) = server_and_token("screen");
    let output = screen(&["--wait", "30", &server.url], Some("unused"), Some(&token));
    let printed = text(&output);
    assert_eq!(output.status.code(), Some(1), "{printed}");
    assert!(printed.contains("PASS on-topic entry: public"), "{printed}");
    assert!(
        printed.contains("FAIL off-topic entry: rejected"),
        "{printed}"
    );
    assert!(printed.contains("pass-all"), "{printed}");
    assert!(!printed.contains(&token), "{printed}");
}

#[test]
fn screen_without_its_settings_names_where_it_looked() {
    let output = screen(&[], Some("unused"), None);
    let printed = text(&output);
    assert_eq!(output.status.code(), Some(2), "{printed}");
    assert!(
        printed.contains("FILEDRAWER_STAGING_SCREEN_API_KEY"),
        "{printed}"
    );
    assert!(printed.contains(".screen-test.env"), "{printed}");
    let output = screen(&[], None, Some("fdt_x"));
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains(".vps.env"), "{}", text(&output));
}
