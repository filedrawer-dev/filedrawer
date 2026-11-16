//! Seam: the router, for `POST /feedback` (design.md, Feedback and usability cycles).

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::test_app;
use common::through_caddy::{hourly, retry_after, wide};
use filedrawer_server::limits::{Limits, Window};

const ITEM: &str = r#"{"experiment": "x", "wanted": "y", "workaround": "z"}"#;

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

#[tokio::test]
async fn an_accounts_stored_feedback_is_limited_and_refused_items_dont_count() {
    let app = test_app().await.with_limits(Limits {
        feedback_per_account: hourly(2),
        ..wide()
    });
    // Refused items store nothing, so they take no place in the count.
    for _ in 0..3 {
        let refused = app
            .send(feedback(Some(&app.token), r#"{"wanted": "y"}"#))
            .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    for _ in 0..2 {
        let stored = app.send(feedback(Some(&app.token), ITEM)).await;
        assert_eq!(stored.status, StatusCode::CREATED, "{}", stored.body);
    }
    let limited = app.send(feedback(Some(&app.token), ITEM)).await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert!(
        limited.body.contains("this account's feedback"),
        "{}",
        limited.body
    );
    // Room comes back when the oldest item is an hour old.
    assert!(
        (3500..=3600).contains(&retry_after(&limited)),
        "{}",
        retry_after(&limited)
    );
    // A refused item is still refused for what is wrong with it, not for the limit.
    let refused = app
        .send(feedback(Some(&app.token), r#"{"wanted": "y"}"#))
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    // Another account has its own count.
    let other = app.other_token().await;
    let theirs = app.send(feedback(Some(&other), ITEM)).await;
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.body);
}

#[tokio::test]
async fn the_site_allows_an_account_10_feedback_items_an_hour_and_50_a_day() {
    let windows: Vec<(i64, i64)> = Limits::site()
        .feedback_per_account
        .iter()
        .map(|window| (window.seconds, window.count))
        .collect();
    assert_eq!(windows, [(3600, 10), (86_400, 50)]);
}

#[tokio::test]
async fn feedback_older_than_the_window_no_longer_counts() {
    let app = test_app().await.with_limits(Limits {
        feedback_per_account: hourly(2),
        ..wide()
    });
    for _ in 0..2 {
        let stored = app.send(feedback(Some(&app.token), ITEM)).await;
        assert_eq!(stored.status, StatusCode::CREATED, "{}", stored.body);
    }
    sqlx::query("UPDATE feedback SET created_at = '2026-01-01T00:00:00Z' WHERE account_id = ?")
        .bind(&app.account)
        .execute(&app.pool)
        .await
        .unwrap();
    let stored = app.send(feedback(Some(&app.token), ITEM)).await;
    assert_eq!(stored.status, StatusCode::CREATED, "{}", stored.body);
}

#[tokio::test]
async fn with_two_windows_full_the_wait_is_for_the_one_that_frees_up_last() {
    let hour = Window {
        seconds: 3600,
        count: 1,
    };
    let day = Window {
        seconds: 86_400,
        count: 1,
    };
    for windows in [vec![hour, day], vec![day, hour]] {
        let app = test_app().await.with_limits(Limits {
            feedback_per_account: windows,
            ..wide()
        });
        let stored = app.send(feedback(Some(&app.token), ITEM)).await;
        assert_eq!(stored.status, StatusCode::CREATED, "{}", stored.body);
        let limited = app.send(feedback(Some(&app.token), ITEM)).await;
        assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(
            (86_300..=86_400).contains(&retry_after(&limited)),
            "{}",
            retry_after(&limited)
        );
    }
}
