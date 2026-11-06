//! Seam: the router and `jobs::publish_due`, with a fake model client, for the fair
//! queue and the review guard (design.md, Submission and screening): every account's
//! first waiting item goes ahead of anyone's second, and an account with an entry in
//! review sends everything else it submits straight to review.

mod common;

use axum::http::StatusCode;
use common::fake_models::FakeModels;
use common::{TestApp, entry, field, id_in, owner, test_app_with};
use filedrawer_server::jobs::publish_due;
use filedrawer_server::screening::Model;
use time::{Duration, OffsetDateTime};

const HAZARD: &str = "FAKE-HAZARD FAKE-SONNET-HAZARD";

async fn app(models: &FakeModels) -> TestApp {
    test_app_with(models.screener(), 1_000_000_000).await
}

/// Submits an entry titled `title` with `token`, held as pending; returns its ID.
async fn held(app: &TestApp, token: &str, title: &str) -> String {
    let reply = app.post_as("/entries", Some(token), entry(title, "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    id_in(&reply.body)
}

/// Runs the worker as if every hold had ended.
async fn holds_end(app: &TestApp) {
    publish_due(&app.state, OffsetDateTime::now_utc() + Duration::hours(2))
        .await
        .unwrap();
}

/// The titles' markers in the order Haiku screened them.
fn screened_order(models: &FakeModels, markers: &[&str]) -> Vec<String> {
    models
        .screened()
        .iter()
        .filter_map(|text| {
            markers
                .iter()
                .find(|marker| text.contains(&format!("Steering {marker} ")))
        })
        .map(|marker| marker.to_string())
        .collect()
}

#[tokio::test]
async fn every_accounts_first_waiting_entry_is_screened_before_anyones_second() {
    let models = FakeModels::default();
    let app = app(&models).await;
    let other = app.other_token().await;
    for title in ["Steering A1 x", "Steering A2 x", "Steering A3 x"] {
        held(&app, &app.token, title).await;
    }
    held(&app, &other, "Steering B1 x").await;
    holds_end(&app).await;
    assert_eq!(
        screened_order(&models, &["A1", "A2", "A3", "B1"]),
        vec!["A1", "B1", "A2", "A3"]
    );
}

#[tokio::test]
async fn a_position_is_kept_while_the_entry_waits() {
    // A1, A2 and B1 wait past the day's cap, at positions 1, 2 and 1; B2 joins later
    // behind B1, at 2. Ties go by submission, so the order is A1, B1, A2, B2.
    let models = FakeModels::default();
    let mut app = app(&models).await;
    let other = app.other_token().await;
    held(&app, &app.token, "Steering A1 x").await;
    held(&app, &app.token, "Steering A2 x").await;
    held(&app, &other, "Steering B1 x").await;
    app.state.screening_daily_nano_usd = 0;
    holds_end(&app).await;
    assert!(models.screened().is_empty());
    held(&app, &other, "Steering B2 x").await;
    holds_end(&app).await;
    app.state.screening_daily_nano_usd = 1_000_000_000;
    holds_end(&app).await;
    assert_eq!(
        screened_order(&models, &["A1", "A2", "B1", "B2"]),
        vec!["A1", "B1", "A2", "B2"]
    );
}

async fn into_review(app: &TestApp) -> String {
    let id = held(app, &app.token, &format!("Steering {HAZARD} x")).await;
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("review"),
        "{}",
        placed.body
    );
    id
}

/// Guards the app's account the way it happens: an entry reaches review and the
/// owner holds it. Returns the owner's session.
async fn guard(app: &TestApp) -> String {
    let id = into_review(app).await;
    let cookie = owner(app).await;
    let hold = app
        .form(&format!("/admin/e/{id}/hold"), &[], Some(&cookie))
        .await;
    assert_eq!(hold.status, StatusCode::SEE_OTHER, "{}", hold.body);
    cookie
}

#[tokio::test]
async fn reaching_review_alone_doesnt_guard_the_account() {
    let models = FakeModels::default();
    let app = app(&models).await;
    into_review(&app).await;
    let id = held(&app, &app.token, "Steering fine x").await;
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
}

#[tokio::test]
async fn a_guarded_accounts_entries_go_to_review_without_a_model_call() {
    let models = FakeModels::default();
    let mut app = app(&models).await;
    guard(&app).await;
    let asked = models.asked().len();

    let waiting = held(&app, &app.token, "Steering harmless x").await;
    // Not held back by a spent cap either.
    app.state.screening_daily_nano_usd = 0;
    holds_end(&app).await;
    let own = app.get_as_author(&format!("/e/{waiting}")).await;
    assert_eq!(field(&own.body, "status"), Some("review"), "{}", own.body);
    assert_eq!(models.asked().len(), asked);

    // Publishing ends the hold the same way.
    app.state.screening_daily_nano_usd = 1_000_000_000;
    let id = held(&app, &app.token, "Steering harmless too x").await;
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("review"),
        "{}",
        placed.body
    );
    assert_eq!(models.asked().len(), asked);
}

#[tokio::test]
async fn another_account_is_screened_as_usual() {
    let models = FakeModels::default();
    let app = app(&models).await;
    guard(&app).await;
    let other = app.other_token().await;
    let id = held(&app, &other, "Steering fine x").await;
    let placed = app
        .post_as(&format!("/e/{id}/publish"), Some(&other), "")
        .await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
}

#[tokio::test]
async fn a_guarded_account_cant_store_a_series() {
    let models = FakeModels::default();
    let app = app(&models).await;
    guard(&app).await;
    let asked = models.asked().len();
    let reply = app
        .post(
            "/series",
            common::series::series_doc("Steering family", "Shared setup."),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(
        reply.body.contains("until the owner clears the account"),
        "{}",
        reply.body
    );
    assert!(!reply.body.contains("as written"), "{}", reply.body);
    assert_eq!(models.asked().len(), asked);
}

#[tokio::test]
async fn once_the_owner_clears_the_guard_the_account_is_screened_again() {
    let models = FakeModels::default();
    let app = app(&models).await;
    let cookie = guard(&app).await;
    let cleared = app
        .form(
            &format!("/admin/accounts/{}/clear-guard", app.account),
            &[],
            Some(&cookie),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::SEE_OTHER, "{}", cleared.body);
    let id = held(&app, &app.token, "Steering fine x").await;
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
    assert_eq!(models.asked().last(), Some(&Model::Haiku));
}

#[tokio::test]
async fn the_review_queue_lists_every_accounts_first_entry_before_anyones_second() {
    let models = FakeModels::default();
    let app = app(&models).await;
    let other = app.other_token().await;
    let a1 = into_review(&app).await;
    let a2 = held(&app, &app.token, &format!("Steering {HAZARD} A2")).await;
    app.post(&format!("/e/{a2}/publish"), "").await;
    let b1 = held(&app, &other, &format!("Steering {HAZARD} B1")).await;
    app.post_as(&format!("/e/{b1}/publish"), Some(&other), "")
        .await;
    let cookie = owner(&app).await;
    let queue = app.page("/admin", Some(&cookie)).await;
    let at = |id: &str| {
        queue
            .body
            .find(&format!("/admin/e/{id}\""))
            .unwrap_or_else(|| panic!("{id} isn't listed: {}", queue.body))
    };
    assert!(at(&a1) < at(&b1) && at(&b1) < at(&a2), "{}", queue.body);
}

#[tokio::test]
async fn a_held_entry_keeps_its_accounts_later_reviews_behind_other_accounts() {
    // A2 is submitted before B1 but reaches review after A1 is held. Held counts
    // toward A's place in the review queue, so B1 (A's first is held) lists before A2.
    let models = FakeModels::default();
    let app = app(&models).await;
    let other = app.other_token().await;
    let cookie = owner(&app).await;
    let a1 = into_review(&app).await;
    let a2 = held(&app, &app.token, "Steering A2 x").await;
    let hold = app
        .form(&format!("/admin/e/{a1}/hold"), &[], Some(&cookie))
        .await;
    assert_eq!(hold.status, StatusCode::SEE_OTHER, "{}", hold.body);
    let b1 = held(&app, &other, &format!("Steering {HAZARD} B1")).await;
    app.post_as(&format!("/e/{b1}/publish"), Some(&other), "")
        .await;
    app.post(&format!("/e/{a2}/publish"), "").await;
    let queue = app.page("/admin", Some(&cookie)).await;
    let at = |id: &str| queue.body.find(&format!("/admin/e/{id}\"")).unwrap();
    assert!(at(&b1) < at(&a2), "{}", queue.body);
}
