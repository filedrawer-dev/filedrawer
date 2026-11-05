//! Seam: the router, for `POST /feedback` (design.md, Feedback and usability cycles).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::test_app;

fn feedback(token: Option<&str>, body: &str) -> Request<Body> {
    let mut request = Request::post("/feedback").header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn feedback_with_all_three_fields_is_stored() {
    let app = test_app().await;
    let body = r#"{"experiment": "steering sweep", "wanted": "a column per layer", "workaround": "one entry per layer"}"#;
    let reply = app.send(feedback(Some(&app.token), body)).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert!(
        reply.body.starts_with("feedback received: "),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn feedback_missing_a_field_is_refused() {
    let app = test_app().await;
    let reply = app
        .send(feedback(
            Some(&app.token),
            r#"{"experiment": "x", "wanted": "y"}"#,
        ))
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("workaround"), "{}", reply.body);
}

#[tokio::test]
async fn feedback_needs_a_token() {
    let app = test_app().await;
    let body = r#"{"experiment": "x", "wanted": "y", "workaround": "z"}"#;
    assert_eq!(
        app.send(feedback(None, body)).await.status,
        StatusCode::UNAUTHORIZED
    );
}
