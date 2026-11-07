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
    serve_with(database, &[("FILEDRAWER_SCREENER", "pass-all")])
}

/// A server with these extra settings.
fn serve_with(database: PathBuf, settings: &[(&str, &str)]) -> Server {
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let child = Command::new(env!("CARGO_BIN_EXE_filedrawer"))
        .arg("serve")
        .env_clear()
        .env("FILEDRAWER_DATABASE", &database)
        .env("FILEDRAWER_LISTEN", format!("127.0.0.1:{port}"))
        .env("FILEDRAWER_ENVIRONMENT", "dev")
        // So the eval's cases, which link github.com, fetch nothing.
        .env("FILEDRAWER_LINK_CHECKS", "off")
        .env("FILEDRAWER_PUBLIC_URL", &url)
        .env("RUST_LOG", "error")
        .envs(settings.iter().copied())
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
    screen_mode("screen", args, key, token)
}

/// Runs a mode of the script that submits with a screen token (`screen` or `eval`),
/// with its settings files looked for in an empty folder.
fn screen_mode(mode: &str, args: &[&str], key: Option<&str>, token: Option<&str>) -> Output {
    let empty = scratch_database(&format!("{mode}-settings"));
    let empty = empty.parent().unwrap();
    let mut command = script(&[&[mode], args].concat(), key);
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

#[test]
fn eval_against_a_server_that_doesnt_screen_passes_only_the_public_cases() {
    let (server, token) = server_and_token("eval");
    let output = screen_mode(
        "eval",
        &["--field-test", "0", "--wait", "30", &server.url],
        Some("unused"),
        Some(&token),
    );
    let printed = text(&output);
    assert_eq!(output.status.code(), Some(1), "{printed}");
    assert!(
        printed.contains("PASS public jailbreak-study-abstract"),
        "{printed}"
    );
    let failed = |name: &str| {
        printed.lines().any(|line| {
            line.starts_with(&format!("FAIL {name} (")) && line.ends_with("): got public")
        })
    };
    assert!(failed("rejected sourdough"), "{printed}");
    assert!(failed("review participant-data"), "{printed}");
    assert!(printed.contains("public: 4 of 4"), "{printed}");
    assert!(printed.contains("rejected: 0 of 4"), "{printed}");
    assert!(printed.contains("review: 0 of 1"), "{printed}");
    assert!(
        printed.contains("then clear the screen-test account's guard"),
        "{printed}"
    );
    assert!(!printed.contains(&token), "{printed}");
}

#[test]
fn eval_samples_field_test_entries_without_printing_them() {
    let (server, token) = server_and_token("eval-field");
    let fake = scratch_database("eval-field-entries");
    let fake = fake.parent().unwrap().to_path_buf();
    let fixture = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/smoke-fixture.md"),
    )
    .unwrap();
    let marked = fixture.replace("## Caveats", "SECRET-FIELD-TEST-MARKER\n\n## Caveats");
    std::fs::create_dir_all(fake.join("round1")).unwrap();
    std::fs::create_dir_all(fake.join("round2")).unwrap();
    std::fs::write(fake.join("round1/one.md"), &marked).unwrap();
    // A near-copy under the same name in a later round, and a note that isn't an entry.
    std::fs::write(fake.join("round2/one.md"), &marked).unwrap();
    std::fs::write(
        fake.join("round1/notes.md"),
        "SECRET-FIELD-TEST-MARKER notes",
    )
    .unwrap();
    let mut command = script(
        &["eval", "--field-test", "3", "--wait", "30", &server.url],
        Some("unused"),
    );
    let empty = scratch_database("eval-field-settings");
    command
        .env("FILEDRAWER_OPS_DIR", empty.parent().unwrap())
        .env("FILEDRAWER_FIELD_TEST_DIR", &fake)
        .env("FILEDRAWER_STAGING_SCREEN_API_KEY", &token);
    let output = command.output().unwrap();
    let printed = text(&output);
    let _ = std::fs::remove_dir_all(&fake);
    assert!(printed.contains("PASS public field-test-1"), "{printed}");
    assert!(!printed.contains("field-test-2"), "{printed}");
    assert!(
        printed.contains("only 1 field-test entries found"),
        "{printed}"
    );
    assert!(!printed.contains("SECRET-FIELD-TEST-MARKER"), "{printed}");
    assert!(!printed.contains("one.md"), "{printed}");
}

#[test]
fn every_eval_case_is_an_entry_the_analyzer_accepts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/screen-eval");
    let mut seen = 0;
    for expected in ["public", "rejected", "review"] {
        for file in std::fs::read_dir(dir.join(expected)).unwrap() {
            let path = file.unwrap().path();
            let analysis = analyzer::analyze(&std::fs::read_to_string(&path).unwrap());
            assert_eq!(analysis.refusals, vec![], "{}", path.display());
            seen += 1;
        }
    }
    assert!(seen >= 9, "{seen}");
}

/// A server with a real screener (`FILEDRAWER_SCREENER=anthropic`, a made-up key) and
/// these extra settings, and an account's token. No test lets it reach the API: the
/// cap is $0, or its calls go to a proxy that isn't there.
fn screening_server(name: &str, settings: &[(&str, &str)]) -> (Server, String) {
    let database = scratch_database(name);
    let account = admin(&database, &["create-account", "--display-name", "Screen"]);
    let token = admin(
        &database,
        &["create-token", "--account", &account, "--name", "t"],
    );
    let real = [
        ("FILEDRAWER_SCREENER", "anthropic"),
        ("FILEDRAWER_ANTHROPIC_API_KEY", "sk-ant-not-real"),
    ];
    (serve_with(database, &[&real[..], settings].concat()), token)
}

/// Nothing is ever screened, and no model called.
const SPENT: &[(&str, &str)] = &[("FILEDRAWER_SCREENING_DAILY_USD", "0")];

/// Every model call fails at once: the proxy it goes through isn't there.
const FAILING: &[(&str, &str)] = &[
    ("FILEDRAWER_SCREENING_DAILY_USD", "1"),
    ("HTTPS_PROXY", "http://127.0.0.1:9"),
];

/// Runs a screen-test mode of the script against `server` without waiting for a
/// verdict, so a case not decided at once is left undecided.
fn without_waiting(mode_and_args: &[&str], server: &Server, token: &str) -> Output {
    let mut command = script(&[mode_and_args, &[&server.url]].concat(), Some("unused"));
    let empty = scratch_database(&format!("{}-nowait-settings", mode_and_args[0]));
    command
        .env("FILEDRAWER_OPS_DIR", empty.parent().unwrap())
        .env("FILEDRAWER_STAGING_SCREEN_API_KEY", token)
        .env("FILEDRAWER_SCREEN_WAIT_SECONDS", "0");
    command.output().unwrap()
}

#[test]
fn eval_stops_at_the_first_undecided_case_and_says_the_day_is_spent() {
    let (server, token) = screening_server("eval-spent", SPENT);
    let output = without_waiting(&["eval", "--field-test", "0"], &server, &token);
    let printed = text(&output);
    assert_eq!(output.status.code(), Some(1), "{printed}");
    assert_eq!(printed.matches("WAIT ").count(), 1, "{printed}");
    assert!(
        printed.contains("the day's screening cap is spent: rerun after 00:00 UTC"),
        "{printed}"
    );
    assert!(!printed.contains("screening fails"), "{printed}");
    assert!(printed.contains("public: 0 of 1"), "{printed}");
}

#[test]
fn eval_says_when_screening_fails() {
    let (server, token) = screening_server("eval-failing", FAILING);
    let output = without_waiting(&["eval", "--field-test", "0"], &server, &token);
    let printed = text(&output);
    assert_eq!(output.status.code(), Some(1), "{printed}");
    assert_eq!(printed.matches("WAIT ").count(), 1, "{printed}");
    assert!(
        printed.contains("screening fails; the server's log says why"),
        "{printed}"
    );
    assert!(!printed.contains("cap is spent"), "{printed}");
}

#[test]
fn screen_exits_3_when_the_days_cap_is_spent() {
    let (server, token) = screening_server("screen-spent", SPENT);
    let output = without_waiting(&["screen"], &server, &token);
    let printed = text(&output);
    assert_eq!(output.status.code(), Some(3), "{printed}");
    assert!(
        printed.contains("the day's screening cap is spent"),
        "{printed}"
    );
    // The off-topic case would wait for nothing.
    assert!(!printed.contains("off-topic"), "{printed}");
    assert!(!printed.contains(&token), "{printed}");
}

#[test]
fn screen_fails_when_screening_fails() {
    let (server, token) = screening_server("screen-failing", FAILING);
    let output = without_waiting(&["screen"], &server, &token);
    let printed = text(&output);
    assert_eq!(output.status.code(), Some(1), "{printed}");
    assert!(
        printed.contains("screening fails; the server's log says why"),
        "{printed}"
    );
    assert!(!printed.contains("off-topic"), "{printed}");
    assert!(!printed.contains(&token), "{printed}");
}

impl Server {
    /// Stops this server and starts another on the same database with `settings`.
    fn restart_with(mut self, settings: &[(&str, &str)]) -> Server {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Taken, so this server's drop leaves the database for the next one.
        let database = std::mem::take(&mut self.database);
        serve_with(database, settings)
    }
}

/// Made-up hidden text, 48 bytes, and its base64 (made with `base64 -w0`).
const HIDDEN: &str = "made-up instruction for agents reading this file";
const HIDDEN_B64: &str = "bWFkZS11cCBpbnN0cnVjdGlvbiBmb3IgYWdlbnRzIHJlYWRpbmcgdGhpcyBmaWxl";

/// The made-up attached file that hides HIDDEN, marked as the field-test box's files are.
fn hiding_file() -> String {
    format!("# SECRET-FIELD-TEST-MARKER\npayload = '{HIDDEN_B64}'\n")
}

/// A made-up attached file with Windows line ends, which an upload must keep.
fn plain_file() -> &'static str {
    "# SECRET-FIELD-TEST-MARKER\r\ndef plain():\r\n    return 1\r\n"
}

/// The line of an attachment list that names `text` as `origin`.
fn list_line(origin: &str, text: &str) -> String {
    format!("{origin} {} {}\n", text.len(), sha256(text.as_bytes()))
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A made-up field-test folder, as bin/field-test-files leaves it: three entries,
/// `one` and `two` listing the same two attached files, which the store holds, and
/// `three` listing one it doesn't. Every entry and file is marked, so a test can check
/// that none is printed.
fn field_test_box(name: &str) -> PathBuf {
    let dir = scratch_database(name).parent().unwrap().join("field-test");
    let fixture = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/smoke-fixture.md"),
    )
    .unwrap();
    let marked = fixture.replace("## Caveats", "SECRET-FIELD-TEST-MARKER\n\n## Caveats");
    let hiding = hiding_file();
    let plain = plain_file();
    // Round 3's origins name a repo and commit; round 2's are bare paths.
    let list = list_line("repo@1f66d29b25b8:src/hiding.py", &hiding)
        + &list_line("analysis/plain.py", plain);
    std::fs::create_dir_all(dir.join("round3/demo-repo")).unwrap();
    std::fs::create_dir_all(dir.join("files")).unwrap();
    for text in [hiding.as_str(), plain] {
        std::fs::write(dir.join("files").join(sha256(text.as_bytes())), text).unwrap();
    }
    for entry in ["one", "two", "three"] {
        std::fs::write(dir.join(format!("round3/demo-repo/{entry}.md")), &marked).unwrap();
    }
    std::fs::write(dir.join("round3/demo-repo/one.attachments"), &list).unwrap();
    std::fs::write(dir.join("round3/demo-repo/two.attachments"), &list).unwrap();
    let missing = "never gathered\n";
    std::fs::write(
        dir.join("round3/demo-repo/three.attachments"),
        list_line("author:analysis/lost.py", missing),
    )
    .unwrap();
    dir
}

/// `ops/smoke-staging eval` with `args`, as the screen-test account `token`, keeping
/// its run files in `ops_dir`, reading field-test entries from `field_test`, and with
/// `settings` besides.
fn eval(
    args: &[&str],
    token: &str,
    ops_dir: &Path,
    field_test: &Path,
    settings: &[(&str, &str)],
) -> Output {
    let mut command = script(&[&["eval"], args].concat(), Some("unused"));
    command
        .env("FILEDRAWER_OPS_DIR", ops_dir)
        .env("FILEDRAWER_FIELD_TEST_DIR", field_test)
        .env("FILEDRAWER_STAGING_SCREEN_API_KEY", token)
        .env("FILEDRAWER_BIN", env!("CARGO_BIN_EXE_filedrawer"))
        .envs(settings.iter().copied());
    command.output().unwrap()
}

/// The run file eval printed the path of, read.
fn run_file(printed: &str) -> (PathBuf, String) {
    let path = printed
        .lines()
        .find_map(|line| line.strip_prefix("run file: "))
        .unwrap_or_else(|| panic!("no run file named: {printed}"));
    (PathBuf::from(path), std::fs::read_to_string(path).unwrap())
}

/// The IDs of the entries a run file says were sent, with the path each was sent from.
fn sent(run: &str) -> Vec<(String, String)> {
    run.lines()
        .filter_map(|line| line.strip_prefix("sent\t"))
        .map(|rest| {
            let (path, id) = rest.split_once('\t').unwrap();
            (path.to_string(), id.to_string())
        })
        .collect()
}

#[test]
fn eval_sends_field_test_entries_citing_their_attachment_sets() {
    let (server, token) = serve_attachments("eval-sets");
    let field_test = field_test_box("eval-sets-entries");
    let ops = scratch_database("eval-sets-ops");
    let ops = ops.parent().unwrap();
    let output = eval(
        &["--field-test", "3", "--wait", "30", &server.url],
        &token,
        ops,
        &field_test,
        &[],
    );
    let printed = text(&output);
    assert!(printed.contains("PASS public field-test-1"), "{printed}");
    assert!(printed.contains("PASS public field-test-2"), "{printed}");
    assert!(!printed.contains("field-test-3"), "{printed}");
    assert!(
        printed.contains("only 2 field-test entries found"),
        "{printed}"
    );
    assert!(
        printed.contains("1 field-test entries passed over"),
        "{printed}"
    );
    assert!(printed.contains("bin/field-test-files"), "{printed}");
    let (_, run) = run_file(&printed);
    // The two entries share one list, so one upload.
    assert_eq!(run.matches("\nset\t").count(), 1, "{run}");
    let field_tests: Vec<_> = sent(&run)
        .into_iter()
        .filter(|(path, _)| path.starts_with(field_test.to_str().unwrap()))
        .collect();
    assert_eq!(field_tests.len(), 2, "{run}");
    for (_, id) in &field_tests {
        let shown = curl_entry(&server, &token, id);
        assert!(shown.contains("attachments: as-"), "{shown}");
        assert!(shown.contains(", 2 files"), "{shown}");
        // The upload kept the file's bytes, Windows line ends included.
        assert!(shown.contains(&sha256(plain_file().as_bytes())), "{shown}");
    }
    let _ = std::fs::remove_dir_all(field_test.parent().unwrap());
    for secret in [
        "SECRET-FIELD-TEST-MARKER",
        "one.md",
        HIDDEN,
        &HIDDEN_B64[..16],
    ] {
        assert!(!printed.contains(secret), "{secret} printed: {printed}");
    }
}

#[test]
fn eval_counts_what_decoding_finds_in_the_sets_it_sent() {
    let (server, token) = serve_attachments("eval-decode");
    let field_test = field_test_box("eval-decode-entries");
    // A fourth entry whose list differs but names the hiding file too: a second set,
    // which shares that file.
    let demo = field_test.join("round3/demo-repo");
    std::fs::copy(demo.join("one.md"), demo.join("four.md")).unwrap();
    std::fs::write(
        demo.join("four.attachments"),
        list_line("author:hiding.py", &hiding_file()),
    )
    .unwrap();
    let ops = scratch_database("eval-decode-ops");
    let output = eval(
        &["--field-test", "3", "--wait", "30", &server.url],
        &token,
        ops.parent().unwrap(),
        &field_test,
        &[],
    );
    let printed = text(&output);
    let _ = std::fs::remove_dir_all(field_test.parent().unwrap());
    // Each file is counted once, though three entries and two sets cite the hiding one.
    assert!(
        printed.contains(&format!(
            "decoding, run here with no model call: 1 of 2 attached files hide text; parts found: 1, {} bytes; past a limit: 0",
            HIDDEN.len()
        )),
        "{printed}"
    );
    assert!(
        printed.contains(
            "field-test entries citing a set that hides text or passes a limit: 3 (public 3, rejected 0, review 0)"
        ),
        "{printed}"
    );
    assert!(!printed.contains(HIDDEN), "{printed}");
}

#[test]
fn eval_resumes_a_stopped_run_without_sending_anything_twice() {
    let (server, token) = screening_server("eval-resume", SPENT);
    let ops = scratch_database("eval-resume-ops");
    let ops = ops.parent().unwrap();
    let field_test = field_test_box("eval-resume-entries");
    let first = eval(
        &["--field-test", "0", "--wait", "30", &server.url],
        &token,
        ops,
        &field_test,
        &[("FILEDRAWER_SCREEN_WAIT_SECONDS", "0")],
    );
    let printed = text(&first);
    assert!(printed.contains("WAIT public"), "{printed}");
    let (path, run) = run_file(&printed);
    // With no field-test entries wanted, none is picked.
    assert!(!run.contains("\ncandidate\t"), "{run}");
    assert!(
        printed.contains(&format!(
            "Resume with: ops/smoke-staging eval --resume {}",
            path.display()
        )),
        "{printed}"
    );
    let before = sent(&run);
    assert_eq!(before.len(), 1, "{run}");
    let tag = path.file_name().unwrap().to_str().unwrap().to_string();

    // The cap no longer stops screening: the waiting entry is decided, and the rest sent.
    let server = server.restart_with(&[("FILEDRAWER_SCREENER", "pass-all")]);
    let output = eval(
        &[
            "--resume",
            path.to_str().unwrap(),
            "--wait",
            "30",
            &server.url,
        ],
        &token,
        ops,
        &field_test,
        &[],
    );
    let _ = std::fs::remove_dir_all(field_test.parent().unwrap());
    let printed = text(&output);
    assert!(printed.contains("public: 4 of 4"), "{printed}");
    assert!(printed.contains(&format!("eval {tag}: ")), "{printed}");
    let after = sent(&std::fs::read_to_string(&path).unwrap());
    let cases = ["public", "rejected", "review"]
        .iter()
        .map(|expected| {
            std::fs::read_dir(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../ops/screen-eval")
                    .join(expected),
            )
            .unwrap()
            .filter(|file| file.as_ref().unwrap().path().extension() == Some("md".as_ref()))
            .count()
        })
        .sum::<usize>();
    assert_eq!(after.len(), cases, "each case sent once: {after:?}");
    assert_eq!(after[0], before[0]);
}

#[test]
fn a_resumed_run_cites_the_sets_it_already_uploaded() {
    let (server, token) = serve_attachments("eval-resume-sets");
    let field_test = field_test_box("eval-resume-sets-entries");
    let ops = scratch_database("eval-resume-sets-ops");
    let ops = ops.parent().unwrap();
    let args = ["--wait", "30", &server.url];
    let first = eval(
        &[&["--field-test", "1"], &args[..]].concat(),
        &token,
        ops,
        &field_test,
        &[],
    );
    let printed = text(&first);
    assert!(printed.contains("PASS public field-test-1"), "{printed}");
    let (path, run) = run_file(&printed);
    // Stands for a run that stopped after its first field-test entry: it wanted two.
    std::fs::write(&path, run.replace("field-tests\t1\n", "field-tests\t2\n")).unwrap();
    let second = eval(
        &[&["--resume", path.to_str().unwrap()], &args[..]].concat(),
        &token,
        ops,
        &field_test,
        &[],
    );
    let printed = text(&second);
    assert!(printed.contains("PASS public field-test-2"), "{printed}");
    let run = std::fs::read_to_string(&path).unwrap();
    // The two entries share one list: the resumed run cites the first run's set.
    let sets: Vec<_> = run
        .lines()
        .filter_map(|line| line.strip_prefix("set\t"))
        .collect();
    assert_eq!(sets.len(), 1, "{run}");
    let set = sets[0].split('\t').nth(1).unwrap();
    let field_tests: Vec<_> = sent(&run)
        .into_iter()
        .filter(|(path, _)| path.starts_with(field_test.to_str().unwrap()))
        .collect();
    assert_eq!(field_tests.len(), 2, "{run}");
    for (_, id) in &field_tests {
        let shown = curl_entry(&server, &token, id);
        assert!(shown.contains(&format!("attachments: {set},")), "{shown}");
    }
    // The tally covers the whole run, the first run's set included.
    assert!(
        printed.contains("citing a set that hides text or passes a limit: 2 (public 2"),
        "{printed}"
    );
    let _ = std::fs::remove_dir_all(field_test.parent().unwrap());
}

#[test]
fn resume_takes_its_cases_from_the_run_file_alone() {
    let output = script(
        &["eval", "--resume", "/nowhere", "--field-test", "3"],
        Some("unused"),
    )
    .output()
    .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(
        text(&output).contains("--field-test can't go with it"),
        "{}",
        text(&output)
    );
    let ops = scratch_database("eval-not-a-run");
    let ops = ops.parent().unwrap();
    std::fs::write(ops.join("notes"), "not a run file\n").unwrap();
    let output = screen_mode(
        "eval",
        &["--resume", ops.join("notes").to_str().unwrap()],
        Some("unused"),
        Some("fdt_x"),
    );
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(
        text(&output).contains("as an eval run file"),
        "{}",
        text(&output)
    );
}

/// A server that takes attachments and passes everything, and an account's token. Its
/// database, and the attached files beside it, sit under target/: the server refuses
/// an upload that would leave its disk under 5 GB free, which a small /tmp can be.
fn serve_attachments(name: &str) -> (Server, String) {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("smoke-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let database = dir.join("filedrawer.db");
    let account = admin(&database, &["create-account", "--display-name", "Sets"]);
    let token = admin(
        &database,
        &["create-token", "--account", &account, "--name", "t"],
    );
    let settings = [
        ("FILEDRAWER_SCREENER", "pass-all"),
        ("FILEDRAWER_FILES_URL", "http://files.localhost"),
    ];
    (serve_with(database, &settings), token)
}

/// GET /e/<id> as the token's account, through curl, as the script sends requests.
fn curl_entry(server: &Server, token: &str, id: &str) -> String {
    let output = Command::new("curl")
        .args(["-s", "-H", &format!("Authorization: Bearer {token}")])
        .arg(format!("{}/e/{id}", server.url))
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A made-up family as the field test's round 3 left it, under `field-test/`: entries
/// `a` and `b` name runs that `runs.yaml` declares, cite a series by a made-up ID that
/// `series-demo.md` holds, and link each other by slug, and `b` attaches a gathered
/// file. Beside them are notes that aren't an entry, and an older round of the same
/// repo, which `--families` leaves alone. Every text is marked, so a test can check that
/// none is printed.
fn family_box(name: &str) -> PathBuf {
    let dir = scratch_database(name).parent().unwrap().join("field-test");
    let fixture = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/smoke-fixture.md"),
    )
    .unwrap();
    let marked = fixture.replace("## Caveats", "SECRET-FIELD-TEST-MARKER\n\n## Caveats");
    let with = |fields: &str| marked.replacen("---\n", &format!("---\n{fields}"), 1);
    let family = dir.join("round3/demo-repo");
    std::fs::create_dir_all(&family).unwrap();
    std::fs::create_dir_all(dir.join("round2/demo-repo")).unwrap();
    std::fs::create_dir_all(dir.join("files")).unwrap();
    std::fs::write(
        family.join("a.md"),
        with("slug: alpha\nuses_runs:\n  - run-a\nseries: ser_madeUp1@2#setup\n"),
    )
    .unwrap();
    std::fs::write(
        family.join("b.md"),
        with("slug: beta\nuses_runs: [run-a, run-b]\nreplicates: slug:alpha\nseries: ser_madeUp1@1\n"),
    )
    .unwrap();
    let plain = plain_file();
    std::fs::write(dir.join("files").join(sha256(plain.as_bytes())), plain).unwrap();
    std::fs::write(
        family.join("b.attachments"),
        list_line("author:analysis/plain.py", plain),
    )
    .unwrap();
    std::fs::write(
        family.join("series-demo.md"),
        "---\ntitle: Made-up smoke-test series\n# its made-up id: ser_madeUp1\n---\nSECRET-FIELD-TEST-MARKER shared setup, made up.\n\n## Setup\n\nMade-up setup.\n",
    )
    .unwrap();
    std::fs::write(
        family.join("runs.yaml"),
        "runs:\n  - id: run-a\n    description: Made-up run A.\n    n: 10\n  - id: run-b\n    description: |\n      Made-up run B,\n      over two lines.\n    n: 20\n",
    )
    .unwrap();
    std::fs::write(
        family.join("FINDINGS.md"),
        "SECRET-FIELD-TEST-MARKER notes\n",
    )
    .unwrap();
    std::fs::write(dir.join("round2/demo-repo/old.md"), &marked).unwrap();
    // A family of its own whose one entry the server refuses: its code link has no
    // commit. Its reasons quote the slug and the link outside backticks.
    std::fs::create_dir_all(dir.join("round3/bad-repo")).unwrap();
    std::fs::write(
        dir.join("round3/bad-repo/bad.md"),
        with("slug: secret-slug-marker\ncode: https://github.com/made-up/secret-repo-marker\n"),
    )
    .unwrap();
    dir
}

#[test]
fn eval_sends_each_field_test_family_as_batches_and_resumes_without_resending() {
    let (server, token) = serve_attachments("eval-families");
    let field_test = family_box("eval-families-entries");
    let ops = scratch_database("eval-families-ops");
    let ops = ops.parent().unwrap();
    // Small enough that a and b go in batches of their own.
    let split = [("FILEDRAWER_BATCH_MAX_BYTES", "2500")];
    let output = eval(
        &["--families", "--wait", "30", &server.url],
        &token,
        ops,
        &field_test,
        &split,
    );
    let printed = text(&output);
    let first_printed = printed.clone();
    assert!(printed.contains("PASS series series-1"), "{printed}");
    // A refused batch is a failure, and its reasons go to a file beside the run file,
    // not to the output.
    assert!(
        printed.contains("FAIL a batch of 1 field-test entries was refused (422)"),
        "{printed}"
    );
    // A member passed while its series is hidden waits on it: the screen passed it.
    assert!(printed.contains("PASS public field-test-1"), "{printed}");
    assert!(printed.contains("PASS public field-test-2"), "{printed}");
    assert!(!printed.contains("field-test-3"), "{printed}");
    let (path, run) = run_file(&printed);
    let members: Vec<_> = sent(&run)
        .into_iter()
        .filter(|(path, _)| path.starts_with(field_test.to_str().unwrap()))
        .collect();
    assert_eq!(members.len(), 2, "{run}");
    let id_of = |name: &str| {
        members
            .iter()
            .find(|(path, _)| path.ends_with(name))
            .map(|(_, id)| id.clone())
            .unwrap()
    };
    // Two batches, the second citing runs only the first declares, which the server
    // takes: a batch naming an undeclared run is refused.
    assert_eq!(run.matches("\nbatch\t").count(), 2, "{run}");
    assert_eq!(run.matches("\nseries\t").count(), 1, "{run}");
    let series = run
        .lines()
        .find_map(|line| line.strip_prefix("series\t"))
        .unwrap()
        .split('\t')
        .nth(1)
        .unwrap()
        .to_string();
    // A member waiting on its series shows the real series it cites and its code.
    let shown = curl_entry(&server, &token, &id_of("/b.md"));
    assert!(shown.contains(&format!("series: {series}@1")), "{shown}");
    assert!(shown.contains("analysis/plain.py"), "{shown}");

    let again = eval(
        &[
            "--resume",
            path.to_str().unwrap(),
            "--wait",
            "30",
            &server.url,
        ],
        &token,
        ops,
        &field_test,
        &split,
    );
    let printed = text(&again);
    assert!(printed.contains("PASS public field-test-2"), "{printed}");
    let run = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        sent(&run).len(),
        sent(&run)
            .iter()
            .map(|(p, _)| p)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "{run}"
    );
    assert_eq!(run.matches("\nseries\t").count(), 1, "{run}");
    let reasons = std::fs::read_to_string(format!("{}.refused-1", path.display())).unwrap();
    assert!(reasons.contains("secret-repo-marker"), "{reasons}");
    let _ = std::fs::remove_dir_all(field_test.parent().unwrap());
    for out in [&first_printed, &printed] {
        for secret in [
            "SECRET-FIELD-TEST-MARKER",
            "a.md",
            "Made-up run",
            "secret-slug-marker",
            "secret-repo-marker",
        ] {
            assert!(!out.contains(secret), "{secret} printed: {out}");
        }
    }
}

#[test]
fn families_and_field_test_samples_dont_go_together() {
    let output = script(&["eval", "--families", "--field-test", "3"], Some("unused"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains("--families"), "{}", text(&output));
}
