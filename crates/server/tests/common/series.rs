//! Series helpers shared by the series and account-deletion tests.

use axum::http::StatusCode;

use super::{Reply, TestApp, entry, field, id_in};

pub const DESCRIPTION: &str = "Shared setup for the sweep.

## Models

model-a over the API, 2026-10-20.

### Prompts

520 prompts from the sweep set.

## Judging

judge-b scores each reply.
";

pub fn series_doc(title: &str, description: &str) -> String {
    format!("---\ntitle: {title}\n---\n\n{description}")
}

/// A series made with `token`: its ID and join key.
pub async fn create(app: &TestApp, token: &str) -> (String, String) {
    let reply = app
        .post_as(
            "/series",
            Some(token),
            series_doc("Layer sweep", DESCRIPTION),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let key = field(&reply.body, "key").expect("the reply shows the key");
    (id_in(&reply.body), key.to_string())
}

/// An entry citing `cite` in its `series` field.
pub fn citing(title: &str, cite: &str, front: &str) -> String {
    entry(title, &format!("series: {cite}\n{front}"))
}

pub async fn make_public(app: &TestApp, cookie: &str, id: &str) -> Reply {
    app.form(&format!("/account/series/{id}/publish"), &[], Some(cookie))
        .await
}

/// A third verified account's token, beside the test app's own and `other_token`'s.
pub async fn third_token(app: &TestApp) -> String {
    let account = filedrawer_server::admin::create_account(&app.pool, "Grace")
        .await
        .unwrap();
    filedrawer_server::admin::create_token(&app.pool, &account, "third")
        .await
        .unwrap()
}

/// Submits `text` with `token` and ends its hold; returns its ID.
pub async fn submit_with(app: &TestApp, token: &str, text: &str) -> String {
    let held = app.post_as("/entries", Some(token), text.to_string()).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let placed = app
        .post_as(&format!("/e/{id}/publish"), Some(token), "")
        .await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    id
}
