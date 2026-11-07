//! Seam: the router, with a fake model client, for `GET /staging/status`: what
//! staging's screen spent today, what waits, and how the last calls went, for agents
//! that can't read the server's log.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::attachments::{citing, set_with};
use common::fake_models::FakeModels;
use common::{TestApp, entry, field, id_in, test_app, test_app_with};
use filedrawer_server::config::Environment;
use filedrawer_server::jobs::publish_due;
use time::OffsetDateTime;

const DOLLAR: i64 = 1_000_000_000;

async fn app_with(models: &FakeModels) -> TestApp {
    test_app_with(models.screener(), DOLLAR).await
}

/// Submits an entry whose title holds `marker` and ends its hold; returns its ID.
async fn publish(app: &TestApp, marker: &str) -> String {
    let held = app
        .post("/entries", entry(&format!("Steering {marker}"), ""))
        .await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    id
}

#[tokio::test]
async fn before_any_call_it_says_nothing_has_happened_yet() {
    let models = FakeModels::default();
    let app = app_with(&models).await;

    let status = app.get("/staging/status").await;

    assert_eq!(status.status, StatusCode::OK, "{}", status.body);
    assert_eq!(field(&status.body, "environment"), Some("dev"));
    assert_eq!(
        field(&status.body, "screener"),
        Some("models (claude-haiku-5-5, then claude-sonnet-5-5 on a hazard doubt)")
    );
    assert_eq!(
        field(&status.body, "spent today"),
        Some("$0.0000 of the day's $1.00")
    );
    assert_eq!(field(&status.body, "waiting"), Some("0"));
    assert_eq!(
        field(&status.body, "last success"),
        Some("none since the server started")
    );
    assert_eq!(
        field(&status.body, "last failure"),
        Some("none since the server started")
    );
}

#[tokio::test]
async fn after_a_screened_entry_it_shows_the_spend_and_the_last_success() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let id = publish(&app, "plain").await;

    let status = app.get("/staging/status").await;

    // One Haiku call: 1,000 input and 200 output tokens.
    assert_eq!(
        field(&status.body, "spent today"),
        Some("$0.0002 of the day's $1.00")
    );
    let success = field(&status.body, "last success").unwrap();
    assert!(
        success.starts_with("20") && success.ends_with('Z'),
        "{success}"
    );
    assert_eq!(
        field(&status.body, "last failure"),
        Some("none since the server started")
    );
    assert!(!status.body.contains(&id), "{}", status.body);
    assert!(!status.body.contains(&app.account), "{}", status.body);
}

#[tokio::test]
async fn a_failing_call_shows_its_error_and_the_entry_it_leaves_waiting() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    publish(&app, "FAKE-ERROR").await;

    let status = app.get("/staging/status").await;

    let waiting = field(&status.body, "waiting").unwrap();
    assert!(
        waiting.starts_with("1, the oldest submitted 20"),
        "{waiting}"
    );
    let failure = field(&status.body, "last failure").unwrap();
    assert!(
        failure.ends_with("Z (entry text): the API answered 529: overloaded_error: Overloaded"),
        "{failure}"
    );
}

#[tokio::test]
async fn a_failing_call_on_attached_code_says_so() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let set = set_with(&app, &app.token, "run.py", "print('FAKE-ERROR')\n").await;
    let held = app.post("/entries", citing("Steering code", &set)).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    app.post(&format!("/e/{id}/publish"), "").await;
    // Entries citing attachments wait for the worker.
    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();

    let status = app.get("/staging/status").await;

    let failure = field(&status.body, "last failure").unwrap();
    assert!(
        failure.contains("Z (attached code): the API answered 529"),
        "{failure}"
    );
}

#[tokio::test]
async fn pass_all_calls_no_model_so_notes_no_success() {
    let app = test_app().await;
    publish(&app, "plain").await;

    let status = app.get("/staging/status").await;

    assert_eq!(field(&status.body, "screener"), Some("pass-all"));
    assert_eq!(
        field(&status.body, "last success"),
        Some("none since the server started")
    );
}

#[tokio::test]
async fn it_answers_json_too() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    publish(&app, "FAKE-ERROR").await;

    let status = app.get_json("/staging/status").await;

    assert_eq!(status["environment"], "dev");
    assert_eq!(status["screener"], "models");
    assert_eq!(
        status["models"],
        serde_json::json!(["claude-haiku-5-5", "claude-sonnet-5-5"])
    );
    assert_eq!(status["cap_usd"], 1.0);
    assert_eq!(status["waiting"], 1);
    assert!(status["oldest_waiting_since"].is_string(), "{status}");
    assert!(status["last_success"].is_null(), "{status}");
    assert_eq!(
        status["last_failure"]["error"],
        "the API answered 529: overloaded_error: Overloaded"
    );
    assert_eq!(status["last_failure"]["subject"], "entry text");
}

#[tokio::test]
async fn production_has_no_such_route() {
    let models = FakeModels::default();
    let mut app = app_with(&models).await;
    app.state.environment = Environment::Production;
    app.router = filedrawer_server::app(app.state.clone());

    // Exactly as a path that doesn't exist answers, for each method.
    for method in ["GET", "POST"] {
        let send = |uri: &'static str| {
            app.send(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
        };
        let (reply, absent) = (send("/staging/status").await, send("/no-such-route").await);
        assert_eq!(reply.status, absent.status, "{method}: {}", reply.body);
        assert_eq!(reply.body, absent.body, "{method}");
    }
    assert_eq!(
        app.get("/staging/status").await.status,
        StatusCode::NOT_FOUND
    );
}
