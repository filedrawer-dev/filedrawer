//! Seam: the router, with a fake model client, for the notice after publication
//! (design.md, Feedback and usability cycles): once an entry is public, its author's
//! replies name the GitHub issue form for public feedback, and never for an entry
//! rejected or with the owner, and only in production.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::fake_models::FakeModels;
use common::{Reply, TestApp, entry, field, id_in, test_app_with};
use filedrawer_server::config::Environment;
use serde_json::{Value, json};

const FORM: &str = "https://github.com/filedrawer-dev/filedrawer/issues/new?template=feedback.yml";

/// A production app, the only one that offers the form.
async fn app_with(models: &FakeModels) -> TestApp {
    app_in(models, Environment::Production).await
}

async fn app_in(models: &FakeModels, environment: Environment) -> TestApp {
    let mut app = test_app_with(models.screener(), 1_000_000_000).await;
    app.state.environment = environment;
    app.router = filedrawer_server::app(app.state.clone());
    app
}

/// Submits `text`, ends its hold, and returns its ID and the publish reply.
async fn publish(app: &TestApp, text: &str) -> (String, Reply) {
    let id = app.submit_held(text).await;
    let reply = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    (id, reply)
}

async fn json_as_author(app: &TestApp, path: &str, method: &str) -> Value {
    let reply = app
        .send(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {}", app.token))
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    serde_json::from_str(&reply.body).unwrap()
}

#[tokio::test]
async fn a_published_entrys_author_is_offered_the_issue_form() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (id, published) = publish(&app, &entry("Steering at 14", "")).await;
    assert_eq!(field(&published.body, "status"), Some("public"));
    assert_eq!(
        field(&published.body, "public_feedback"),
        Some(FORM),
        "{}",
        published.body
    );

    let mine = app.get_as_author(&format!("/e/{id}")).await.body;
    assert_eq!(field(&mine, "public_feedback"), Some(FORM), "{mine}");
    let delimiter = mine.find("=== Author text").unwrap();
    assert!(mine.find("public_feedback:").unwrap() < delimiter, "{mine}");
    let json = json_as_author(&app, &format!("/e/{id}"), "GET").await;
    assert_eq!(json["public_feedback"], FORM);
}

#[tokio::test]
async fn staging_and_dev_never_offer_it() {
    // Their authors are test agents, and a usability run that followed the notice
    // would file a real public issue.
    for environment in [Environment::Staging, Environment::Dev] {
        let models = FakeModels::default();
        let app = app_in(&models, environment).await;
        let (id, published) = publish(&app, &entry("Steering at 14", "")).await;
        assert_eq!(field(&published.body, "status"), Some("public"));
        assert!(
            !published.body.contains("public_feedback"),
            "{}",
            published.body
        );
        let mine = app.get_as_author(&format!("/e/{id}")).await.body;
        assert!(!mine.contains("public_feedback"), "{mine}");
        let json = json_as_author(&app, &format!("/e/{id}"), "GET").await;
        assert!(json["public_feedback"].is_null(), "{json}");
    }
}

#[tokio::test]
async fn a_reader_other_than_the_author_is_not_offered_it() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (id, _) = publish(&app, &entry("Steering at 14", "")).await;
    let anyone = app.get(&format!("/e/{id}")).await;
    assert_eq!(anyone.status, StatusCode::OK);
    assert!(!anyone.body.contains("public_feedback"), "{}", anyone.body);
    let stranger = app.other_token().await;
    let theirs = app.get_with_token(&format!("/e/{id}"), &stranger).await;
    assert!(!theirs.body.contains("public_feedback"), "{}", theirs.body);
    assert!(app.get_json(&format!("/e/{id}")).await["public_feedback"].is_null());
}

#[tokio::test]
async fn a_rejected_entry_or_one_with_the_owner_is_never_offered_it() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    for (marker, status) in [
        ("FAKE-OFFTOPIC", "rejected"),
        ("FAKE-HAZARD FAKE-SONNET-HAZARD", "review"),
    ] {
        let (id, placed) = publish(&app, &entry(&format!("Steering {marker}"), "")).await;
        assert_eq!(
            field(&placed.body, "status"),
            Some(status),
            "{}",
            placed.body
        );
        assert!(!placed.body.contains("public_feedback"), "{}", placed.body);
        let mine = app.get_as_author(&format!("/e/{id}")).await;
        assert_eq!(mine.status, StatusCode::OK, "{}", mine.body);
        assert!(!mine.body.contains("public_feedback"), "{}", mine.body);
    }
    // Held for the owner, then held until the vendor fixes the hole.
    let (id, _) = publish(&app, &entry("Steering FAKE-HAZARD FAKE-SONNET-HAZARD", "")).await;
    sqlx::query("UPDATE entries SET status = 'held' WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await
        .unwrap();
    let held = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(field(&held.body, "status"), Some("held"), "{}", held.body);
    assert!(!held.body.contains("public_feedback"), "{}", held.body);
}

#[tokio::test]
async fn a_still_pending_entry_is_not_offered_it() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let reply = app.post("/entries", entry("Steering at 14", "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert!(!reply.body.contains("public_feedback"), "{}", reply.body);
    let mine = app
        .get_as_author(&format!("/e/{}", id_in(&reply.body)))
        .await;
    assert!(!mine.body.contains("public_feedback"), "{}", mine.body);
}

#[tokio::test]
async fn a_published_batch_offers_it_once_and_a_rejected_one_not_at_all() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    for (marker, offered) in [("", true), ("FAKE-OFFTOPIC", false)] {
        let members: Vec<String> = ["a", "b"]
            .iter()
            .map(|slug| {
                entry(
                    &format!("Steering {marker} {slug}"),
                    &format!("slug: {slug}{marker}\n"),
                )
            })
            .collect();
        let posted = app
            .send(
                Request::post("/entries/batch")
                    .header("authorization", format!("Bearer {}", app.token))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"entries": members}).to_string()))
                    .unwrap(),
            )
            .await;
        assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
        let batch = id_in(&posted.body);
        let path = format!("/batches/{batch}/publish");
        let placed = app.post(&path, "").await;
        assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
        let lines = placed
            .body
            .lines()
            .filter(|line| *line == format!("public_feedback: {FORM}"))
            .count();
        assert_eq!(lines, usize::from(offered), "{}", placed.body);
    }
}

#[tokio::test]
async fn the_batch_publish_json_names_it_when_its_members_are_public() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let members = vec![entry("Steering a", "slug: a\n")];
    let posted = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .body(Body::from(json!({"entries": members}).to_string()))
                .unwrap(),
        )
        .await;
    let batch = id_in(&posted.body);
    let json = json_as_author(&app, &format!("/batches/{batch}/publish"), "POST").await;
    assert_eq!(json["public_feedback"], FORM, "{json}");
    let (_, single) = publish(&app, &entry("Steering b", "")).await;
    assert!(single.body.contains(FORM));
    let id = app.submit_held(&entry("Steering c", "")).await;
    let json = json_as_author(&app, &format!("/e/{id}/publish"), "POST").await;
    assert_eq!(json["public_feedback"], FORM, "{json}");
}
