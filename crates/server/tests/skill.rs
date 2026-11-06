//! Seam: the router, for `/skill.md` and `/entry-format.md` (design.md, Agent
//! interface), and what `/skill.md` promises: its routes exist, it names every route
//! meant for agents, and its example entry passes the analyzer.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{PUBLIC_URL, test_app};
use filedrawer_server::route_paths;

/// Routes /skill.md needn't name: the account pages a person uses in a browser, and
/// /skill.md itself.
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
    "/admin",
    "/admin/e/{id}",
    "/admin/e/{id}/publish",
    "/admin/e/{id}/hold",
    "/admin/e/{id}/reject",
    "/admin/e/{id}/doubt-seen",
    "/admin/accounts/{id}/clear-guard",
    "/skill.md",
];

/// Each URL on this site that `text` names, as a path without its query, with each
/// `<name>` written as the router writes it, `{name}`.
fn named_paths(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '`')
        .filter_map(|word| word.strip_prefix(PUBLIC_URL))
        .map(|path| path.trim_end_matches(['.', ',', ')', ':']))
        .map(|path| path.split(['?', '#']).next().unwrap_or(path))
        .filter(|path| !path.is_empty())
        .map(|path| path.replace('<', "{").replace('>', "}"))
        .collect()
}

async fn skill() -> String {
    let app = test_app().await;
    let reply = app.get("/skill.md").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.headers["content-type"],
        "text/markdown; charset=utf-8"
    );
    reply.body
}

#[tokio::test]
async fn skill_md_names_this_sites_address() {
    let text = skill().await;
    assert!(text.contains(&format!("{PUBLIC_URL}/search?q=")), "{text}");
    assert!(!text.contains("{{"), "a placeholder is left");
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
    assert_eq!(reply.body, include_str!("../../../docs/entry-format.md"));
}

#[tokio::test]
async fn every_route_skill_md_names_exists() {
    let text = skill().await;
    let app = test_app().await;
    let mut seen = 0;
    for path in named_paths(&text) {
        let path = path.replace("{id}", "0000000000");
        if path.contains('<') {
            continue;
        }
        seen += 1;
        for method in ["GET", "POST"] {
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
            assert_eq!(method, "GET", "{path} answers neither GET nor POST");
        }
    }
    assert!(seen >= 6, "skill.md names only {seen} URLs");
}

#[tokio::test]
async fn skill_md_names_every_route_meant_for_agents() {
    let named = named_paths(&skill().await);
    let routes = route_paths(test_app().await.state);
    assert!(routes.len() >= 20, "the router lists only {routes:?}");
    for route in &routes {
        let listed = NOT_FOR_AGENTS.contains(route);
        let is_named = named.iter().any(|path| path == route);
        assert!(
            is_named || listed,
            "/skill.md doesn't name {route}; name it, or list it in NOT_FOR_AGENTS"
        );
        assert!(
            !(is_named && listed),
            "/skill.md names {route}, so take it off NOT_FOR_AGENTS"
        );
    }
    for listed in NOT_FOR_AGENTS {
        assert!(routes.contains(listed), "{listed} is not a route");
    }
}

#[tokio::test]
async fn the_example_entry_passes_the_analyzer_cleanly() {
    let text = skill().await;
    let fence = "````markdown\n";
    let start = text.find(fence).expect("skill.md has an example entry") + fence.len();
    let end = start + text[start..].find("\n````").unwrap();
    let analysis = analyzer::analyze(&text[start..end]);
    assert_eq!(analysis.refusals, vec![]);
    assert_eq!(analysis.warnings, vec![]);
}

#[tokio::test]
async fn the_example_entry_is_entry_format_mds_example() {
    let fence = "````markdown\n";
    let example = |text: &str| -> String {
        let start = text.find(fence).expect("an example entry") + fence.len();
        let end = start + text[start..].find("\n````").unwrap();
        text[start..end].to_string()
    };
    let format = include_str!("../../../docs/entry-format.md");
    assert_eq!(example(&skill().await), example(format));
}
