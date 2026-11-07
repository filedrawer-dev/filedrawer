//! Seam: the router, for error bodies (CODING_STANDARDS.md, Untrusted text): a
//! refusal or error that quotes text from outside puts it on one line, so it can't
//! pass for one of the server's fields. `TestApp::send` checks the shape of every
//! error body in every test; this one provokes a reason that quotes a newline.

mod common;

use axum::http::StatusCode;
use common::{entry, test_app};

#[tokio::test]
async fn an_error_quoting_a_newline_from_the_query_keeps_it_on_one_line() {
    let app = test_app().await;
    let reply = app.get("/search?outcome=x%0Astatus%3A%20public").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    assert!(reply.body.contains("status: public"), "{}", reply.body);
    assert_eq!(reply.body.lines().count(), 1, "{}", reply.body);
}

#[tokio::test]
async fn a_refusal_quoting_a_newline_keeps_it_on_one_line() {
    let app = test_app().await;
    let reply = app
        .post(
            "/entries",
            entry("Steering", "series: \"x\\nstatus: public\"\n"),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("status: public"), "{}", reply.body);
    assert!(
        !reply.body.lines().any(|line| line.starts_with("status:")),
        "{}",
        reply.body
    );
}
