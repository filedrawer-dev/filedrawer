//! Seam: the router, with a fake model client, and the hourly tidy, for how long a
//! rejected entry is kept (design.md, Entry lifecycle; #132): its author reads it and
//! its reasons for 60 days after the rejection, and then the server deletes it, with
//! what only it used, unless a member of its batch still waits on the screen or the
//! owner.

use crate::common;

use axum::http::StatusCode;
use common::attachments::{COMMIT, citing, set_with, upload};
use common::fake_models::FakeModels;
use common::{TestApp, entry, field, id_in, owner, test_app_with};
use filedrawer_server::jobs::{expire_rejected, publish_due};
use time::{Duration, OffsetDateTime};

const HAZARD: &str = "FAKE-HAZARD FAKE-SONNET-HAZARD";

async fn app() -> TestApp {
    test_app_with(FakeModels::default().screener(), 1_000_000_000).await
}

/// Submits `text` as the app's account, ends its hold and lets the queue place it;
/// returns its ID.
async fn screened(app: &TestApp, text: &str) -> String {
    let held = app.post("/entries", text.to_string()).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();
    id
}

async fn tidy_at(app: &TestApp, now: OffsetDateTime) {
    expire_rejected(&app.state, now).await.unwrap();
}

async fn stored(app: &TestApp, id: &str) -> bool {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE id = ?")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    count == 1
}

#[tokio::test]
async fn a_rejected_entry_is_read_by_its_author_for_60_days_then_deleted() {
    let app = app().await;
    let rejected_at = OffsetDateTime::now_utc();
    let id = screened(&app, &entry("Cake FAKE-OFFTOPIC", "")).await;

    tidy_at(&app, rejected_at + Duration::days(59)).await;
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(field(&own.body, "status"), Some("rejected"), "{}", own.body);
    assert!(field(&own.body, "reason").is_some(), "{}", own.body);

    tidy_at(
        &app,
        rejected_at + Duration::days(60) + Duration::seconds(2),
    )
    .await;
    assert!(!stored(&app, &id).await, "its text is gone");
    let gone = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(gone.status, StatusCode::GONE, "{}", gone.body);
    assert!(
        gone.body.contains(&format!("entry {id} was deleted at ")),
        "{}",
        gone.body
    );
    // Nobody else learns it existed.
    let other = app.other_token().await;
    let theirs = app.get_with_token(&format!("/e/{id}"), &other).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND, "{}", theirs.body);
}

#[tokio::test]
async fn the_60_days_count_from_the_rejection_not_the_submission() {
    // An entry can wait in review for weeks before the owner rejects it.
    let app = app().await;
    let cookie = owner(&app).await;
    let id = screened(&app, &entry(&format!("Steering {HAZARD}"), "")).await;
    sqlx::query(
        "UPDATE entries SET submitted_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-100 days'),
             first_submitted_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-100 days')
         WHERE id = ?",
    )
    .bind(&id)
    .execute(&app.pool)
    .await
    .unwrap();
    let rejected_at = OffsetDateTime::now_utc();
    let decided = app
        .form(&format!("/admin/e/{id}/reject"), &[], Some(&cookie))
        .await;
    assert_eq!(decided.status, StatusCode::SEE_OTHER, "{}", decided.body);
    publish_due(&app.state, rejected_at).await.unwrap();
    assert_eq!(
        common::author_status(&app, &app.token, &id)
            .await
            .as_deref(),
        Some("rejected")
    );

    tidy_at(&app, rejected_at + Duration::days(59)).await;
    assert!(stored(&app, &id).await, "rejected 59 days ago");
    tidy_at(
        &app,
        rejected_at + Duration::days(60) + Duration::seconds(2),
    )
    .await;
    assert!(!stored(&app, &id).await, "rejected 60 days ago");
}

/// Uploads a set holding `src/<name>` with `text` for each of `files`; returns its ID.
async fn set_of(app: &TestApp, files: &[(&str, &str)]) -> String {
    let files: Vec<serde_json::Value> = files
        .iter()
        .map(|(name, text)| {
            serde_json::json!({"origin": format!("main@{COMMIT}:src/{name}"), "text": text})
        })
        .collect();
    let set = serde_json::json!({"repos": {"main": "no remote"}, "files": files});
    let reply = upload(app, &app.token, set).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    serde_json::from_str::<serde_json::Value>(&reply.body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_rejected_entrys_attached_files_go_with_it_unless_something_public_holds_them() {
    let app = app().await;
    let rejected_at = OffsetDateTime::now_utc();
    // Its own set shares a file with a public entry's set.
    let own = set_of(&app, &[("shared.py", "shared\n"), ("lone.py", "lone\n")]).await;
    let rejected = screened(&app, &citing("Cake FAKE-OFFTOPIC", &own)).await;
    let other = set_of(&app, &[("shared.py", "shared\n"), ("kept.py", "kept\n")]).await;
    screened(&app, &citing("Steering", &other)).await;
    // A set a rejected and a public entry both cite.
    let both = set_with(&app, &app.token, "both.py", "both\n").await;
    let also_rejected = screened(&app, &citing("Pie FAKE-OFFTOPIC", &both)).await;
    screened(&app, &citing("Probing", &both)).await;
    for id in [&rejected, &also_rejected] {
        assert_eq!(
            common::author_status(&app, &app.token, id).await.as_deref(),
            Some("rejected")
        );
    }
    assert_eq!(
        common::stored_texts(&app),
        vec!["both\n", "kept\n", "lone\n", "shared\n"]
    );

    tidy_at(
        &app,
        rejected_at + Duration::days(60) + Duration::seconds(2),
    )
    .await;
    assert!(!stored(&app, &rejected).await);
    assert!(!stored(&app, &also_rejected).await);
    assert_eq!(
        common::stored_texts(&app),
        vec!["both\n", "kept\n", "shared\n"]
    );
}

#[tokio::test]
async fn a_rejected_batch_member_stays_while_another_member_waits_on_the_owner() {
    // Otherwise the member in review, published once its rejected sibling is gone,
    // would publish alone rather than go back with its batch (design.md, Admin).
    let app = app().await;
    let cookie = owner(&app).await;
    let rejected_at = OffsetDateTime::now_utc();
    let body = serde_json::json!({
        "runs": [],
        "entries": [
            // Screened first, so it is in review when its sibling fails.
            entry(&format!("Steering {HAZARD}"), "slug: steering\n"),
            entry("Cake FAKE-OFFTOPIC", "slug: cake\n"),
        ],
    });
    let sent = app
        .send(
            axum::http::Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    let reply: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    let batch = reply["id"].as_str().unwrap();
    let ids = common::batch_member_ids(&reply);
    let published = app.post(&format!("/batches/{batch}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    publish_due(&app.state, rejected_at).await.unwrap();
    let status = async |id: &str| common::author_status(&app, &app.token, id).await;
    assert_eq!(status(&ids[1]).await.as_deref(), Some("rejected"));
    assert_eq!(status(&ids[0]).await.as_deref(), Some("review"));

    tidy_at(&app, rejected_at + Duration::days(61)).await;
    assert!(stored(&app, &ids[1]).await, "its sibling waits in review");

    // Publishing the one in review sends it back with its batch; then both go.
    let decided = app
        .form(&format!("/admin/e/{}/publish", ids[0]), &[], Some(&cookie))
        .await;
    assert_eq!(decided.status, StatusCode::SEE_OTHER, "{}", decided.body);
    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(status(&ids[0]).await.as_deref(), Some("rejected"));
    tidy_at(&app, rejected_at + Duration::days(61)).await;
    assert!(!stored(&app, &ids[1]).await);
    assert!(!stored(&app, &ids[0]).await);
    // A retried deletion of the batch hears that it went, and when.
    let again = app
        .delete_as(&format!("/batches/{batch}"), &app.token, false)
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(
        field(&again.body, "status"),
        Some("deleted"),
        "{}",
        again.body
    );
    for id in &ids {
        assert!(
            again.body.contains(&format!("entry: {id}")),
            "{}",
            again.body
        );
    }
}
