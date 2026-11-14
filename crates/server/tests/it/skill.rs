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
    "/admin/e/{id}/remove",
    "/admin/e/{id}/restore",
    "/admin/accounts/{id}",
    "/admin/accounts/{id}/suspend",
    "/admin/accounts/{id}/lift",
    "/admin/series/{id}",
    "/admin/series/{id}/remove",
    "/admin/series/{id}/restore",
    "/admin/series-refusals/{id}/confirm",
    "/admin/series-refusals/{id}/dismiss",
    "/admin/accounts/{id}/clear-guard",
    "/admin/f/{sha256}",
    "/admin/f/{sha256}/remove",
    "/admin/f/{sha256}/remove-hazard",
    "/entry-format.md",
    "/staging/status",
    "/staging/headers",
    // Its plain text only points an agent without the skills to /skill.md, so the
    // skills needn't name it (CODING_STANDARDS.md, Untrusted text).
    "/",
    "/static/site.css",
    "/static/copy.js",
    "/static/fonts/{name}",
];

/// Each URL on this site that `text` names, as a path without its query, with each
/// `<name>` written as the router writes it, `{name}`. A URL may follow other text in
/// its word, as in `(URL)` or a Markdown link `[text](URL)`; angle brackets at its end
/// are left alone, since a path's `<name>` uses them.
fn named_paths(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '`' || c == '"')
        .filter_map(|word| {
            word.find(PUBLIC_URL)
                .map(|at| &word[at + PUBLIC_URL.len()..])
        })
        .map(|path| path.trim_end_matches(['.', ',', ')', ']', ':', ';']))
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

#[test]
fn named_paths_reads_a_url_in_brackets_parentheses_or_a_markdown_link() {
    let text = format!(
        "one ({PUBLIC_URL}/beta), two [{PUBLIC_URL}/tags], [three]({PUBLIC_URL}/search). \
         And {PUBLIC_URL}/e/<id>."
    );
    assert_eq!(named_paths(&text), ["/beta", "/tags", "/search", "/e/{id}"]);
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
    assert!(text.contains(&format!("{PUBLIC_URL}/install.sh")), "{text}");
    for placeholder in [
        "{{BASE}}",
        "{{HOST}}",
        "{{SKILLS}}",
        "{{TAG_URLS}}",
        "{{CURL}}",
    ] {
        assert!(!text.contains(placeholder), "{placeholder} is left");
    }
}

#[tokio::test]
async fn skill_md_says_the_archive_is_in_beta_and_problems_go_to_feedback() {
    let text = served("/skill.md").await;
    let line = text
        .lines()
        .find(|line| line.contains(&format!("{PUBLIC_URL}/beta")))
        .unwrap_or_else(|| panic!("no line names /beta: {text}"));
    assert!(
        line.contains(&format!("POST {PUBLIC_URL}/feedback")),
        "{line}"
    );
    assert!(!line.to_lowercase().contains("github"), "{line}");
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

/// A test app whose site is at `public_url`.
async fn app_at(public_url: &str) -> common::TestApp {
    let mut app = test_app().await;
    app.state.public_url = public_url.to_string();
    app.router = filedrawer_server::app(app.state.clone());
    app
}

const PRODUCTION: &str = "https://filedrawer.dev";
const STAGING: &str = "https://staging.filedrawer.dev";

#[tokio::test]
async fn production_serves_each_skills_files_as_the_repo_holds_them() {
    let app = app_at(PRODUCTION).await;
    for name in NAMES {
        assert!(files(name).len() >= 3, "{name}");
        for file in files(name) {
            let reply = app.get(&format!("/skills/{name}/{file}")).await;
            assert_eq!(reply.status, StatusCode::OK, "{name}/{file}");
            assert_eq!(reply.body, raw(name, &file), "{name}/{file}");
        }
    }
}

#[tokio::test]
async fn staging_serves_each_skills_files_pointed_at_itself_once() {
    let app = app_at(STAGING).await;
    for name in NAMES {
        for file in files(name) {
            let text = app.get(&format!("/skills/{name}/{file}")).await.body;
            let source = raw(name, &file);
            assert!(!text.contains("https://filedrawer.dev"), "{name}/{file}");
            assert!(!text.contains("staging.staging"), "{name}/{file}");
            assert_eq!(
                text.matches(STAGING).count(),
                source.matches(PRODUCTION).count(),
                "{name}/{file}"
            );
            for kept in [
                "github.com/filedrawer-dev/filedrawer",
                "~/.config/filedrawer/",
                "filedrawer.dev counts as its ID",
            ] {
                assert_eq!(
                    text.matches(kept).count(),
                    source.matches(kept).count(),
                    "{name}/{file}: {kept}"
                );
            }
        }
        let skill = app.get(&format!("/skills/{name}/SKILL.md")).await.body;
        assert!(
            skill.contains(&format!(
                "-H @\"$(ls ~/.config/filedrawer/staging.filedrawer.dev.header 2>/dev/null || echo /dev/null)\" -H \"X-Filedrawer-Skill: {name}/{}\"",
                version(name)
            )),
            "{name}"
        );
        assert!(
            skill.contains("https://staging.filedrawer.dev/skill.md | tr -d"),
            "{name}"
        );
    }
}

#[tokio::test]
async fn each_skills_files_are_served_for_this_site() {
    for name in NAMES {
        for file in files(name) {
            let text = served(&format!("/skills/{name}/{file}")).await;
            assert!(!text.contains(PRODUCTION), "{name}/{file}");
            assert!(!text.contains("{{"), "{name}/{file}");
        }
    }
    let search = served("/skills/filedrawer-search/SKILL.md").await;
    assert!(
        search.contains("~/.config/filedrawer/localhost.header"),
        "{search}"
    );
    assert!(search.contains("\"http://localhost/search?q="), "{search}");
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
    let mut child = shell(&script, home)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

/// `sh -c script` with `home` as HOME and no token in the environment.
fn shell(script: &str, home: &Path) -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(script)
        .env("HOME", home)
        .env_remove("FILEDRAWER_TOKEN");
    command
}

fn run(script: &str, home: &Path, mirror: &Path) -> Output {
    run_with(script, home, mirror, &[], "")
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr)
}

/// A site and GitHub for the installer to reach: the test app at `public_url`, served
/// on loopback, and a folder laid out as GitHub serves the tags. A `curl` first on PATH
/// sends requests for `public_url` to the app and those for GitHub to the folder, and
/// logs each call's arguments.
struct StandIn {
    app: common::TestApp,
    root: tempfile::TempDir,
    mirror: PathBuf,
    log: PathBuf,
    path: String,
}

impl StandIn {
    async fn at(public_url: &str) -> StandIn {
        Self::with_tags(public_url, |_, _, text| text).await
    }

    /// With `edit` applied to each tagged file's text, given its skill and path.
    async fn with_tags(public_url: &str, edit: impl Fn(&str, &str, String) -> String) -> StandIn {
        let app = app_at(public_url).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let router = app.router.clone();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let root = tempfile::tempdir().unwrap();
        let mirror = tags_mirror(&root.path().join("mirror"), edit);
        let log = root.path().join("curl.log");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let shim = format!(
            "#!/bin/sh\n\
             for a do\n\
             \x20 case $a in\n\
             \x20   {public_url}*) a=\"http://127.0.0.1:{port}${{a#{public_url}}}\" ;;\n\
             \x20   {RAW_GITHUB}*) a=\"file://{mirror}${{a#{RAW_GITHUB}}}\" ;;\n\
             \x20 esac\n\
             \x20 set -- \"$@\" \"$a\"\n\
             \x20 shift\n\
             done\n\
             printf '%s\\n' \"$*\" >> '{log}'\n\
             exec '{curl}' \"$@\"\n",
            mirror = mirror.display(),
            log = log.display(),
            curl = real_curl().display(),
        );
        std::fs::write(bin.join("curl"), shim).unwrap();
        std::fs::set_permissions(
            bin.join("curl"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        StandIn {
            app,
            root,
            mirror,
            log,
            path,
        }
    }

    async fn get(&self, path: &str) -> common::Reply {
        self.app.get(path).await
    }

    /// A home folder, with Claude Code's folder in it.
    fn home(&self) -> PathBuf {
        let home = self.root.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        home
    }

    /// `sh -c script` with `home` as HOME and the stand-in curl first on PATH.
    fn run(&self, script: &str, home: &Path) -> Output {
        shell(script, home)
            .env("PATH", &self.path)
            .output()
            .unwrap()
    }

    /// The installer as the site serves it, run with `args`.
    async fn install(&self, home: &Path, args: &str) -> Output {
        let file = self.root.path().join("install.sh");
        std::fs::write(&file, self.get("/install.sh").await.body).unwrap();
        self.run(&format!("sh '{}' {args}", file.display()), home)
    }

    /// Each curl call so far, its arguments joined by spaces.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// The curl on PATH, which the stand-in hands each call to.
fn real_curl() -> PathBuf {
    std::env::split_paths(&std::env::var("PATH").unwrap())
        .map(|dir| dir.join("curl"))
        .find(|path| path.is_file())
        .expect("curl on PATH")
}

/// Asserts each skill installed under `home` holds what `site` serves, and Claude
/// Code's folder links to it.
async fn assert_installed(site: &StandIn, home: &Path, name: &str) {
    let installed = home.join(".agents/skills").join(name);
    for file in files(name) {
        assert_eq!(
            std::fs::read_to_string(installed.join(&file)).unwrap(),
            site.get(&format!("/skills/{name}/{file}")).await.body,
            "{name}/{file}"
        );
    }
    let link = home.join(".claude/skills").join(name);
    assert_eq!(std::fs::read_link(&link).unwrap(), installed, "{name}");
}

/// Asserts the installer failed, said `why`, and left `home` without the skills or a
/// recorded mode.
fn assert_installed_nothing(output: &Output, home: &Path, why: &str) {
    let said = text(output);
    assert!(!output.status.success(), "{said}");
    assert!(said.contains(why), "{said}");
    assert!(said.contains("install nothing by other means"), "{said}");
    for name in NAMES {
        assert!(!home.join(".agents/skills").join(name).exists(), "{name}");
    }
    assert!(!home.join(".config/filedrawer/upgrade").exists());
}

fn tag_url(name: &str) -> String {
    format!(
        "https://github.com/filedrawer-dev/filedrawer/tree/skill-{name}-v{}/skills/{name}",
        version(name)
    )
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[tokio::test(flavor = "multi_thread")]
async fn on_production_the_installer_installs_each_tags_folder_byte_for_byte() {
    let site = StandIn::at(PRODUCTION).await;
    let home = site.home();
    let output = site.install(&home, "auto").await;
    assert!(output.status.success(), "{}", text(&output));
    for name in NAMES {
        let installed = home.join(".agents/skills").join(name);
        for file in files(name) {
            assert_eq!(
                read(&installed.join(&file)),
                raw(name, &file),
                "{name}/{file}"
            );
        }
        assert_eq!(
            std::fs::read_link(home.join(".claude/skills").join(name)).unwrap(),
            installed
        );
    }
    let header = home.join(".config/filedrawer/filedrawer.dev.header");
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(header).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(read(&home.join(".config/filedrawer/upgrade")), "auto\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn elsewhere_the_installer_points_the_skills_at_its_site_once() {
    let site = StandIn::at(STAGING).await;
    let home = site.home();
    let output = site.install(&home, "auto").await;
    assert!(output.status.success(), "{}", text(&output));
    for name in NAMES {
        assert_installed(&site, &home, name).await;
        let skill = read(&home.join(".agents/skills").join(name).join("SKILL.md"));
        assert!(!skill.contains(PRODUCTION), "{name}");
        assert!(
            skill.contains("~/.config/filedrawer/staging.filedrawer.dev.header"),
            "{name}"
        );
    }
    assert!(
        home.join(".config/filedrawer/staging.filedrawer.dev.header")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_asks_the_site_with_the_header_file_when_there_is_one() {
    let site = StandIn::at(STAGING).await;
    let home = site.home();
    let output = site.install(&home, "auto").await;
    assert!(output.status.success(), "{}", text(&output));
    let header = home.join(".config/filedrawer/staging.filedrawer.dev.header");
    let asked = |calls: &[String]| {
        calls
            .iter()
            .filter(|call| call.contains("/skill.md"))
            .cloned()
            .collect::<Vec<_>>()
    };
    // The first run found no header file, and made one.
    assert!(
        asked(&site.calls())[0].contains("-H @/dev/null"),
        "{:?}",
        site.calls()
    );
    std::fs::write(&header, "X-Staging-Key: k\n").unwrap();
    let output = site.install(&home, "auto").await;
    assert!(output.status.success(), "{}", text(&output));
    let calls = asked(&site.calls());
    assert!(
        calls[1].contains(&format!("-H @{}", header.display())),
        "{calls:?}"
    );
    // Requests to GitHub carry nothing of the site's.
    for call in site.calls() {
        if call.contains("file://") {
            assert!(!call.contains(".header"), "{call}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_tells_the_agent_what_it_installed_and_how_to_switch_the_mode() {
    let site = StandIn::at(PRODUCTION).await;
    let home = site.home();
    let config = home.join(".config/filedrawer");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("upgrade"),
        "auto\ndeclined filedrawer-search 4\n",
    )
    .unwrap();
    let output = site.install(&home, "ask").await;
    assert!(output.status.success(), "{}", text(&output));
    let said = String::from_utf8(output.stdout).unwrap();
    for name in NAMES {
        assert!(said.contains(&tag_url(name)), "{said}");
        let folder = home.join(".agents/skills").join(name);
        assert!(said.contains(&format!("{}/", folder.display())), "{said}");
    }
    assert!(
        said.contains(&format!(
            "{}/<name>/SKILL.md",
            home.join(".agents/skills").display()
        )),
        "{said}"
    );
    assert_eq!(
        read(&config.join("upgrade")),
        "ask\ndeclined filedrawer-search 4\n"
    );
    // The command it gives switches the mode and keeps what was declined.
    let switch = said
        .lines()
        .skip_while(|line| !line.contains("To switch to auto"))
        .nth(1)
        .unwrap_or_else(|| panic!("no command to switch: {said}"));
    let output = site.run(switch.trim(), &home);
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        read(&config.join("upgrade")),
        "auto\ndeclined filedrawer-search 4\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_installs_the_same_again() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = site.home();
    for _ in 0..2 {
        let output = site.install(&home, "auto").await;
        assert!(output.status.success(), "{}", text(&output));
    }
    for name in NAMES {
        assert_installed(&site, &home, name).await;
    }
    let left: Vec<_> = std::fs::read_dir(home.join(".agents/skills"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left.len(), 2, "{left:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_or_missing_mode_fails_before_anything_is_fetched() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = site.home();
    for args in ["maybe", "", "auto ask", "MODE"] {
        let output = site.install(&home, args).await;
        assert_installed_nothing(&output, &home, "auto or ask");
        assert_eq!(site.calls(), Vec::<String>::new(), "{args:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_installs_neither_skill_when_one_tag_is_missing() {
    let site = StandIn::at(PUBLIC_URL).await;
    let submit = format!(
        "refs/tags/skill-filedrawer-submit-v{}",
        version("filedrawer-submit")
    );
    std::fs::remove_dir_all(site.mirror.join(&submit)).unwrap();
    let home = site.home();
    let output = site.install(&home, "auto").await;
    let url = format!("couldn't fetch {RAW_GITHUB}/{submit}");
    assert_installed_nothing(&output, &home, &url);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_installs_neither_skill_when_a_tag_holds_another_version() {
    let site = StandIn::with_tags(PUBLIC_URL, |name, file, text| {
        if name == "filedrawer-submit" && file == "SKILL.md" {
            let n = version(name);
            text.replace(&format!("  version: \"{n}\""), "  version: \"999\"")
        } else {
            text
        }
    })
    .await;
    let home = site.home();
    let output = site.install(&home, "auto").await;
    assert_installed_nothing(&output, &home, "isn't filedrawer-submit version");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_refuses_a_tag_that_names_a_file_outside_its_folder() {
    let site = StandIn::with_tags(PUBLIC_URL, |name, file, text| {
        let listed = "  files: \"SKILL.md references/entry-format.md references/upgrade.md\"";
        if name == "filedrawer-submit" && file == "SKILL.md" {
            text.replace(listed, "  files: \"SKILL.md ../../escaped.md\"")
        } else {
            text
        }
    })
    .await;
    let home = site.home();
    let output = site.install(&home, "auto").await;
    assert_installed_nothing(
        &output,
        &home,
        "refused path in filedrawer-submit's files: ../../escaped.md",
    );
    assert!(!site.root.path().join("escaped.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_installs_nothing_when_agents_folder_is_a_file() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = site.home();
    std::fs::write(home.join(".agents"), "").unwrap();
    let output = site.install(&home, "auto").await;
    let said = text(&output);
    assert!(!output.status.success(), "{said}");
    assert!(said.contains("install nothing by other means"), "{said}");
    assert!(home.join(".agents").is_file());
    assert!(!home.join(".config/filedrawer/upgrade").exists());
    assert!(!home.join(".claude/skills").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_installs_nothing_when_claude_codes_skills_folder_is_a_file() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = site.home();
    std::fs::write(home.join(".claude/skills"), "").unwrap();
    let output = site.install(&home, "auto").await;
    assert_installed_nothing(&output, &home, "couldn't");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_rerun_leaves_the_install_and_the_mode_as_they_were() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = site.home();
    let output = site.install(&home, "ask").await;
    assert!(output.status.success(), "{}", text(&output));
    std::fs::remove_dir_all(site.mirror.join("refs")).unwrap();
    let output = site.install(&home, "auto").await;
    assert!(!output.status.success(), "{}", text(&output));
    for name in NAMES {
        assert_installed(&site, &home, name).await;
    }
    assert_eq!(read(&home.join(".config/filedrawer/upgrade")), "ask\n");
}

// A rerun over skills already installed (the owner's rulings of 2026-11-13, #78).

/// The host of a site's address, as the installer names its header file.
fn host(base: &str) -> &str {
    base.split_once("://").map_or(base, |(_, rest)| rest)
}

/// `name`'s current files as an install from the site at `base` holds them.
fn installed_from(name: &str, base: &str) -> Vec<(String, String)> {
    files(name)
        .into_iter()
        .map(|file| {
            let text = raw(name, &file)
                .replace(PRODUCTION, base)
                .replace("/filedrawer.dev.header", &format!("/{}.header", host(base)));
            (file, text)
        })
        .collect()
}

/// `name`'s current files written as an old tag would hold them, with placeholders,
/// at `version`.
fn old_files(name: &str, version: &str) -> Vec<(String, String)> {
    files(name)
        .into_iter()
        .map(|file| {
            let text = raw(name, &file)
                .replace(PRODUCTION, "{{BASE}}")
                .replace("/filedrawer.dev.header", "/{{HOST}}.header")
                .replace(
                    &format!("  version: \"{}\"", self::version(name)),
                    &format!("  version: \"{version}\""),
                );
            (file, text)
        })
        .collect()
}

/// Writes `files` as the tag `skill-NAME-vVERSION` in `mirror`.
fn add_tag(mirror: &Path, name: &str, version: &str, files: &[(String, String)]) {
    let tag = mirror.join(format!("refs/tags/skill-{name}-v{version}/skills/{name}"));
    lay_out(&tag, files);
}

/// Writes `files` into `folder`.
fn lay_out(folder: &Path, files: &[(String, String)]) {
    for (file, text) in files {
        let path = folder.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn skill_folder(home: &Path, name: &str) -> PathBuf {
    home.join(".agents/skills").join(name)
}

/// Every file under `dir` with its bytes, and every link with its target.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_symlink() {
                let target = std::fs::read_link(&path).unwrap();
                out.push((path, target.into_os_string().into_encoded_bytes()));
            } else if kind.is_dir() {
                out.push((path.clone(), Vec::new()));
                stack.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Asserts a rerun refused for `name`, saying `why`, and changed nothing in `home`.
async fn assert_refused(site: &StandIn, home: &Path, name: &str, why: &str) {
    let before = snapshot(home);
    let output = site.install(home, "ask").await;
    let said = text(&output);
    assert!(!output.status.success(), "{said}");
    assert!(said.contains(name), "{said}");
    assert!(said.contains(why), "{why}: {said}");
    assert!(said.contains("install nothing by other means"), "{said}");
    assert_eq!(snapshot(home), before, "{said}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_installs_over_unchanged_installs_from_the_old_installer() {
    for base in [PRODUCTION, STAGING] {
        let site = StandIn::at(base).await;
        let home = site.home();
        for name in NAMES {
            let old = old_files(name, "3");
            add_tag(&site.mirror, name, "3", &old);
            let filled: Vec<(String, String)> = old
                .into_iter()
                .map(|(file, text)| {
                    let text = text
                        .replace("{{BASE}}", base)
                        .replace("{{HOST}}", host(base));
                    (file, text)
                })
                .collect();
            lay_out(&skill_folder(&home, name), &filled);
        }
        let output = site.install(&home, "auto").await;
        assert!(output.status.success(), "{base}: {}", text(&output));
        for name in NAMES {
            assert_installed(&site, &home, name).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_on_staging_replaces_skills_an_old_upgrade_left_naming_production() {
    let site = StandIn::at(STAGING).await;
    let home = site.home();
    for name in NAMES {
        lay_out(
            &skill_folder(&home, name),
            &installed_from(name, PRODUCTION),
        );
    }
    let output = site.install(&home, "auto").await;
    assert!(output.status.success(), "{}", text(&output));
    for name in NAMES {
        assert_installed(&site, &home, name).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_installs_over_an_unchanged_install_from_another_site() {
    let site = StandIn::at(PRODUCTION).await;
    let home = site.home();
    for name in NAMES {
        lay_out(&skill_folder(&home, name), &installed_from(name, STAGING));
    }
    let output = site.install(&home, "auto").await;
    assert!(output.status.success(), "{}", text(&output));
    for name in NAMES {
        assert_installed(&site, &home, name).await;
    }
}

/// A home where the installer ran once, in `ask` mode.
async fn reinstall_home(site: &StandIn) -> PathBuf {
    let home = site.home();
    let output = site.install(&home, "ask").await;
    assert!(output.status.success(), "{}", text(&output));
    home
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_refuses_a_changed_file_and_writes_nothing() {
    let site = StandIn::at(STAGING).await;
    let home = reinstall_home(&site).await;
    let skill = skill_folder(&home, "filedrawer-submit").join("references/upgrade.md");
    std::fs::write(&skill, read(&skill) + "my note\n").unwrap();
    std::fs::write(home.join(".config/filedrawer/upgrade"), "auto\n").unwrap();
    assert_refused(&site, &home, "filedrawer-submit", "references/upgrade.md").await;
    let said = text(&site.install(&home, "auto").await);
    assert!(said.contains("move or remove"), "{said}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_refuses_an_extra_or_a_missing_file() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = reinstall_home(&site).await;
    std::fs::write(
        skill_folder(&home, "filedrawer-search").join("notes.md"),
        "mine",
    )
    .unwrap();
    assert_refused(&site, &home, "filedrawer-search", "notes.md").await;
    std::fs::remove_file(skill_folder(&home, "filedrawer-search").join("notes.md")).unwrap();
    std::fs::remove_file(
        skill_folder(&home, "filedrawer-search").join("references/entry-format.md"),
    )
    .unwrap();
    assert_refused(
        &site,
        &home,
        "filedrawer-search",
        "references/entry-format.md",
    )
    .await;
    std::fs::write(
        skill_folder(&home, "filedrawer-search").join("references/entry-format.md"),
        raw("filedrawer-search", "references/entry-format.md"),
    )
    .unwrap();
    // A name that word splitting would read as listed files is still extra.
    for odd in ["SKILL.md SKILL.md", "SKILL.md\nSKILL.md", "*"] {
        let file = skill_folder(&home, "filedrawer-search").join(odd);
        std::fs::write(&file, "mine").unwrap();
        assert_refused(&site, &home, "filedrawer-search", "isn't in version").await;
        std::fs::remove_file(&file).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_ignores_finders_ds_store() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = reinstall_home(&site).await;
    std::fs::write(
        skill_folder(&home, "filedrawer-search").join(".DS_Store"),
        "x",
    )
    .unwrap();
    let output = site.install(&home, "auto").await;
    assert!(output.status.success(), "{}", text(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_refuses_a_folder_with_no_version_it_can_check() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = reinstall_home(&site).await;
    let skill = skill_folder(&home, "filedrawer-search").join("SKILL.md");
    let text_now = read(&skill);
    let n = version("filedrawer-search");
    std::fs::write(
        &skill,
        text_now.replace(&format!("  version: \"{n}\""), "  version: x"),
    )
    .unwrap();
    assert_refused(&site, &home, "filedrawer-search", "names no version").await;
    // A version with no tag can't be checked either.
    std::fs::write(
        &skill,
        text_now.replace(&format!("  version: \"{n}\""), "  version: \"99\""),
    )
    .unwrap();
    assert_refused(
        &site,
        &home,
        "filedrawer-search",
        "skill-filedrawer-search-v99",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_refuses_a_link_where_the_folder_goes() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = reinstall_home(&site).await;
    let folder = skill_folder(&home, "filedrawer-search");
    let elsewhere = home.join("dotfiles/filedrawer-search");
    std::fs::create_dir_all(home.join("dotfiles")).unwrap();
    std::fs::rename(&folder, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &folder).unwrap();
    assert_refused(&site, &home, "filedrawer-search", "is a link").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_installs_a_version_declined_before_and_keeps_the_line() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = site.home();
    let config = home.join(".config/filedrawer");
    std::fs::create_dir_all(&config).unwrap();
    let declined = format!(
        "ask\ndeclined filedrawer-search {}\n",
        version("filedrawer-search")
    );
    std::fs::write(config.join("upgrade"), &declined).unwrap();
    let output = site.install(&home, "ask").await;
    assert!(output.status.success(), "{}", text(&output));
    assert_installed(&site, &home, "filedrawer-search").await;
    assert_eq!(read(&config.join("upgrade")), declined);
}

#[tokio::test]
async fn production_serves_the_installer_byte_for_byte_as_plain_text() {
    let app = app_at(PRODUCTION).await;
    let script = include_str!("../../install.sh");
    for page in [false, true] {
        let mut request = Request::builder().uri("/install.sh");
        if page {
            request = request
                .header("sec-fetch-dest", "document")
                .header("sec-fetch-mode", "navigate");
        }
        let reply = app.send(request.body(Body::empty()).unwrap()).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.headers["content-type"], "text/plain; charset=utf-8");
        assert!(reply.headers.contains_key("x-filedrawer-commit"));
        assert_eq!(reply.body, script);
    }
}

#[tokio::test]
async fn staging_serves_the_installer_with_its_own_address_alone() {
    let app = app_at(STAGING).await;
    let script = include_str!("../../install.sh");
    let reply = app.get("/install.sh").await;
    let changed: Vec<(&str, &str)> = script
        .lines()
        .zip(reply.body.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        changed,
        [(
            "  base=https://filedrawer.dev",
            "  base=https://staging.filedrawer.dev"
        )]
    );
    assert_eq!(script.lines().count(), reply.body.lines().count());
}

/// `/skill.md`'s two install commands at `site`: the download, then the run.
async fn page_commands(site: &StandIn) -> (String, String) {
    let page = site.get("/skill.md").await.body;
    (
        command(&page, "### 2. Download", 0),
        command(&page, "### 3. Run", 0),
    )
}

/// A home where the agent ran `/skill.md`'s two commands, with Claude Code's folder
/// present.
async fn installed_home(site: &StandIn) -> PathBuf {
    let home = site.home();
    let (download, install) = page_commands(site).await;
    for command in [download, install] {
        let output = site.run(&command, &home);
        assert!(output.status.success(), "{command}: {}", text(&output));
    }
    home
}

#[tokio::test(flavor = "multi_thread")]
async fn skill_mds_two_commands_download_the_installer_then_install() {
    for public_url in [PRODUCTION, STAGING] {
        let site = StandIn::at(public_url).await;
        let home = site.home();
        let (download, install) = page_commands(&site).await;
        let output = site.run(&download, &home);
        assert!(output.status.success(), "{}", text(&output));
        // Downloaded, to read before the next command, and nothing installed yet.
        assert_eq!(
            read(&home.join(".config/filedrawer/install.sh")),
            site.get("/install.sh").await.body
        );
        assert!(!home.join(".agents").exists());
        assert_eq!(install, "sh ~/.config/filedrawer/install.sh auto");
        let output = site.run(&install, &home);
        assert!(output.status.success(), "{}", text(&output));
        for name in NAMES {
            assert_installed(&site, &home, name).await;
        }
    }
}

#[tokio::test]
async fn skill_md_downloads_from_production_with_a_plain_curl() {
    let page = app_at(PRODUCTION).await.get("/skill.md").await.body;
    assert_eq!(
        command(&page, "### 2. Download", 0),
        "curl -fsSL --create-dirs -o ~/.config/filedrawer/install.sh https://filedrawer.dev/install.sh"
    );
}

#[tokio::test]
async fn skill_md_never_pipes_or_chains_a_download_into_a_shell() {
    for public_url in [PRODUCTION, STAGING] {
        let page = app_at(public_url).await.get("/skill.md").await.body;
        for piped in page.split('|').skip(1) {
            let program = piped.split_whitespace().next().unwrap_or_default();
            let program = program.trim_end_matches(['`', ')', '.']);
            assert!(
                !["sh", "bash", "zsh", "dash"].contains(&program),
                "{public_url}: a pipe into {program}"
            );
        }
        for line in page.lines().filter(|line| line.contains("curl ")) {
            for chain in ["&&", ";", "||", "$(curl"] {
                let after_url = line.split("/install.sh").nth(1).unwrap_or_default();
                assert!(!after_url.contains(chain), "{public_url}: {line}");
            }
        }
        assert!(
            !page.contains("sh -eu <<"),
            "{public_url}: a script in the page"
        );
    }
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

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_puts_the_tags_files_in_place() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = installed_home(&site).await;
    let mirror = site.mirror.clone();
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
        assert_installed(&site, &home, name).await;
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

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_replaces_a_copy_where_claude_codes_link_would_be() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = installed_home(&site).await;
    let mirror = site.mirror.clone();
    let name = "filedrawer-submit";
    let link = home.join(".claude/skills").join(name);
    std::fs::remove_file(&link).unwrap();
    std::fs::create_dir_all(&link).unwrap();
    std::fs::write(link.join("SKILL.md"), "old\n").unwrap();
    let (_, new) = fetch_upgrade(name, &version(name), &home, &mirror).await;
    let output = put_in_place(name, &new, &home).await;
    assert!(output.status.success(), "{}", text(&output));
    assert_installed(&site, &home, name).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_links_the_skill_for_claude_code_when_its_folder_is_new() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = installed_home(&site).await;
    let mirror = site.mirror.clone();
    std::fs::remove_dir_all(home.join(".claude/skills")).unwrap();
    let name = "filedrawer-search";
    let (_, new) = fetch_upgrade(name, &version(name), &home, &mirror).await;
    let output = put_in_place(name, &new, &home).await;
    assert!(output.status.success(), "{}", text(&output));
    assert_installed(&site, &home, name).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_leaves_the_skill_to_a_session_holding_its_lock() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = installed_home(&site).await;
    let mirror = site.mirror.clone();
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

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_clears_a_lock_a_dead_session_left() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = installed_home(&site).await;
    let mirror = site.mirror.clone();
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
    assert_installed(&site, &home, name).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_left_unfilled_changes_nothing() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = installed_home(&site).await;
    let mirror = site.mirror.clone();
    let name = "filedrawer-search";
    let (output, _) = fetch_upgrade(name, "VERSION", &home, &mirror).await;
    assert!(!output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("not a version"), "{}", text(&output));
    write_old_copy(&home, name);
    let output = put_in_place(name, Path::new("FOLDER"), &home).await;
    assert!(!output.status.success(), "{}", text(&output));
    assert_eq!(installed_text(&home, name), "old\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_names_the_url_it_couldnt_fetch() {
    let site = StandIn::at(PUBLIC_URL).await;
    let home = installed_home(&site).await;
    let mirror = site.mirror.clone();
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
async fn a_token_pasted_into_chat_goes_through_the_humans_command() {
    let root = tempfile::tempdir().unwrap();
    let (home, file) = home_with_header(root.path(), "X-Staging-Key: k\n");
    let skill = submit_skill().await;
    let pasted = command(&skill, "### The token", 2);
    let by_hand = command(&skill, "### The token", 1);
    assert!(pasted.ends_with(&format!("| {by_hand}")), "{pasted}");
    let pasted = pasted.replace("'TOKEN'", "'fdt_pasted'");
    let output = run_with(&pasted, &home, root.path(), &[], "");
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "X-Staging-Key: k\nAuthorization: Bearer fdt_pasted\n"
    );
    assert!(
        !text(&output).contains("fdt_pasted"),
        "the token was printed"
    );
}

/// Runs `script` in `dir` with `home` as HOME.
fn run_in(script: &str, dir: &Path, home: &Path) -> Output {
    shell(script, home).current_dir(dir).output().unwrap()
}

#[tokio::test]
async fn drafts_stay_out_of_the_repo_and_the_home_folder() {
    let root = tempfile::tempdir().unwrap();
    let (repo, home) = (root.path().join("repo"), root.path().join("home"));
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let git = |args: &[&str]| {
        let output = run_in(&format!("git {}", args.join(" ")), &repo, &home);
        assert!(output.status.success(), "{}", text(&output));
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    let drafts = command(&submit_skill().await, "### Drafts", 0);
    let output = run_in(&drafts, &repo.join("src"), &home);
    assert!(output.status.success(), "{}", text(&output));
    let folder = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert_eq!(folder, repo.join(".filedrawer/drafts"));
    std::fs::write(folder.join("entry.md"), "a draft").unwrap();
    assert_eq!(git(&["status", "--porcelain", "--ignored=no"]), "");
    assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
}

async fn search_skill() -> String {
    served("/skills/filedrawer-search/SKILL.md").await
}

/// The search skill's example search, sent to a file that stands in for the site.
async fn search_a_file(root: &Path) -> String {
    let site = root.join("site");
    std::fs::create_dir_all(&site).unwrap();
    std::fs::write(site.join("search"), "cards\n").unwrap();
    let skill = search_skill().await;
    let search = skill
        .lines()
        .find(|line| line.starts_with("curl ") && line.contains("/search?q="))
        .expect("an example search");
    search.replace(PUBLIC_URL, &format!("file://{}", site.display()))
}

#[tokio::test]
async fn searching_needs_no_header_file() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let output = run(&search_a_file(root.path()).await, &home, root.path());
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(text(&output), "cards\n");
    assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
}

#[tokio::test]
async fn searching_sends_the_header_file_when_there_is_one() {
    // curl refuses a header file it can't read, so a refusal shows it read this one; a
    // folder in its place can't be read even as root.
    let root = tempfile::tempdir().unwrap();
    let (home, file) = home_with_header(root.path(), "X-Staging-Key: k\n");
    let search = search_a_file(root.path()).await;
    assert!(run(&search, &home, root.path()).status.success());
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let output = run(&search, &home, root.path());
    assert!(!output.status.success(), "{}", text(&output));
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

#[tokio::test]
async fn skill_md_names_the_commit_the_binary_was_built_from() {
    let app = test_app().await;
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
        // What the build was given (CI's FILEDRAWER_COMMIT), read here rather than
        // through the server's constant.
        let built_from = option_env!("FILEDRAWER_COMMIT")
            .filter(|commit| !commit.is_empty())
            .unwrap_or("unknown");
        assert_eq!(reply.headers["x-filedrawer-commit"], built_from, "{method}");
    }
}
