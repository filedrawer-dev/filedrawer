//! Seam: the router, for `/skill.md` (the installer), the skills' files under
//! `/skills/<name>/` and `/entry-format.md` (design.md, Agent interface and Skills);
//! and what they promise: every route they name exists, they name every route meant
//! for agents, the example entry passes the analyzer, and the shell commands they give
//! install, upgrade and set up exactly what they say.

use crate::common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::skills::{NAMES, files, raw, tags_mirror, version};
use common::{PUBLIC_URL, test_app};
use filedrawer_server::route_paths;

/// Routes the installer and the skills needn't name: the account pages a person uses
/// in a browser, `/entry-format.md`, which each skill bundles, and staging's own state.
const NOT_FOR_AGENTS: &[&str] = &[
    "/verify",
    "/login",
    "/logout",
    "/reset",
    "/reset/confirm",
    "/account/tokens",
    "/account/tokens/{id}/revoke",
    "/account/verify-email",
    "/account/series/{id}/publish",
    "/account/delete",
    "/account/e/{id}/withdraw",
    "/admin",
    "/admin/invites",
    "/admin/invites/{code}/revoke",
    "/admin/e/{id}/release",
    "/admin/e/{id}",
    "/admin/e/{id}/publish",
    "/admin/e/{id}/hold",
    "/admin/e/{id}/reject",
    "/admin/e/{id}/reject-chain",
    "/admin/e/{id}/doubt-seen",
    "/admin/series-refusals/{id}/confirm",
    "/admin/series-refusals/{id}/dismiss",
    "/admin/accounts/{id}/clear-guard",
    "/admin/f/{sha256}",
    "/admin/f/{sha256}/remove",
    "/admin/f/{sha256}/remove-hazard",
    "/entry-format.md",
    "/staging/status",
    // Its plain text only points an agent without the skills to /skill.md, so the
    // skills needn't name it (CODING_STANDARDS.md, Untrusted text).
    "/",
    "/static/site.css",
    "/static/fonts/{name}",
];

/// Each URL on this site that `text` names, as a path without its query, with each
/// `<name>` written as the router writes it, `{name}`.
fn named_paths(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '`' || c == '"')
        .filter_map(|word| word.strip_prefix(PUBLIC_URL))
        .map(|path| path.trim_end_matches(['.', ',', ')', ':', ';']))
        .map(|path| path.split(['?', '#']).next().unwrap_or(path))
        .filter(|path| !path.is_empty())
        .map(|path| path.replace('<', "{").replace('>', "}"))
        .collect()
}

async fn served(path: &str) -> String {
    let app = test_app().await;
    let reply = app.get(path).await;
    assert_eq!(reply.status, StatusCode::OK, "{path}");
    assert_eq!(
        reply.headers["content-type"], "text/markdown; charset=utf-8",
        "{path}"
    );
    reply.body
}

/// The installer and every file of both skills, as the site serves them.
async fn every_document() -> String {
    let mut text = served("/skill.md").await;
    for name in NAMES {
        for file in files(name) {
            text += &served(&format!("/skills/{name}/{file}")).await;
        }
    }
    text
}

#[tokio::test]
async fn skill_md_is_the_installer_for_this_site() {
    let text = served("/skill.md").await;
    for name in NAMES {
        assert!(
            text.contains(&format!("{name}/{}", version(name))),
            "{text}"
        );
        let tag = format!(
            "https://github.com/filedrawer-dev/filedrawer/tree/skill-{name}-v{}/skills/{name}",
            version(name)
        );
        assert!(text.contains(&tag), "{text}");
    }
    assert!(
        text.contains(&format!("{PUBLIC_URL}/skills/<name>/SKILL.md")),
        "{text}"
    );
    assert!(text.contains("filedrawer/localhost.header"), "{text}");
    assert!(
        text.contains("sed 's#[{]{BASE}}#http://localhost#g; s#[{]{HOST}}#localhost#g'"),
        "{text}"
    );
    for placeholder in ["{{BASE}}", "{{HOST}}", "{{SKILLS}}", "{{TAG_URLS}}"] {
        assert!(!text.contains(placeholder), "{placeholder} is left");
    }
}

#[tokio::test]
async fn skill_md_names_each_skills_version_in_a_header() {
    let app = test_app().await;
    let expected = format!(
        "filedrawer-search/{}, filedrawer-submit/{}",
        version("filedrawer-search"),
        version("filedrawer-submit")
    );
    for method in ["GET", "HEAD"] {
        let reply = app
            .send(
                Request::builder()
                    .method(method)
                    .uri("/skill.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{method}");
        assert_eq!(reply.headers["x-filedrawer-skills"], expected, "{method}");
    }
}

#[tokio::test]
async fn no_other_reply_names_the_versions() {
    let app = test_app().await;
    for path in [
        "/tags",
        "/entry-format.md",
        "/skills/filedrawer-submit/SKILL.md",
        "/no-such-page",
    ] {
        let reply = app.get(path).await;
        assert!(
            !reply.headers.contains_key("x-filedrawer-skills"),
            "{path} names the versions"
        );
    }
}

#[tokio::test]
async fn each_skills_files_are_served_for_this_site() {
    for name in NAMES {
        assert!(files(name).len() >= 3, "{name}");
        for file in files(name) {
            let text = served(&format!("/skills/{name}/{file}")).await;
            let expected = raw(name, &file)
                .replace("{{BASE}}", "http://localhost")
                .replace("{{HOST}}", "localhost");
            assert_eq!(text, expected, "{name}/{file}");
        }
    }
}

#[tokio::test]
async fn an_unknown_skill_or_file_is_not_found() {
    let app = test_app().await;
    for path in [
        "/skills/filedrawer-other/SKILL.md",
        "/skills/filedrawer-other/references/entry-format.md",
        "/skills/filedrawer-search/references/nothing.md",
        "/skills/filedrawer-search/../filedrawer-submit/SKILL.md",
    ] {
        assert_eq!(app.get(path).await.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn entry_format_md_is_the_format_this_server_reads() {
    let app = test_app().await;
    let reply = app.get("/entry-format.md").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.headers["content-type"],
        "text/markdown; charset=utf-8"
    );
    assert_eq!(reply.body, include_str!("../../../../docs/entry-format.md"));
}

#[tokio::test]
async fn every_route_the_documents_name_exists() {
    let text = every_document().await;
    let app = test_app().await;
    let mut seen = 0;
    for path in named_paths(&text) {
        let path = path
            .replace("{id}", "0000000000")
            .replace("{name}", "filedrawer-search")
            .replace("{path}", "SKILL.md");
        if path.contains('{') {
            continue;
        }
        seen += 1;
        for method in ["GET", "POST", "DELETE"] {
            let reply = app
                .send(
                    Request::builder()
                        .method(method)
                        .uri(&path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            // An unknown entry or series is not found; the route list covers those.
            let missing = reply.status == StatusCode::NOT_FOUND
                && !path.starts_with("/e/")
                && !path.starts_with("/series/");
            assert!(!missing, "{method} {path} is not a route");
            if reply.status != StatusCode::METHOD_NOT_ALLOWED {
                break;
            }
            assert_ne!(
                method, "DELETE",
                "{path} answers neither GET, POST nor DELETE"
            );
        }
    }
    assert!(seen >= 6, "the documents name only {seen} URLs");
}

#[tokio::test]
async fn the_documents_name_every_route_meant_for_agents() {
    let named = named_paths(&every_document().await);
    let routes = route_paths(test_app().await.state);
    assert!(routes.len() >= 20, "the router lists only {routes:?}");
    for route in &routes {
        let listed = NOT_FOR_AGENTS.contains(route);
        // A document names a catch-all `{*path}` as `<path>`.
        let as_named = route.replace("{*", "{");
        let is_named = named.contains(&as_named);
        assert!(
            is_named || listed,
            "neither /skill.md nor a skill names {route}; name it, or list it in NOT_FOR_AGENTS"
        );
        assert!(
            !(is_named && listed),
            "a document names {route}, so take it off NOT_FOR_AGENTS"
        );
    }
    for listed in NOT_FOR_AGENTS {
        assert!(routes.contains(listed), "{listed} is not a route");
    }
}

/// The text between the fences of the example entry in `text`.
fn example(text: &str) -> String {
    let fence = "````markdown\n";
    let start = text.find(fence).expect("an example entry") + fence.len();
    let end = start + text[start..].find("\n````").unwrap();
    text[start..end].to_string()
}

#[tokio::test]
async fn the_example_entry_passes_the_analyzer_cleanly() {
    let text = served("/skills/filedrawer-submit/SKILL.md").await;
    let analysis = analyzer::analyze(&example(&text));
    assert_eq!(analysis.refusals, vec![]);
    assert_eq!(analysis.warnings, vec![]);
}

#[tokio::test]
async fn the_example_entry_is_entry_format_mds_example() {
    let format = include_str!("../../../../docs/entry-format.md");
    assert_eq!(
        example(&served("/skills/filedrawer-submit/SKILL.md").await),
        example(format)
    );
}

// The shell commands, run for real against a local copy of the GitHub tags.

const RAW_GITHUB: &str = "https://raw.githubusercontent.com/filedrawer-dev/filedrawer";

fn mirror(root: &Path) -> PathBuf {
    tags_mirror(&root.join("mirror"), |_, _, text| text)
}

/// The `n`th `sh -eu <<'SH'` script after `heading` in `text`, through its `SH` line.
fn script(text: &str, heading: &str, n: usize) -> String {
    let after = &text[text.find(heading).expect(heading)..];
    let start = after
        .match_indices("\nsh -eu <<'SH'\n")
        .nth(n)
        .expect("a script")
        .0
        + 1;
    let block = &after[start..];
    let end = block.find("\nSH\n").expect("the script's end") + "\nSH\n".len();
    block[..end].to_string()
}

/// The `n`th one-line `sh` block after `heading` in `text`.
fn command(text: &str, heading: &str, n: usize) -> String {
    let after = &text[text.find(heading).expect(heading)..];
    let fence = "```sh\n";
    let start = after.match_indices(fence).nth(n).expect("a command").0 + fence.len();
    let line = &after[start..];
    line[..line.find('\n').unwrap()].trim().to_string()
}

/// Runs `script` with `home` as HOME, the GitHub URLs pointing at `mirror`, and
/// `stdin` as its input.
fn run_with(script: &str, home: &Path, mirror: &Path, env: &[(&str, &str)], stdin: &str) -> Output {
    let script = script.replace(RAW_GITHUB, &format!("file://{}", mirror.display()));
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("HOME", home)
        .env_remove("FILEDRAWER_TOKEN")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn run(script: &str, home: &Path, mirror: &Path) -> Output {
    run_with(script, home, mirror, &[], "")
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr)
}

/// Asserts the skill installed under `home` holds what the site serves, and Claude
/// Code's folder links to it.
async fn assert_installed(home: &Path, name: &str) {
    let installed = home.join(".agents/skills").join(name);
    for file in files(name) {
        assert_eq!(
            std::fs::read_to_string(installed.join(&file)).unwrap(),
            served(&format!("/skills/{name}/{file}")).await,
            "{name}/{file}"
        );
    }
    let link = home.join(".claude/skills").join(name);
    assert_eq!(std::fs::read_link(&link).unwrap(), installed, "{name}");
}

async fn installer() -> String {
    served("/skill.md").await
}

/// A home where the installer has run, with Claude Code's folder present.
async fn installed_home(root: &Path, mirror: &Path) -> PathBuf {
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let output = run(&script(&installer().await, "## Install", 0), &home, mirror);
    assert!(output.status.success(), "{}", text(&output));
    home
}

#[tokio::test]
async fn the_installer_installs_what_the_site_serves() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    for name in NAMES {
        assert_installed(&home, name).await;
    }
    let header = home.join(".config/filedrawer/localhost.header");
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(header).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
}

#[tokio::test]
async fn the_installer_installs_neither_skill_when_one_tag_is_missing() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let submit = format!(
        "refs/tags/skill-filedrawer-submit-v{}",
        version("filedrawer-submit")
    );
    std::fs::remove_dir_all(mirror.join(&submit)).unwrap();
    let home = root.path().join("home");
    let output = run(&script(&installer().await, "## Install", 0), &home, &mirror);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains(&format!(
            "couldn't fetch file://{}/{submit}",
            mirror.display()
        )),
        "{}",
        text(&output)
    );
    for name in NAMES {
        assert!(!home.join(".agents/skills").join(name).exists(), "{name}");
    }
}

#[tokio::test]
async fn recording_the_mode_keeps_the_versions_declined_before() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let config = home.join(".config/filedrawer");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("upgrade"),
        "auto\ndeclined filedrawer-search 4\n",
    )
    .unwrap();
    let record = script(&installer().await, "## Install", 1);
    let output = run(
        &record.replace("mode=MODE\n", "mode=ask\n"),
        &home,
        root.path(),
    );
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        std::fs::read_to_string(config.join("upgrade")).unwrap(),
        "ask\ndeclined filedrawer-search 4\n"
    );
    let output = run(&record, &home, root.path());
    assert!(!output.status.success(), "an unfilled mode was recorded");
}

/// A skill's `references/upgrade.md`, as the site serves it.
async fn upgrade_steps(name: &str) -> String {
    served(&format!("/skills/{name}/references/upgrade.md")).await
}

/// Step 2 of an upgrade, fetching `version` from the mirror: its output, and the
/// folder it printed.
async fn fetch_upgrade(name: &str, version: &str, home: &Path, mirror: &Path) -> (Output, PathBuf) {
    let fetch = script(&upgrade_steps(name).await, "## 2. Fetch", 0)
        .replace("N=VERSION\n", &format!("N={version}\n"));
    let output = run(&fetch, home, mirror);
    let folder = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    (output, folder)
}

/// Step 5 of an upgrade, putting `new` in place.
async fn put_in_place(name: &str, new: &Path, home: &Path) -> Output {
    let swap = script(&upgrade_steps(name).await, "## 5. Put it in place", 0)
        .replace("new=FOLDER\n", &format!("new={}\n", new.display()));
    run(&swap, home, Path::new("/nowhere"))
}

fn write_old_copy(home: &Path, name: &str) {
    let skill = home.join(".agents/skills").join(name).join("SKILL.md");
    std::fs::write(skill, "old\n").unwrap();
}

fn installed_text(home: &Path, name: &str) -> String {
    std::fs::read_to_string(home.join(".agents/skills").join(name).join("SKILL.md")).unwrap()
}

#[tokio::test]
async fn an_upgrade_puts_the_tags_files_in_place() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    for name in NAMES {
        write_old_copy(&home, name);
        let (output, new) = fetch_upgrade(name, &version(name), &home, &mirror).await;
        assert!(output.status.success(), "{}", text(&output));
        let output = put_in_place(name, &new, &home).await;
        assert!(output.status.success(), "{}", text(&output));
        assert!(
            text(&output).contains(&format!("upgraded {name}")),
            "{}",
            text(&output)
        );
        assert_installed(&home, name).await;
        let skills = home.join(".agents/skills");
        assert!(
            !skills.join(format!(".{name}.lock")).exists(),
            "{name}'s lock is left"
        );
        assert!(
            !skills.join(format!(".{name}.old")).exists(),
            "{name}'s old copy is left"
        );
    }
}

#[tokio::test]
async fn an_upgrade_replaces_a_copy_where_claude_codes_link_would_be() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    let name = "filedrawer-submit";
    let link = home.join(".claude/skills").join(name);
    std::fs::remove_file(&link).unwrap();
    std::fs::create_dir_all(&link).unwrap();
    std::fs::write(link.join("SKILL.md"), "old\n").unwrap();
    let (_, new) = fetch_upgrade(name, &version(name), &home, &mirror).await;
    let output = put_in_place(name, &new, &home).await;
    assert!(output.status.success(), "{}", text(&output));
    assert_installed(&home, name).await;
}

#[tokio::test]
async fn an_upgrade_links_the_skill_for_claude_code_when_its_folder_is_new() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    std::fs::remove_dir_all(home.join(".claude/skills")).unwrap();
    let name = "filedrawer-search";
    let (_, new) = fetch_upgrade(name, &version(name), &home, &mirror).await;
    let output = put_in_place(name, &new, &home).await;
    assert!(output.status.success(), "{}", text(&output));
    assert_installed(&home, name).await;
}

#[tokio::test]
async fn an_upgrade_leaves_the_skill_to_a_session_holding_its_lock() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    let name = "filedrawer-search";
    let lock = home.join(".agents/skills").join(format!(".{name}.lock"));
    std::fs::create_dir(&lock).unwrap();
    write_old_copy(&home, name);
    let (_, new) = fetch_upgrade(name, &version(name), &home, &mirror).await;
    let output = put_in_place(name, &new, &home).await;
    assert!(!output.status.success(), "{}", text(&output));
    assert_eq!(installed_text(&home, name), "old\n");
    assert!(lock.exists(), "the other session's lock was removed");
}

#[tokio::test]
async fn an_upgrade_clears_a_lock_a_dead_session_left() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    let name = "filedrawer-search";
    let lock = home.join(".agents/skills").join(format!(".{name}.lock"));
    std::fs::create_dir(&lock).unwrap();
    let aged = Command::new("touch")
        .args(["-d", "20 minutes ago"])
        .arg(&lock)
        .status()
        .unwrap();
    assert!(aged.success());
    let (_, new) = fetch_upgrade(name, &version(name), &home, &mirror).await;
    let output = put_in_place(name, &new, &home).await;
    assert!(output.status.success(), "{}", text(&output));
    assert_installed(&home, name).await;
}

#[tokio::test]
async fn an_upgrade_left_unfilled_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    let name = "filedrawer-search";
    let (output, _) = fetch_upgrade(name, "VERSION", &home, &mirror).await;
    assert!(!output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("not a version"), "{}", text(&output));
    write_old_copy(&home, name);
    let output = put_in_place(name, Path::new("FOLDER"), &home).await;
    assert!(!output.status.success(), "{}", text(&output));
    assert_eq!(installed_text(&home, name), "old\n");
}

#[tokio::test]
async fn an_upgrade_names_the_url_it_couldnt_fetch() {
    let root = tempfile::tempdir().unwrap();
    let mirror = mirror(root.path());
    let home = installed_home(root.path(), &mirror).await;
    let (output, _) = fetch_upgrade("filedrawer-search", "99", &home, &mirror).await;
    assert!(!output.status.success(), "{}", text(&output));
    assert!(output.stdout.is_empty(), "{}", text(&output));
    let url = format!(
        "couldn't fetch file://{}/refs/tags/skill-filedrawer-search-v99/skills/filedrawer-search/SKILL.md",
        mirror.display()
    );
    assert!(text(&output).contains(&url), "{}", text(&output));
}

#[tokio::test]
async fn an_upgrade_refuses_a_tag_that_names_a_file_outside_its_folder() {
    let root = tempfile::tempdir().unwrap();
    let mirror = tags_mirror(&root.path().join("mirror"), |name, file, text| {
        let listed = "  files: \"SKILL.md references/entry-format.md references/upgrade.md\"";
        if name == "filedrawer-search" && file == "SKILL.md" {
            text.replace(listed, "  files: \"SKILL.md ../../escaped.md\"")
        } else {
            text
        }
    });
    let home = root.path().join("home");
    let name = "filedrawer-search";
    let (output, _) = fetch_upgrade(name, &version(name), &home, &mirror).await;
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("refused path: ../../escaped.md"),
        "{}",
        text(&output)
    );
}

// The submit skill's token commands.

async fn submit_skill() -> String {
    served("/skills/filedrawer-submit/SKILL.md").await
}

fn home_with_header(root: &Path, header: &str) -> (PathBuf, PathBuf) {
    let home = root.join("home");
    let config = home.join(".config/filedrawer");
    std::fs::create_dir_all(&config).unwrap();
    let file = config.join("localhost.header");
    std::fs::write(&file, header).unwrap();
    (home, file)
}

#[tokio::test]
async fn a_cloud_agents_token_goes_into_the_header_file_once() {
    let root = tempfile::tempdir().unwrap();
    let (home, file) = home_with_header(root.path(), "X-Staging-Key: k\n");
    let from_env = command(&submit_skill().await, "### The token", 0);
    let token = [("FILEDRAWER_TOKEN", "fdt_cloud")];
    let output = run_with(&from_env, &home, root.path(), &token, "");
    assert!(output.status.success(), "{}", text(&output));
    let written = "X-Staging-Key: k\nAuthorization: Bearer fdt_cloud\n";
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
    assert!(
        !text(&output).contains("fdt_cloud"),
        "the token was printed"
    );
    run_with(
        &from_env,
        &home,
        root.path(),
        &[("FILEDRAWER_TOKEN", "fdt_other")],
        "",
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
}

#[tokio::test]
async fn with_no_token_set_the_header_file_stays_as_it_was() {
    let root = tempfile::tempdir().unwrap();
    let (home, file) = home_with_header(root.path(), "");
    let from_env = command(&submit_skill().await, "### The token", 0);
    run_with(&from_env, &home, root.path(), &[], "");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
}

#[tokio::test]
async fn the_humans_command_replaces_the_token_line() {
    let root = tempfile::tempdir().unwrap();
    let old = "Authorization: Bearer fdt_old\nX-Staging-Key: k\n";
    let (home, file) = home_with_header(root.path(), old);
    let by_hand = command(&submit_skill().await, "### The token", 1);
    let output = run_with(&by_hand, &home, root.path(), &[], "fdt_new\n");
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "X-Staging-Key: k\nAuthorization: Bearer fdt_new\n"
    );
    assert!(!text(&output).contains("fdt_new"), "the token was printed");
}

#[tokio::test]
async fn the_humans_command_refuses_what_isnt_a_token() {
    let root = tempfile::tempdir().unwrap();
    let old = "Authorization: Bearer fdt_old\n";
    let (home, file) = home_with_header(root.path(), old);
    let by_hand = command(&submit_skill().await, "### The token", 1);
    for typed in ["\n", "hunter2\n"] {
        let output = run_with(&by_hand, &home, root.path(), &[], typed);
        assert!(!output.status.success(), "{typed:?}: {}", text(&output));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), old, "{typed:?}");
    }
}

#[tokio::test]
async fn the_documents_offer_no_embargo_or_hidden_series_while_they_are_paused() {
    // design.md, Embargo (paused, 2026-11-08): the documents name an embargo only to
    // say it is refused, or for entries embargoed before the pause.
    let text = every_document().await;
    for line in text
        .lines()
        .filter(|line| line.to_lowercase().contains("embargo"))
    {
        assert!(
            ["paused", "no release dates", "before 2026-11-08"]
                .iter()
                .any(|allowed| line.contains(allowed)),
            "{line}"
        );
    }
    assert!(!text.contains("waiting_on_series"));
}
