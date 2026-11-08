//! Seam: the router, as deployed, with embargoes and hidden series paused (design.md,
//! Embargo: the owner's decision, 2026-11-08). The server refuses `embargo_until`,
//! a new series is public from the start, and entries embargoed before the pause
//! keep working: a new version of one keeps its chain's date.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::series::{citing, create, submit_with};
use common::{TestApp, author_status, browser_account, date_in, entry, field, id_in, test_app};
use serde_json::json;

const PAUSED: &str = "embargoes are paused";

async fn paused_app() -> TestApp {
    test_app().await.paused()
}

fn assert_paused(reply: &common::Reply) {
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains(PAUSED), "{}", reply.body);
}

#[tokio::test]
async fn a_submission_that_sets_embargo_until_is_refused_with_only_that_reason() {
    let app = paused_app().await;
    let reply = app
        .post(
            "/entries",
            entry("Steering", &format!("embargo_until: {}\n", date_in(30))),
        )
        .await;
    assert_paused(&reply);
    // Not the embargo rules' own reasons, which would contradict it.
    let far = app
        .post("/entries", entry("Steering", "embargo_until: 2099-01-01\n"))
        .await;
    assert_paused(&far);
    assert!(!far.body.contains("3 months"), "{}", far.body);
    let unreadable = app
        .post(
            "/entries",
            entry("Steering", "embargo_until: next spring\n"),
        )
        .await;
    assert_paused(&unreadable);
    assert!(!unreadable.body.contains("guess"), "{}", unreadable.body);

    let plain = app.post("/entries", entry("Steering", "")).await;
    assert_eq!(plain.status, StatusCode::CREATED, "{}", plain.body);
}

#[tokio::test]
async fn a_batch_member_or_a_drafts_put_that_sets_embargo_until_is_refused() {
    let app = paused_app().await;
    let reply = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"entries": [
                        entry("A", "slug: a\n"),
                        entry("B", &format!("slug: b\nembargo_until: {}\n", date_in(30))),
                    ]})
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_paused(&reply);

    let id = app.submit_held(&entry("Steering", "")).await;
    let put = app
        .put_as(
            &format!("/e/{id}"),
            &app.token,
            entry("Steering", &format!("embargo_until: {}\n", date_in(30))),
        )
        .await;
    assert_paused(&put);
}

/// An entry embargoed before the pause: submitted while embargoes were open.
async fn embargoed(app: &TestApp, date: &str) -> String {
    let id = app
        .submit(&entry("Steering", &format!("embargo_until: {date}\n")))
        .await;
    assert_eq!(
        author_status(app, &app.token, &id).await.as_deref(),
        Some("embargoed")
    );
    id
}

#[tokio::test]
async fn a_version_of_an_entry_embargoed_before_the_pause_keeps_its_date() {
    let date = date_in(30);
    let app = test_app().await;
    let (left_out, repeated, moved) = (
        embargoed(&app, &date).await,
        embargoed(&app, &date).await,
        embargoed(&app, &date).await,
    );
    let app = app.paused();
    // Left out, or repeated from the stored text, the date is kept.
    for (old, front) in [
        (&left_out, String::new()),
        (&repeated, format!("embargo_until: {date}\n")),
    ] {
        let reply = app
            .post(
                &format!("/e/{old}/versions"),
                entry("Steering, corrected", &front),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::CREATED,
            "{front:?}: {}",
            reply.body
        );
        assert_eq!(field(&reply.body, "embargo_until"), Some(date.as_str()));
    }
    let other = app
        .post(
            &format!("/e/{moved}/versions"),
            entry(
                "Steering, corrected",
                &format!("embargo_until: {}\n", date_in(40)),
            ),
        )
        .await;
    assert_paused(&other);
}

#[tokio::test]
async fn a_version_of_a_released_entry_may_repeat_its_old_date() {
    // Its public text still says `embargo_until`, and an agent edits that text.
    let date = date_in(3);
    let app = test_app().await;
    let old = embargoed(&app, &date).await;
    let app = app.paused();
    filedrawer_server::jobs::release_due(
        &app.state,
        time::OffsetDateTime::now_utc() + time::Duration::days(5),
    )
    .await
    .unwrap();
    assert_eq!(app.get(&format!("/e/{old}")).await.status, StatusCode::OK);
    let reply = app
        .post(
            &format!("/e/{old}/versions"),
            entry("Steering, corrected", &format!("embargo_until: {date}\n")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let version = id_in(&reply.body);
    let placed = app.post(&format!("/e/{version}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
    // A third version, edited from the second's text, may repeat it too.
    let third = app
        .post(
            &format!("/e/{version}/versions"),
            entry(
                "Steering, corrected again",
                &format!("embargo_until: {date}\n"),
            ),
        )
        .await;
    assert_eq!(third.status, StatusCode::CREATED, "{}", third.body);
}

#[tokio::test]
async fn a_put_of_a_pending_version_keeps_the_chains_date_and_refuses_another() {
    let date = date_in(30);
    let app = test_app().await;
    let old = embargoed(&app, &date).await;
    let app = app.paused();
    let held = app
        .post(
            &format!("/e/{old}/versions"),
            entry("Steering, corrected", ""),
        )
        .await;
    let url = format!("/e/{}", id_in(&held.body));
    let kept = app
        .put_as(
            &url,
            &app.token,
            entry("Steering, corrected", &format!("embargo_until: {date}\n")),
        )
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.body);
    assert_eq!(field(&kept.body, "embargo_until"), Some(date.as_str()));
    let other = app
        .put_as(
            &url,
            &app.token,
            entry(
                "Steering, corrected",
                &format!("embargo_until: {}\n", date_in(40)),
            ),
        )
        .await;
    assert_paused(&other);
}

#[tokio::test]
async fn a_new_series_is_public_from_the_start() {
    let app = paused_app().await;
    let (_, token) = browser_account(&app, "grace@example.com").await;
    let made = app
        .post_as(
            "/series",
            Some(&token),
            common::series::series_doc("Layer sweep", common::series::DESCRIPTION),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(field(&made.body, "status"), Some("public"), "{}", made.body);
    assert!(!made.body.contains("wait"), "{}", made.body);
    let (series, _) = create(&app, &token).await;
    assert_eq!(
        app.get(&format!("/series/{series}")).await.status,
        StatusCode::OK
    );
    let id = submit_with(
        &app,
        &token,
        &citing("Steering at 14", &format!("{series}@1#models"), ""),
    )
    .await;
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}
