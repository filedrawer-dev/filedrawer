//! Seam: the router, with a request log in the test's directory, for staging's record
//! of the headers clients send and `GET /staging/headers`, which groups them by
//! client (#28). Also `RequestLog::for_environment`, for which servers keep one.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{DELIMITER, TestApp, test_app};
use filedrawer_server::config::Environment;
use filedrawer_server::request_log::RequestLog;
use serde_json::{Value, json};

const CAP: u64 = 1_000_000;

const FIREFOX: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";

/// GETs `path` with these headers.
async fn get_with(app: &TestApp, path: &str, headers: &[(&str, &str)]) -> StatusCode {
    let mut request = Request::get(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.send(request.body(Body::empty()).unwrap()).await.status
}

/// What the log's files hold, both of them, as text.
fn on_disk(app: &TestApp) -> String {
    ["request-headers.jsonl.1", "request-headers.jsonl"]
        .iter()
        .filter_map(|name| std::fs::read_to_string(app.files_dir.path().join(name)).ok())
        .collect()
}

fn clients(summary: &Value) -> &Vec<Value> {
    summary["clients"].as_array().unwrap()
}

#[tokio::test]
async fn it_groups_requests_by_client_with_each_route_pattern_and_the_rules_verdict() {
    let app = test_app().await.with_request_log(CAP);
    let id = app.submit(&common::entry("Steering", "")).await;
    let firefox = [
        ("user-agent", FIREFOX),
        ("accept", "text/html,application/xhtml+xml"),
        ("sec-fetch-dest", "document"),
        ("sec-fetch-mode", "navigate"),
    ];
    get_with(&app, &format!("/e/{id}"), &firefox).await;
    get_with(&app, "/search?q=sycophancy", &firefox).await;
    get_with(&app, "/search?q=probes", &firefox).await;
    get_with(
        &app,
        "/tags",
        &[("user-agent", "curl/8.5.0"), ("accept", "*/*")],
    )
    .await;

    let summary = app.get_json("/staging/headers").await;

    assert_eq!(summary["recording"], json!(true));
    let browser = clients(&summary)
        .iter()
        .find(|client| client["user_agent"] == json!(FIREFOX))
        .unwrap();
    assert_eq!(browser["accept"], json!("text/html,application/xhtml+xml"));
    assert_eq!(browser["sec_fetch_dest"], json!("document"));
    assert_eq!(browser["sec_fetch_mode"], json!("navigate"));
    assert_eq!(browser["requests"], json!(3));
    assert_eq!(browser["rule_pages"], json!(3));
    assert_eq!(
        browser["routes"],
        json!([
            {"method": "GET", "route": "/e/{id}", "requests": 1},
            {"method": "GET", "route": "/search", "requests": 2},
        ])
    );
    let curl = clients(&summary)
        .iter()
        .find(|client| client["user_agent"] == json!("curl/8.5.0"))
        .unwrap();
    assert_eq!(curl["sec_fetch_dest"], Value::Null);
    assert_eq!(curl["rule_pages"], json!(0));
    assert_eq!(
        curl["routes"],
        json!([{"method": "GET", "route": "/tags", "requests": 1}])
    );
    assert!(!on_disk(&app).contains("sycophancy"), "{}", on_disk(&app));
    assert!(!on_disk(&app).contains(&id), "{}", on_disk(&app));
}

#[tokio::test]
async fn a_browsers_headers_with_a_token_count_as_text_as_the_rule_has_it() {
    let app = test_app().await.with_request_log(CAP);
    let token = app.token.clone();
    let bearer = format!("Bearer {token}");
    let headers = [
        ("user-agent", "agent-fetch/1"),
        ("sec-fetch-dest", "document"),
    ];
    get_with(&app, "/tags", &headers).await;
    get_with(
        &app,
        "/tags",
        &[headers[0], headers[1], ("authorization", &bearer)],
    )
    .await;

    let summary = app.get_json("/staging/headers").await;

    let client = &clients(&summary)[0];
    assert_eq!(client["requests"], json!(2));
    assert_eq!(client["rule_pages"], json!(1));
}

#[tokio::test]
async fn it_keeps_no_token_staging_key_cookie_or_address() {
    let app = test_app().await.with_request_log(CAP);
    let bearer = format!("Bearer {}", app.token);
    let request = Request::get("/tags")
        .header("user-agent", "agent-fetch/1")
        .header("authorization", &bearer)
        .header("x-staging-key", "staging-key-value")
        .header("cookie", "session=cookie-value")
        .body(Body::empty())
        .unwrap();
    app.send_from("203.0.113.9:4000", request).await;

    let stored = on_disk(&app);
    let shown = app.get("/staging/headers").await.body;

    for secret in [
        app.token.as_str(),
        "staging-key-value",
        "cookie-value",
        "203.0.113.9",
    ] {
        assert!(!stored.contains(secret), "{secret} is on disk:\n{stored}");
        assert!(!shown.contains(secret), "{secret} is shown:\n{shown}");
    }
    assert!(stored.contains("agent-fetch/1"), "{stored}");
}

#[tokio::test]
async fn reading_the_record_adds_nothing_to_it() {
    let app = test_app().await.with_request_log(CAP);

    app.get_json("/staging/headers").await;
    let summary = app.get_json("/staging/headers").await;

    assert_eq!(summary["requests"], json!(0));
    assert_eq!(on_disk(&app), "");
}

#[tokio::test]
async fn in_plain_text_the_clients_headers_come_below_the_delimiter() {
    let app = test_app().await.with_request_log(CAP);
    get_with(&app, "/tags", &[("user-agent", "requests: 999")]).await;

    let reply = app.get("/staging/headers").await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (above, below) = reply.body.split_once(DELIMITER).expect(&reply.body);
    assert_eq!(common::field(above, "recording"), Some("yes"));
    assert_eq!(common::field(above, "requests"), Some("1"));
    assert_eq!(common::field(above, "clients"), Some("1"));
    assert!(
        below.contains("user-agent: requests: 999"),
        "{}",
        reply.body
    );
    assert!(below.contains("route: GET /tags (1)"), "{}", reply.body);
}

#[tokio::test]
async fn a_long_header_is_kept_to_its_first_300_characters() {
    let app = test_app().await.with_request_log(CAP);
    let long = "a".repeat(1000);
    get_with(
        &app,
        "/tags",
        &[("user-agent", &long), ("accept", "text/plain")],
    )
    .await;

    let summary = app.get_json("/staging/headers").await;

    let agent = clients(&summary)[0]["user_agent"].as_str().unwrap();
    assert_eq!(agent.chars().count(), 300);
}

#[tokio::test]
async fn past_its_cap_the_file_moves_aside_and_the_record_before_that_goes() {
    let app = test_app().await.with_request_log(2_000);
    for n in 0..40 {
        get_with(&app, "/tags", &[("user-agent", &format!("agent-{n:02}"))]).await;
    }

    let summary = app.get_json("/staging/headers").await;

    let agents: Vec<&str> = clients(&summary)
        .iter()
        .map(|client| client["user_agent"].as_str().unwrap())
        .collect();
    assert!(agents.contains(&"agent-39"), "{agents:?}");
    assert!(!agents.contains(&"agent-00"), "{agents:?}");
    let kept: u64 = ["request-headers.jsonl", "request-headers.jsonl.1"]
        .iter()
        .map(|name| {
            std::fs::metadata(app.files_dir.path().join(name))
                .unwrap()
                .len()
        })
        .sum();
    assert!(kept <= 4_000 + 1_000, "{kept} bytes kept");
    assert_eq!(
        summary["requests"],
        json!(agents.len()),
        "each agent sent one request"
    );
}

#[tokio::test]
async fn a_server_without_a_log_says_it_keeps_none() {
    let app = test_app().await;
    get_with(&app, "/tags", &[("user-agent", "agent-fetch/1")]).await;

    let summary = app.get_json("/staging/headers").await;
    let text = app.get("/staging/headers").await;

    assert_eq!(summary["recording"], json!(false));
    assert_eq!(summary["requests"], json!(0));
    assert_eq!(
        common::field(&text.body, "recording"),
        Some("no, only staging keeps a request log")
    );
}

#[tokio::test]
async fn production_has_no_such_route() {
    let mut app = test_app().await.with_request_log(CAP);
    app.state.environment = Environment::Production;
    app.router = filedrawer_server::app(app.state.clone());

    let reply = app.get("/staging/headers").await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
}

#[tokio::test]
async fn only_staging_keeps_a_log_and_it_lives_beside_the_database() {
    let mut app = test_app().await;
    let database = app.files_dir.path().join("staging.db");
    let database = database.to_str().unwrap();
    assert!(RequestLog::for_environment(Environment::Production, database).is_none());
    assert!(RequestLog::for_environment(Environment::Dev, database).is_none());
    app.state.request_log =
        RequestLog::for_environment(Environment::Staging, database).map(std::sync::Arc::new);
    app.router = filedrawer_server::app(app.state.clone());

    get_with(&app, "/tags", &[("user-agent", "agent-fetch/1")]).await;

    assert!(on_disk(&app).contains("agent-fetch/1"), "{}", on_disk(&app));
}
