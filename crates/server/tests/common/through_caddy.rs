//! Requests as Caddy forwards them, from a client IP a test chooses, and rate limits
//! a test sets small: for the tests of the limits and of invite codes.

use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use filedrawer_server::limits::{Limits, Rate, Window};

use super::{PUBLIC_URL, Reply, TestApp, encode};

/// Caddy on the same host, as staging and production run.
pub const CADDY: &str = "127.0.0.1:50000";
/// The password `sign_up_from` signs up with.
pub const SIGNUP_PASSWORD: &str = "correct-horse-battery-9";

pub fn per_minute(count: u32) -> Rate {
    Rate {
        count,
        per: Duration::from_secs(60),
    }
}

pub fn hourly(count: i64) -> Vec<Window> {
    vec![Window {
        seconds: 3600,
        count,
    }]
}

/// Limits wide enough that only the one a test sets small trips.
pub fn wide() -> Limits {
    Limits {
        per_ip: per_minute(10_000),
        writes_per_account: per_minute(10_000),
        signups_per_ip: hourly(10_000),
        emails_per_address: hourly(10_000),
        emails_per_ip: hourly(10_000),
        failed_logins_per_address: hourly(10_000),
    }
}

/// The seconds a 429 says to wait.
pub fn retry_after(reply: &Reply) -> u64 {
    reply
        .headers
        .get("retry-after")
        .expect("a 429 says when to try again")
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// GETs `path` from the client `forwarded`, through Caddy.
pub async fn get_from(app: &TestApp, forwarded: &str, path: &str) -> Reply {
    let request = Request::get(path)
        .header("x-forwarded-for", forwarded)
        .body(Body::empty())
        .unwrap();
    app.send_from(CADDY, request).await
}

/// POSTs a form from the client `forwarded`, through Caddy, from the site's origin.
pub async fn form_from(
    app: &TestApp,
    forwarded: &str,
    path: &str,
    fields: &[(&str, &str)],
) -> Reply {
    let body: String = fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let request = Request::post(path)
        .header("x-forwarded-for", forwarded)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", PUBLIC_URL)
        .body(Body::from(body))
        .unwrap();
    app.send_from(CADDY, request).await
}

/// Signs up `email` from `forwarded`, with an invite code if given.
pub async fn sign_up_from(
    app: &TestApp,
    forwarded: &str,
    email: &str,
    invite: Option<&str>,
) -> Reply {
    let mut fields = vec![
        ("email", email),
        ("password", SIGNUP_PASSWORD),
        ("display_name", "Ada"),
    ];
    fields.extend(invite.map(|code| ("invite", code)));
    form_from(app, forwarded, "/signup", &fields).await
}

/// Asks for a password reset for `email` from `forwarded`.
pub async fn reset_from(app: &TestApp, forwarded: &str, email: &str) -> Reply {
    form_from(app, forwarded, "/reset", &[("email", email)]).await
}
