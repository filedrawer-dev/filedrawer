//! Seam: the router, for the largest body each route reads: a declared length past
//! it is refused before any of the body is read, so the time a body may take
//! (`serve::Bounds`) follows from what the route accepts (uncertainties.md, How long
//! a request's headers and body may take).

use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};

use crate::common::{PUBLIC_URL, TestApp, test_app};

/// Longer than any refusal takes on a busy machine; a route that waits for the
/// body reaches it.
const TOO_LONG: Duration = Duration::from_secs(30);

/// A body that never arrives, behind headers that declare `length` bytes.
fn declaring(request: axum::http::request::Builder, length: u64) -> Request<Body> {
    let never = futures::stream::pending::<Result<Bytes, std::io::Error>>();
    request
        .header("content-length", length.to_string())
        .body(Body::from_stream(never))
        .unwrap()
}

/// The reply to `request`, which must come without the body.
async fn before_the_body(app: &TestApp, request: Request<Body>) -> crate::common::Reply {
    tokio::time::timeout(TOO_LONG, app.send(request))
        .await
        .expect("the route answered without waiting for the body")
}

fn login() -> axum::http::request::Builder {
    Request::post("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", PUBLIC_URL)
}

#[tokio::test]
async fn a_form_route_refuses_a_body_longer_than_a_form_before_reading_it() {
    let app = test_app().await;
    let reply = before_the_body(&app, declaring(login(), 64 * 1024 + 1)).await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
    assert_eq!(
        reply
            .headers
            .get("connection")
            .map(|value| value.as_bytes()),
        Some(&b"close"[..])
    );
    for route in ["/signup", "/verify", "/reset", "/reset/confirm"] {
        let request = Request::post(route)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("origin", PUBLIC_URL);
        let reply = before_the_body(&app, declaring(request, 64 * 1024 + 1)).await;
        assert_eq!(
            reply.status,
            StatusCode::PAYLOAD_TOO_LARGE,
            "{route}: {}",
            reply.body
        );
    }
}

#[tokio::test]
async fn a_route_refuses_a_body_longer_than_it_reads_before_reading_it() {
    let app = test_app().await;
    let entries = Request::post("/entries")
        .header("content-type", "text/markdown")
        .header("authorization", format!("Bearer {}", app.token));
    let reply = before_the_body(&app, declaring(entries, 1024 * 1024 + 1)).await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
    assert_eq!(
        reply.body,
        "refused:\n- the body declares 1048577 bytes, and this route takes at most 1048576\n"
    );
    let attachments = Request::post("/attachments")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", app.token));
    let reply = before_the_body(&app, declaring(attachments, 4 * 1024 * 1024 + 1)).await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
}

/// Attached files take a larger body than other routes: one past 1 MiB is read.
#[tokio::test]
async fn attachments_read_a_body_past_the_other_routes_limit() {
    let app = test_app().await;
    let body = format!("{{\"files\": \"{}\"}}", "a".repeat(2 * 1024 * 1024));
    let request = Request::post("/attachments")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", app.token))
        .header("content-length", body.len().to_string())
        .body(Body::from(body))
        .unwrap();
    let reply = app.send(request).await;
    assert_ne!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
}

/// A form as long as a form gets is read as one.
#[tokio::test]
async fn a_form_route_reads_a_form_up_to_its_limit() {
    let app = test_app().await;
    let password = "a".repeat(1024);
    let next = "a".repeat(64 * 1024 - 1024 - 200);
    let fields = [
        ("email", "nobody@example.com"),
        ("password", password.as_str()),
        ("next", next.as_str()),
    ];
    let reply = app.form("/login", &fields, None).await;
    assert_ne!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
}
