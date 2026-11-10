//! Each entry records the skill version and User-Agent of the request that wrote its
//! text, for the owner to read through SQL (#45). Nothing shows them.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, batch_member_ids, entry, id_in, test_app};
use serde_json::json;

const SKILL: &str = "filedrawer-submit/7";
const AGENT: &str = "curl/8.5.0 (an agent's shell)";

/// The `skill` and `user_agent` stored for entry `id`.
async fn provenance(app: &TestApp, id: &str) -> (Option<String>, Option<String>) {
    sqlx::query_as("SELECT skill, user_agent FROM entries WHERE id = ?")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// A request to `path` with this app's token and the two headers, when given.
fn request(
    app: &TestApp,
    method: &str,
    path: &str,
    content_type: &str,
    skill: Option<&str>,
    agent: Option<&str>,
) -> axum::http::request::Builder {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {}", app.token))
        .header("content-type", content_type);
    if let Some(skill) = skill {
        request = request.header("x-filedrawer-skill", skill);
    }
    if let Some(agent) = agent {
        request = request.header("user-agent", agent);
    }
    request
}

async fn submit_with(app: &TestApp, skill: Option<&str>, agent: Option<&str>) -> String {
    let reply = app
        .send(
            request(app, "POST", "/entries", "text/markdown", skill, agent)
                .body(Body::from(entry("Provenance", "")))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    id_in(&reply.body)
}

#[tokio::test]
async fn a_submission_records_its_skill_and_user_agent() {
    let app = test_app().await;
    let id = submit_with(&app, Some(SKILL), Some(AGENT)).await;
    assert_eq!(
        provenance(&app, &id).await,
        (Some(SKILL.into()), Some(AGENT.into()))
    );
}

#[tokio::test]
async fn a_submission_without_the_headers_records_neither() {
    let app = test_app().await;
    let id = submit_with(&app, None, None).await;
    assert_eq!(provenance(&app, &id).await, (None, None));
}

#[tokio::test]
async fn any_lowercase_name_with_a_number_is_a_skill() {
    let app = test_app().await;
    for skill in ["filedrawer-search/4", "a/1", "some-future-skill2/123456789"] {
        let id = submit_with(&app, Some(skill), None).await;
        assert_eq!(provenance(&app, &id).await.0.as_deref(), Some(skill));
    }
}

#[tokio::test]
async fn a_skill_header_not_shaped_name_slash_number_is_stored_as_absent() {
    let app = test_app().await;
    let long_name = format!("{}/1", "a".repeat(65));
    for skill in [
        "",
        "filedrawer-submit",
        "filedrawer-submit/",
        "/7",
        "filedrawer-submit/v7",
        "filedrawer-submit/7 ",
        " filedrawer-submit/7",
        "Filedrawer-Submit/7",
        "filedrawer_submit/7",
        "filedrawer-submit/7/8",
        "filedrawer-submit/1234567890",
        "filedrawer-submit/-7",
        long_name.as_str(),
    ] {
        let id = submit_with(&app, Some(skill), None).await;
        assert_eq!(provenance(&app, &id).await.0, None, "{skill:?}");
    }
    let id = submit_with(&app, Some(&format!("{}/1", "a".repeat(64))), None).await;
    assert!(
        provenance(&app, &id).await.0.is_some(),
        "64 characters is a name"
    );
}

#[tokio::test]
async fn a_user_agent_is_cut_to_200_characters_and_an_empty_one_is_absent() {
    let app = test_app().await;
    let long = "é".repeat(250);
    let id = submit_with(&app, None, Some(&long)).await;
    assert_eq!(
        provenance(&app, &id).await.1,
        Some("é".repeat(200)),
        "cut by characters, not bytes"
    );
    let id = submit_with(&app, None, Some("")).await;
    assert_eq!(provenance(&app, &id).await.1, None);
}

#[tokio::test]
async fn a_put_records_the_skill_and_agent_that_wrote_the_new_text() {
    let app = test_app().await;
    let id = submit_with(&app, Some("filedrawer-submit/6"), Some("old agent")).await;
    let reply = app
        .send(
            request(
                &app,
                "PUT",
                &format!("/e/{id}"),
                "text/markdown",
                Some(SKILL),
                Some(AGENT),
            )
            .body(Body::from(entry("Provenance, fixed", "")))
            .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        provenance(&app, &id).await,
        (Some(SKILL.into()), Some(AGENT.into()))
    );
}

#[tokio::test]
async fn a_new_version_records_its_own_skill_and_agent() {
    let app = test_app().await;
    let old = app.submit(&entry("Provenance", "")).await;
    let reply = app
        .send(
            request(
                &app,
                "POST",
                &format!("/e/{old}/versions"),
                "text/markdown",
                Some(SKILL),
                Some(AGENT),
            )
            .body(Body::from(entry("Provenance, v2", "")))
            .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let new = id_in(&reply.body);
    assert_eq!(
        provenance(&app, &new).await,
        (Some(SKILL.into()), Some(AGENT.into()))
    );
    assert_eq!(provenance(&app, &old).await, (None, None));
}

#[tokio::test]
async fn each_batch_member_records_the_batch_requests_skill_and_agent() {
    let app = test_app().await;
    let body = json!({"entries": [
        entry("First", "slug: first\n"),
        entry("Second", "slug: second\n"),
    ]});
    let reply = app
        .send(
            request(
                &app,
                "POST",
                "/entries/batch",
                "application/json",
                Some(SKILL),
                Some(AGENT),
            )
            .header("accept", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let ids = batch_member_ids(&serde_json::from_str(&reply.body).unwrap());
    assert_eq!(ids.len(), 2);
    for id in ids {
        assert_eq!(
            provenance(&app, &id).await,
            (Some(SKILL.into()), Some(AGENT.into()))
        );
    }
}

#[tokio::test]
async fn neither_shows_to_the_author_or_on_the_public_entry() {
    let app = test_app().await;
    let agent = "agent-marker-7Qz";
    let reply = app
        .send(
            request(
                &app,
                "POST",
                "/entries",
                "text/markdown",
                Some("marker-skill/9"),
                Some(agent),
            )
            .body(Body::from(entry("Provenance", "")))
            .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let id = id_in(&reply.body);
    let mut seen = vec![reply.body.clone()];
    seen.push(app.get_as_author(&format!("/e/{id}")).await.body);
    seen.push(app.get_as_author("/entries/mine").await.body);
    app.post(&format!("/e/{id}/publish"), "").await;
    seen.push(app.get(&format!("/e/{id}")).await.body);
    seen.push(app.get_json(&format!("/e/{id}")).await.to_string());
    seen.push(app.get("/search?q=Provenance").await.body);
    for body in seen {
        assert!(!body.contains(agent), "{body}");
        assert!(!body.contains("marker-skill"), "{body}");
    }
}

#[tokio::test]
async fn headers_that_are_not_utf8_are_stored_as_absent() {
    let app = test_app().await;
    let not_utf8 = axum::http::HeaderValue::from_bytes(b"filedrawer-submit/7\xff").unwrap();
    let reply = app
        .send(
            request(&app, "POST", "/entries", "text/markdown", None, None)
                .header("x-filedrawer-skill", not_utf8.clone())
                .header("user-agent", not_utf8)
                .body(Body::from(entry("Provenance", "")))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(provenance(&app, &id_in(&reply.body)).await, (None, None));
}
