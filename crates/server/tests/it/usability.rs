//! Seam: `ops/usability-run account` and `delete`, and `ops/remove-entries`, run as
//! processes against a real `filedrawer-server serve` on loopback, the way they run against
//! staging. The server logs the mail it can't send, so the scripts read the
//! verification link from its log (FILEDRAWER_USABILITY_MAIL_LOG) instead of IMAP.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};

const RUN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ops/usability-run");
const REMOVE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ops/remove-entries");
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ops/smoke-fixture.md");

struct Server {
    child: Child,
    url: String,
    dir: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn database(&self) -> PathBuf {
        self.dir.path().join("filedrawer.db")
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("server.log")
    }
}

fn serve() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let log = std::fs::File::create(dir.path().join("server.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_filedrawer-server"))
        .arg("serve")
        .env_clear()
        .env("FILEDRAWER_DATABASE", dir.path().join("filedrawer.db"))
        .env("FILEDRAWER_LISTEN", format!("127.0.0.1:{port}"))
        .env("FILEDRAWER_ENVIRONMENT", "dev")
        .env("FILEDRAWER_LINK_CHECKS", "off")
        .env("FILEDRAWER_SCREENER", "pass-all")
        .env("FILEDRAWER_PUBLIC_URL", &url)
        // The mail module logs each email it can't send at info, with its body.
        .env("RUST_LOG", "warn,filedrawer_server::mail=info")
        // The log goes to stdout.
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    // Dropped, it stops the server, so a panic below leaves none running.
    let server = Server { child, url, dir };
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the server never listened on {}", server.url);
}

fn script(path: &str, server: &Server, args: &[&str]) -> Output {
    Command::new(path)
        .args(args)
        .env_remove("BASH_ENV")
        .env_remove("FILEDRAWER_STAGING_KEY")
        .env("FILEDRAWER_USABILITY_MAIL_LOG", server.log())
        .env("FILEDRAWER_USABILITY_MAIL_WAIT", "20")
        .env("FILEDRAWER_USABILITY_DIR", server.dir.path())
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Makes an account through the script and returns its saved login.
fn account(server: &Server, name: &str) -> serde_json::Value {
    let state = server.dir.path().join(format!("{name}.json"));
    let made = script(
        RUN,
        server,
        &[
            "account",
            "--base",
            &server.url,
            "--state",
            state.to_str().unwrap(),
            "--name",
            name,
        ],
    );
    assert!(made.status.success(), "{}", text(&made));
    let login: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    let shown = text(&made);
    assert!(shown.contains(login["id"].as_str().unwrap()), "{shown}");
    for secret in ["token", "password", "address"] {
        assert!(
            !shown.contains(login[secret].as_str().unwrap()),
            "the output shows the {secret}"
        );
    }
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&state).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    login
}

/// `curl` with the token, returning the status code and the body.
fn curl(
    server: &Server,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&Path>,
) -> (u16, String) {
    let mut command = Command::new("curl");
    command.args(["-sS", "-X", method, "-w", "\n%{http_code}", "-H"]);
    command.arg(format!("Authorization: Bearer {token}"));
    if let Some(body) = body {
        command
            .arg("--data-binary")
            .arg(format!("@{}", body.display()));
    }
    let output = command
        .arg(format!("{}{path}", server.url))
        .output()
        .unwrap();
    let out = String::from_utf8(output.stdout).unwrap();
    let (body, code) = out.rsplit_once('\n').unwrap();
    (code.parse().unwrap(), body.to_string())
}

fn submit(server: &Server, token: &str) -> String {
    let (status, body) = curl(server, token, "POST", "/entries", Some(Path::new(FIXTURE)));
    assert_eq!(status, 201, "{body}");
    body.lines()
        .find_map(|line| line.strip_prefix("id: "))
        .unwrap_or_else(|| panic!("no id in {body}"))
        .to_string()
}

#[test]
fn an_account_is_made_through_signup_and_mail_and_deleted_with_its_held_entries() {
    let server = serve();
    let login = account(&server, "jason1");
    let token = login["token"].as_str().unwrap();
    let id = submit(&server, token);
    let state = server.dir.path().join("jason1.json");
    let deleted = script(
        RUN,
        &server,
        &[
            "delete",
            "--base",
            &server.url,
            "--state",
            state.to_str().unwrap(),
        ],
    );
    assert!(deleted.status.success(), "{}", text(&deleted));
    assert!(
        text(&deleted).contains("public entries left: none"),
        "{}",
        text(&deleted)
    );
    // The token went with the account, and so did the entry in the hour's hold.
    let (status, _) = curl(&server, token, "GET", "/entries/mine", None);
    assert_eq!(status, 401);
    let (status, _) = curl(&server, token, "GET", &format!("/e/{id}"), None);
    assert_ne!(status, 200);
}

#[test]
fn remove_entries_takes_down_each_listed_entry_as_the_admin() {
    let server = serve();
    let author = account(&server, "jason2");
    let admin = account(&server, "owner");
    let granted = Command::new(env!("CARGO_BIN_EXE_filedrawer-server"))
        .args(["admin", "grant", "--account", admin["id"].as_str().unwrap()])
        .env_clear()
        .env("FILEDRAWER_DATABASE", server.database())
        .output()
        .unwrap();
    assert!(granted.status.success(), "{}", text(&granted));
    let first = submit(&server, author["token"].as_str().unwrap());
    let second = submit(&server, author["token"].as_str().unwrap());
    let list = server.dir.path().join("public-entries.txt");
    std::fs::write(
        &list,
        format!("{first}\tjason2-claude\n# a comment\n\n{second}\n{first}\n"),
    )
    .unwrap();
    let remove = |file: &Path| {
        Command::new(REMOVE)
            .arg(file)
            .args(["--base", &server.url])
            .env_remove("FILEDRAWER_STAGING_KEY")
            .env("FILEDRAWER_ADMIN_EMAIL", admin["address"].as_str().unwrap())
            .env(
                "FILEDRAWER_ADMIN_PASSWORD",
                admin["password"].as_str().unwrap(),
            )
            .output()
            .unwrap()
    };
    let done = remove(&list);
    assert!(done.status.success(), "{}", text(&done));
    assert!(
        text(&done).contains(&format!("{first}: removed")),
        "{}",
        text(&done)
    );
    assert!(text(&done).contains("2 of 2 removed"), "{}", text(&done));
    let again = remove(&list);
    assert!(again.status.success(), "{}", text(&again));
    assert!(
        text(&again).contains(&format!("{second}: already removed")),
        "{}",
        text(&again)
    );
}

#[test]
fn production_is_refused() {
    let server = serve();
    let refused = script(
        RUN,
        &server,
        &[
            "account",
            "--base",
            "https://filedrawer.dev",
            "--state",
            "/dev/null",
        ],
    );
    assert_eq!(refused.status.code(), Some(2), "{}", text(&refused));
    assert!(
        text(&refused).contains("--base must be"),
        "{}",
        text(&refused)
    );
}

/// A stand-in for drive-agent that does what an agent with the run's token would:
/// submits two entries, publishes the first and leaves the second in its hold, and
/// writes their IDs and the token to `ids`, then its arguments, one a line.
fn fake_drive_agent(dir: &Path, ids: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!(
        r#"#!/bin/sh
set -eu
submit() {{
  curl -sS -H "Authorization: Bearer $FILEDRAWER_TOKEN" --data-binary @{fixture} \
    "$FILEDRAWER_USABILITY_BASE/entries" | sed -n 's/^id: //p'
}}
public=$(submit)
held=$(submit)
curl -sSf -o /dev/null -X POST -H "Authorization: Bearer $FILEDRAWER_TOKEN" \
  "$FILEDRAWER_USABILITY_BASE/e/$public/publish"
printf '%s %s %s\n' "$public" "$held" "$FILEDRAWER_TOKEN" > {ids}
printf '%s\n' "$@" >> {ids}
"#,
        fixture = FIXTURE,
        ids = ids.display()
    );
    let path = bin.join("drive-agent");
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[test]
fn a_token_carrying_run_deletes_its_account_and_lists_the_entries_left_public() {
    let server = serve();
    let ids = server.dir.path().join("ids");
    let bin = fake_drive_agent(server.dir.path(), &ids);
    let scenario = server.dir.path().join("scenario.json");
    std::fs::write(
        &scenario,
        r#"{"token": "header", "turns": ["hi"], "gated": ["posting"]}"#,
    )
    .unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let ran = Command::new(RUN)
        .args([scenario.to_str().unwrap(), "claude", "jason3"])
        .args(["--base", &server.url])
        .env_remove("BASH_ENV")
        .env_remove("FILEDRAWER_STAGING_KEY")
        .env("PATH", path)
        .env("FILEDRAWER_USABILITY_MAIL_LOG", server.log())
        .env("FILEDRAWER_USABILITY_MAIL_WAIT", "20")
        .env("FILEDRAWER_USABILITY_DIR", server.dir.path())
        .output()
        .unwrap();
    assert!(ran.status.success(), "{}", text(&ran));
    let ids = std::fs::read_to_string(&ids).expect("the fake drive-agent ran");
    let (first, args) = ids.split_once('\n').unwrap();
    let [public, held, token] = first.split(' ').collect::<Vec<_>>()[..] else {
        panic!("{first}")
    };
    // The token reaches the setup script, which writes the header file, by name.
    assert!(args.contains("--setup-env\nFILEDRAWER_TOKEN\n"), "{args}");
    let day = std::fs::read_dir(server.dir.path().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let listed = std::fs::read_to_string(day.join("public-entries.txt")).unwrap();
    assert_eq!(listed, format!("{public}\tjason3-claude\n"));
    let note = std::fs::read_to_string(day.join("jason3-claude/account.txt")).unwrap();
    assert!(note.contains("deleted: yes"), "{note}");
    assert!(
        note.contains(&format!("public entries left: {public}\n")),
        "{note}"
    );
    let (status, _) = curl(&server, token, "GET", "/entries/mine", None);
    assert_eq!(status, 401, "the token outlived its account");
    // The public entry stays up; the held one isn't public.
    let anonymous = |id: &str| {
        let out = Command::new("curl")
            .args(["-sS", "-o", "/dev/null", "-w", "%{http_code}"])
            .arg(format!("{}/e/{id}", server.url))
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(anonymous(public), "200");
    assert_ne!(anonymous(held), "200");
}
