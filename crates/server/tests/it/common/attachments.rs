//! Attachment helpers for the tests of removed and banned files.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Reply, TestApp, entry};

pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

pub fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `POST /attachments` of `set` with `token`, asking for JSON.
pub async fn upload(app: &TestApp, token: &str, set: Value) -> Reply {
    app.send(
        Request::post("/attachments")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Body::from(set.to_string()))
            .unwrap(),
    )
    .await
}

/// A set holding one file, `src/<name>`, with `text`.
pub fn one_file(name: &str, text: &str) -> Value {
    json!({
        "repos": {"main": "no remote"},
        "files": [{"origin": format!("main@{COMMIT}:src/{name}"), "text": text}],
    })
}

/// Uploads a set holding `src/<name>` with `token`, which must be stored; returns
/// the set's ID.
pub async fn set_with(app: &TestApp, token: &str, name: &str, text: &str) -> String {
    let reply = upload(app, token, one_file(name, text)).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    serde_json::from_str::<Value>(&reply.body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// An entry titled `title` citing `set`, with no `code` link.
pub fn citing(title: &str, set: &str) -> String {
    entry(title, &format!("attachments: {set}\n"))
}

/// A GET of `path` addressed to `host`.
pub async fn get_on(app: &TestApp, host: &str, path: &str) -> Reply {
    app.send(
        Request::get(path)
            .header("host", host)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}
