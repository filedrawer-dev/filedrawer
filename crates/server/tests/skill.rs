//! Seam: the router, for `/skill.md` and `/entry-format.md` (design.md, Agent
//! interface), and what `/skill.md` promises: its routes exist, and its example
//! entry passes the analyzer.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{PUBLIC_URL, test_app};

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
    for word in text.split(|c: char| c.is_whitespace() || c == '`') {
        let Some(path) = word.strip_prefix(PUBLIC_URL) else {
            continue;
        };
        let path = path.trim_end_matches(['.', ',', ')', ':']);
        let path = path.replace("<id>", "0000000000");
        if path.is_empty() || path.contains('<') {
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
            let missing = reply.status == StatusCode::NOT_FOUND && !path.starts_with("/e/");
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
