//! Seam: the router, for `POST /entries` (design.md, Submission and screening; Entry
//! status; Embargo).

mod common;

use axum::http::StatusCode;
use common::{date_in, entry, test_app};

#[tokio::test]
async fn a_valid_entry_is_published_and_the_response_shows_its_card() {
    let app = test_app().await;
    let reply = app
        .post("/entries", entry("Steering does not transfer", ""))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let id = reply
        .body
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap();
    assert_eq!(id.len(), 10);
    assert!(
        id.bytes()
            .all(|b| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&b)),
        "{id}"
    );
    assert!(reply.body.contains("status: public\n"), "{}", reply.body);
    assert!(
        reply.body.contains(&format!(
            "  {id}; experiment; null; main; rows: 1 effect, 1 null"
        )),
        "{}",
        reply.body
    );
    assert!(reply.body.contains("Ada, account a-"), "{}", reply.body);
    assert!(
        reply.body.contains("primary row: model model-a;"),
        "{}",
        reply.body
    );
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_submission_without_a_valid_token_is_unauthorised() {
    let app = test_app().await;
    let none = app.post_as("/entries", None, entry("T", "")).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    let wrong = app
        .post_as("/entries", Some("fdt_nottherighttoken"), entry("T", ""))
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_refused_entry_lists_every_reason() {
    let app = test_app().await;
    let text = entry("T", "code: https://github.com/a/b/tree/main\n").replace("title: T\n", "");
    let reply = app.post("/entries", text).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        reply.body.contains("the required field `title` is missing"),
        "{}",
        reply.body
    );
    assert!(reply.body.contains("isn't pinned"), "{}", reply.body);
}

#[tokio::test]
async fn an_entry_over_the_size_cap_is_refused() {
    let app = test_app().await;
    let text = entry("T", "").replace("not recorded", &"x".repeat(110 * 1024));
    let reply = app.post("/entries", text).await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("over the cap"), "{}", reply.body);
}

#[tokio::test]
async fn warnings_come_back_and_point_to_feedback() {
    let app = test_app().await;
    let reply = app
        .post("/entries", entry("T", "compute: 3 GPU hours\n"))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert!(
        reply
            .body
            .contains("warning: `compute` is not a field the server knows"),
        "{}",
        reply.body
    );
    assert!(reply.body.contains("POST /feedback"), "{}", reply.body);
}

#[tokio::test]
async fn an_embargoed_entry_is_held_and_invisible() {
    let app = test_app().await;
    let text = entry(
        "Secret steering result",
        &format!("embargo_until: {}\ntags: [hidden-tag]\n", date_in(30)),
    )
    .replace("tags: [steering, refusal]\n", "");
    let reply = app.post("/entries", text).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert!(reply.body.contains("status: embargoed\n"), "{}", reply.body);
    assert!(
        reply.body.contains(&format!("release: {}", date_in(30))),
        "{}",
        reply.body
    );
    let id = reply
        .body
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap();
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(!app.get("/search?q=secret").await.body.contains(id));
    assert!(!app.get("/cards.txt").await.body.contains(id));
    assert!(!app.get("/tags").await.body.contains("hidden-tag"));
}

#[tokio::test]
async fn an_embargo_date_today_or_past_publishes_at_once() {
    let app = test_app().await;
    let text = entry("T", &format!("embargo_until: {}\n", date_in(0)));
    let reply = app.post("/entries", text).await;
    assert!(reply.body.contains("status: public\n"), "{}", reply.body);
}

#[tokio::test]
async fn an_embargo_past_three_months_or_not_a_date_is_refused() {
    let app = test_app().await;
    for value in [
        date_in(100),
        "next spring".to_string(),
        "2026-13-40".to_string(),
    ] {
        let reply = app
            .post("/entries", entry("T", &format!("embargo_until: {value}\n")))
            .await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{value}: {}",
            reply.body
        );
        assert!(reply.body.contains("embargo_until"), "{}", reply.body);
    }
}

#[tokio::test]
async fn label_and_fd_fields_are_not_stored() {
    let app = test_app().await;
    let id = app
        .submit(&entry("T", "fd_status: verified\nlabel: supported\n"))
        .await;
    let page = app.get(&format!("/e/{id}")).await.body;
    assert!(!page.contains("fd_status: verified"), "{page}");
    assert!(!page.contains("label: supported"), "{page}");
}

#[tokio::test]
async fn json_is_returned_when_asked_for() {
    let app = test_app().await;
    let reply = app
        .send(
            axum::http::Request::post("/entries")
                .header("authorization", format!("Bearer {}", app.token))
                .header("accept", "application/json")
                .body(axum::body::Body::from(entry("T", "compute: 1\n")))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(json["status"], "public");
    assert_eq!(json["id"].as_str().unwrap().len(), 10);
    assert!(json["card"].as_str().unwrap().contains("experiment; null"));
    assert_eq!(json["warnings"].as_array().unwrap().len(), 1);
}
