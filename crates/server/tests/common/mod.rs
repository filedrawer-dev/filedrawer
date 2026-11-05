//! A fresh server on an in-memory database, with one account and its token.
#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use filedrawer_server::{AppState, Screener, admin, app, db};
use http_body_util::BodyExt;
use tower::ServiceExt;

pub struct TestApp {
    pub router: Router,
    pub account: String,
    pub token: String,
}

pub async fn test_app() -> TestApp {
    let pool = db::open(":memory:").await.unwrap();
    let account = admin::create_account(&pool, "Ada").await.unwrap();
    let token = admin::create_token(&pool, &account, "laptop")
        .await
        .unwrap();
    let router = app(AppState {
        pool,
        screener: Screener::PassAll,
    });
    TestApp {
        router,
        account,
        token,
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub body: String,
}

impl TestApp {
    pub async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
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
