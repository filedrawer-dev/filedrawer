//! A fresh server on an in-memory database, with one account and its token.
#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use std::sync::{Arc, Mutex};

use filedrawer_server::mail::{Email, Mailer, Sending};
use filedrawer_server::{AppState, Screener, admin, app, db};

/// A mailer that keeps every email it is sent, for tests to read back.
#[derive(Clone, Default)]
pub struct Outbox(Arc<Mutex<Vec<Email>>>);

impl Outbox {
    pub fn sent(&self) -> Vec<Email> {
        self.0.lock().unwrap().clone()
    }
}

impl Mailer for Outbox {
    fn send(&self, email: Email) -> Sending<'_> {
        self.0.lock().unwrap().push(email);
        Box::pin(async { Ok(()) })
    }
}
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The site's address in tests; emails link to it and forms must come from it.
pub const PUBLIC_URL: &str = "http://localhost";

pub struct TestApp {
    pub router: Router,
    pub pool: sqlx::SqlitePool,
    pub account: String,
    pub token: String,
    pub outbox: Outbox,
}

/// The state every test app shares: an in-memory database and mailer.
pub fn state(pool: sqlx::SqlitePool, outbox: &Outbox) -> AppState {
    AppState {
        pool,
        screener: Screener::PassAll,
        mailer: Arc::new(outbox.clone()),
        public_url: PUBLIC_URL.to_string(),
    }
}

pub async fn test_app() -> TestApp {
    let pool = db::open(":memory:").await.unwrap();
    let account = admin::create_account(&pool, "Ada").await.unwrap();
    let token = admin::create_token(&pool, &account, "laptop")
        .await
        .unwrap();
    let outbox = Outbox::default();
    let router = app(state(pool.clone(), &outbox));
    TestApp {
        router,
        pool,
        account,
        token,
        outbox,
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    /// The `name=value` pair the reply's Set-Cookie sets, without its attributes.
    pub fn cookie(&self) -> Option<String> {
        let header = self.headers.get("set-cookie")?.to_str().ok()?;
        Some(header.split(';').next()?.to_string())
    }

    pub fn location(&self) -> Option<&str> {
        self.headers.get("location")?.to_str().ok()
    }
}

impl TestApp {
    pub async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    /// GETs `path`, with a session cookie if given.
    pub async fn page(&self, path: &str, cookie: Option<&str>) -> Reply {
        let mut request = Request::get(path);
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }

    /// POSTs a form as a browser on the site would: with its Origin, and the session
    /// cookie if given.
    pub async fn form(&self, path: &str, fields: &[(&str, &str)], cookie: Option<&str>) -> Reply {
        let body: String = fields
            .iter()
            .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let mut request = Request::post(path)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("origin", PUBLIC_URL);
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        self.send(request.body(Body::from(body)).unwrap()).await
    }

    /// Every email sent so far. The server sends in spawned tasks, so this first gives
    /// them the chance to run.
    pub async fn sent(&self) -> Vec<Email> {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        self.outbox.sent()
    }

    /// The one link to this site in the last email to `to`.
    pub async fn link_in_last_email_to(&self, to: &str) -> String {
        let email = self
            .sent()
            .await
            .into_iter()
            .rev()
            .find(|email| email.to == to)
            .unwrap_or_else(|| panic!("no email to {to}"));
        let start = email.body.find(PUBLIC_URL).expect("the email has a link");
        let link: String = email.body[start..]
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        link.strip_prefix(PUBLIC_URL).unwrap().to_string()
    }

    pub async fn get(&self, path: &str) -> Reply {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    pub async fn get_json(&self, path: &str) -> serde_json::Value {
        let reply = self
            .send(
                Request::get(path)
                    .header("accept", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        serde_json::from_str(&reply.body).unwrap()
    }

    /// POSTs `body` to `path` with this app's token.
    pub async fn post(&self, path: &str, body: impl Into<String>) -> Reply {
        self.post_as(path, Some(&self.token), body).await
    }

    pub async fn post_as(&self, path: &str, token: Option<&str>, body: impl Into<String>) -> Reply {
        let mut request = Request::post(path).header("content-type", "text/markdown");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        self.send(request.body(Body::from(body.into())).unwrap())
            .await
    }

    /// Submits `entry` and returns its ID, which must be accepted.
    pub async fn submit(&self, entry: &str) -> String {
        let reply = self.post("/entries", entry).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        reply
            .body
            .lines()
            .find_map(|line| line.strip_prefix("id: "))
            .expect("the response names the ID")
            .to_string()
    }
}

/// A made-up entry the analyzer reads without warnings. `front` adds front-matter lines.
pub fn entry(title: &str, front: &str) -> String {
    format!(
        "---
title: {title}
safety_motivation: Made up for a test.
hypothesis: Steering at layer 14 raises refusals.
outcome: no_effect
stage: main
models:
  - name: model-a
    role: subject
    route: api
  - name: judge-b
    role: judge
    route: api
tags: [steering, refusal]
{front}---

## Setup

not recorded

## Results

Intervals: 95%, Wilson, per prompt.

| model | condition | estimate | ci_low | ci_high | n | outcome |
|---|---|---|---|---|---|---|
| model-a | steered vs unsteered | 0.01 | -0.05 | 0.07 | 100/100 | |
| model-c | steered vs unsteered | 0.30 | 0.20 | 0.40 | 100/100 | effect |

## Reproduce

Command not recorded.
"
    )
}

/// `today + days` as YYYY-MM-DD.
pub fn date_in(days: i64) -> String {
    let date = time::OffsetDateTime::now_utc().date() + time::Duration::days(days);
    date.to_string()
}

/// Percent-encodes a form value.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
