//! Seam: the router, for the rate limits (design.md, Limits and abuse; issue #14):
//! every request per client IP, token-authenticated writes per account, and signups
//! and account emails per IP and per address.

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Reply, TestApp, test_app};
use filedrawer_server::limits::{Limits, Rate, Window};

/// Caddy on the same host, as staging and production run.
const CADDY: &str = "127.0.0.1:50000";

fn per_minute(count: u32) -> Rate {
    Rate {
        count,
        per: Duration::from_secs(60),
    }
}

fn hourly(count: i64) -> Vec<Window> {
    vec![Window {
        seconds: 3600,
        count,
    }]
}

/// Limits wide enough that only the one a test sets small trips.
fn wide() -> Limits {
    Limits {
        per_ip: per_minute(10_000),
        writes_per_account: per_minute(10_000),
        signups_per_ip: hourly(10_000),
        emails_per_address: hourly(10_000),
        emails_per_ip: hourly(10_000),
    }
}

/// GETs /tags from `forwarded` through Caddy.
async fn read_as(app: &TestApp, forwarded: &str) -> Reply {
    let request = Request::get("/tags")
        .header("x-forwarded-for", forwarded)
        .body(Body::empty())
        .unwrap();
    app.send_from(CADDY, request).await
}

fn retry_after(reply: &Reply) -> u64 {
    reply
        .headers
        .get("retry-after")
        .expect("a 429 says when to try again")
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn a_client_past_its_rate_waits_and_others_dont() {
    let app = test_app().await.with_limits(Limits {
        per_ip: per_minute(3),
        ..wide()
    });
    for _ in 0..3 {
        assert_eq!(read_as(&app, "203.0.113.5").await.status, StatusCode::OK);
    }
    let refused = read_as(&app, "203.0.113.5").await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.body
    );
    // One request comes back every 20 seconds.
    assert!(
        (1..=20).contains(&retry_after(&refused)),
        "{}",
        retry_after(&refused)
    );
    assert!(
        refused.body.contains("too many requests"),
        "{}",
        refused.body
    );
    assert_eq!(read_as(&app, "203.0.113.6").await.status, StatusCode::OK);
}

#[tokio::test]
async fn forwarded_for_counts_only_from_the_proxy_on_the_same_host() {
    let app = test_app().await.with_limits(Limits {
        per_ip: per_minute(2),
        ..wide()
    });
    // From elsewhere, the header is the client's own claim: the peer is the key.
    for (n, forwarded) in ["198.51.100.1", "198.51.100.2", "198.51.100.3"]
        .iter()
        .enumerate()
    {
        let request = Request::get("/tags")
            .header("x-forwarded-for", *forwarded)
            .body(Body::empty())
            .unwrap();
        let reply = app.send_from("192.0.2.9:4000", request).await;
        let expected = if n < 2 {
            StatusCode::OK
        } else {
            StatusCode::TOO_MANY_REQUESTS
        };
        assert_eq!(reply.status, expected, "{forwarded}");
    }
    // Through Caddy, the last address is the one Caddy saw.
    assert_eq!(
        read_as(&app, "192.0.2.9, 203.0.113.7").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "192.0.2.9, 203.0.113.7").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "192.0.2.9, 203.0.113.7").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn ipv6_clients_share_their_64() {
    let app = test_app().await.with_limits(Limits {
        per_ip: per_minute(2),
        ..wide()
    });
    assert_eq!(
        read_as(&app, "2001:db8:1:2::1").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "2001:db8:1:2::ffff").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "2001:db8:1:2:aaaa::3").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        read_as(&app, "2001:db8:1:3::1").await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn an_accounts_writes_are_limited_refused_ones_included() {
    let app = test_app().await.with_limits(Limits {
        writes_per_account: per_minute(2),
        ..wide()
    });
    // Each is refused by the mechanical checks, which costs no allowance.
    for _ in 0..2 {
        let refused = app.post("/entries", "not an entry").await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            refused.body
        );
    }
    let limited = app.post("/entries", "not an entry").await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert!(retry_after(&limited) >= 1);
    // Reads with the token aren't writes.
    assert_eq!(app.get("/tags").await.status, StatusCode::OK);
    // Another account's writes have their own count.
    let other = app.other_token().await;
    let theirs = app.post_as("/entries", Some(&other), "not an entry").await;
    assert_eq!(
        theirs.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        theirs.body
    );
}

/// POSTs the signup form from `forwarded` through Caddy.
async fn sign_up_as(app: &TestApp, forwarded: &str, email: &str) -> Reply {
    let body = format!("email={email}&password=correct-horse-battery-9&display_name=Ada");
    let request = Request::post("/signup")
        .header("x-forwarded-for", forwarded)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", common::PUBLIC_URL)
        .body(Body::from(body))
        .unwrap();
    app.send_from(CADDY, request).await
}

#[tokio::test]
async fn signups_are_limited_per_ip_and_survive_a_restart() {
    let app = test_app().await.with_limits(Limits {
        signups_per_ip: hourly(2),
        ..wide()
    });
    for n in 0..2 {
        let reply = sign_up_as(&app, "203.0.113.5", &format!("a{n}%40example.com")).await;
        assert!(
            reply.status.is_success() || reply.status.is_redirection(),
            "{}",
            reply.body
        );
    }
    let limited = sign_up_as(&app, "203.0.113.5", "a9%40example.com").await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert!(retry_after(&limited) > 60, "{}", retry_after(&limited));
    // The count is kept in the database, so a deploy doesn't reset it.
    let restarted = app.with_limits(Limits {
        signups_per_ip: hourly(2),
        ..wide()
    });
    let again = sign_up_as(&restarted, "203.0.113.5", "a8%40example.com").await;
    assert_eq!(
        again.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        again.body
    );
    let elsewhere = sign_up_as(&restarted, "203.0.113.6", "a7%40example.com").await;
    assert!(elsewhere.status.is_success() || elsewhere.status.is_redirection());
}

/// POSTs the password-reset form for `email` from `forwarded` through Caddy.
async fn reset_as(app: &TestApp, forwarded: &str, email: &str) -> Reply {
    let request = Request::post("/reset")
        .header("x-forwarded-for", forwarded)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", common::PUBLIC_URL)
        .body(Body::from(format!("email={email}")))
        .unwrap();
    app.send_from(CADDY, request).await
}

#[tokio::test]
async fn account_emails_are_limited_per_address_whether_or_not_it_has_an_account() {
    let app = test_app().await.with_limits(Limits {
        emails_per_address: hourly(2),
        ..wide()
    });
    let nobody = "nobody%40example.com";
    for forwarded in ["203.0.113.5", "203.0.113.6"] {
        assert_eq!(
            reset_as(&app, forwarded, nobody).await.status,
            StatusCode::OK
        );
    }
    // From a third IP, the address is still full.
    let limited = reset_as(&app, "203.0.113.7", nobody).await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert_eq!(
        reset_as(&app, "203.0.113.7", "other%40example.com")
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn account_emails_are_limited_per_ip() {
    let app = test_app().await.with_limits(Limits {
        emails_per_ip: hourly(2),
        ..wide()
    });
    for n in 0..2 {
        let reply = reset_as(&app, "203.0.113.5", &format!("p{n}%40example.com")).await;
        assert_eq!(reply.status, StatusCode::OK);
    }
    let limited = reset_as(&app, "203.0.113.5", "p9%40example.com").await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
}
