//! Seam: the router, with a fake model client and a browser session, for the owner's
//! review queue (design.md, Admin; Entry status): only the admin's session reaches
//! it, the owner publishes, holds or rejects an entry in review, a batch is still
//! decided as one, and the review guard is recorded and cleared.

mod common;

use axum::http::StatusCode;
use common::fake_models::FakeModels;
use common::{TestApp, browser_account, date_in, entry, field, id_in, owner, test_app_with};
use filedrawer_server::jobs::publish_due;
use time::OffsetDateTime;

const HAZARD: &str = "FAKE-HAZARD FAKE-SONNET-HAZARD";

async fn app() -> TestApp {
    test_app_with(FakeModels::default().screener(), 1_000_000_000).await
}

/// Submits `text` as the app's account and ends its hold; returns its ID and status.
async fn publish(app: &TestApp, text: &str) -> (String, String) {
    let held = app.post("/entries", text.to_string()).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    (id, field(&placed.body, "status").unwrap().to_string())
}

async fn in_review(app: &TestApp, front: &str) -> String {
    let (id, status) = publish(app, &entry(&format!("Steering {HAZARD}"), front)).await;
    assert_eq!(status, "review");
    id
}

/// The status the author's token reads for `id`.
async fn author_status(app: &TestApp, id: &str) -> String {
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    field(&own.body, "status").unwrap().to_string()
}

/// Posts an admin form and checks it went back to the queue.
async fn decide(app: &TestApp, cookie: &str, path: &str, fields: &[(&str, &str)]) {
    let reply = app.form(path, fields, Some(cookie)).await;
    assert_eq!(
        reply.status,
        StatusCode::SEE_OTHER,
        "{path}: {}",
        reply.body
    );
    assert_eq!(reply.location(), Some("/admin"));
}

#[tokio::test]
async fn only_the_admins_session_reaches_the_queue() {
    let app = app().await;
    let id = in_review(&app, "").await;

    let anonymous = app.page("/admin", None).await;
    assert_eq!(anonymous.status, StatusCode::SEE_OTHER);
    assert_eq!(anonymous.location(), Some("/login"));
    let with_token = app.get_as_author("/admin").await;
    assert_eq!(with_token.location(), Some("/login"));

    let (someone, _) = browser_account(&app, "grace@example.com").await;
    for path in ["/admin".to_string(), format!("/admin/e/{id}")] {
        let reply = app.page(&path, Some(&someone)).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
    }
    let forged = app
        .form(&format!("/admin/e/{id}/publish"), &[], Some(&someone))
        .await;
    assert_eq!(forged.status, StatusCode::NOT_FOUND);
    assert_eq!(author_status(&app, &id).await, "review");

    let cookie = owner(&app).await;
    let queue = app.page("/admin", Some(&cookie)).await;
    assert_eq!(queue.status, StatusCode::OK, "{}", queue.body);
    assert!(
        queue.body.contains(&format!("/admin/e/{id}")),
        "{}",
        queue.body
    );
    let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    // The source is shown as text, escaped.
    assert!(page.body.contains(HAZARD), "{}", page.body);
    assert!(page.body.contains("## Reproduce"), "{}", page.body);
    assert!(!page.body.contains("<h2>Reproduce"), "{}", page.body);
}

#[tokio::test]
async fn the_owners_page_shows_the_declarations_of_the_runs_an_entry_uses() {
    let app = app().await;
    let cookie = owner(&app).await;
    let body = serde_json::json!({
        "runs": [{"id": "sweep-1", "description": "Layers 10 to 18 <b>bold</b>", "n": 520,
                  "commit": "0123456789abcdef0123456789abcdef01234567",
                  "code_state": "committed", "route": "weights",
                  "dates": ["2026-10-20", "2026-10-21"]}],
        "entries": [entry(&format!("One {HAZARD}"), "slug: one\nuses_runs: [sweep-1]\n")],
    });
    let ids = batch_body(&app, body).await;
    let page = app
        .page(&format!("/admin/e/{}", ids[0]), Some(&cookie))
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("sweep-1"), "{}", page.body);
    assert!(
        page.body.contains("Layers 10 to 18 &#60;b&#62;"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("<b>bold"), "{}", page.body);
}

#[tokio::test]
async fn publishing_from_review_places_the_entry_as_a_pass_would() {
    let app = app().await;
    let cookie = owner(&app).await;
    let public = in_review(&app, "").await;
    decide(&app, &cookie, &format!("/admin/e/{public}/publish"), &[]).await;
    assert_eq!(
        app.get(&format!("/e/{public}")).await.status,
        StatusCode::OK
    );

    let later = in_review(&app, &format!("embargo_until: {}\n", date_in(30))).await;
    decide(&app, &cookie, &format!("/admin/e/{later}/publish"), &[]).await;
    assert_eq!(author_status(&app, &later).await, "embargoed");

    let queue = app.page("/admin", Some(&cookie)).await;
    assert!(
        !queue.body.contains(&format!("/admin/e/{public}\"")),
        "{}",
        queue.body
    );
}

#[tokio::test]
async fn a_held_entry_waits_until_the_owner_publishes_it() {
    let app = app().await;
    let cookie = owner(&app).await;
    let id = in_review(&app, "").await;
    decide(&app, &cookie, &format!("/admin/e/{id}/hold"), &[]).await;
    assert_eq!(author_status(&app, &id).await, "held");
    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(author_status(&app, &id).await, "held");
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let queue = app.page("/admin", Some(&cookie)).await;
    assert!(
        queue.body.contains(&format!("/admin/e/{id}")),
        "{}",
        queue.body
    );

    decide(&app, &cookie, &format!("/admin/e/{id}/publish"), &[]).await;
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_rejection_tells_the_author_only_that_review_rejected_it_and_frees_the_slug() {
    let app = app().await;
    let cookie = owner(&app).await;
    let id = in_review(&app, "slug: risky\n").await;
    let held = in_review(&app, "slug: later\n").await;
    decide(&app, &cookie, &format!("/admin/e/{held}/hold"), &[]).await;

    decide(&app, &cookie, &format!("/admin/e/{id}/reject"), &[]).await;
    decide(&app, &cookie, &format!("/admin/e/{held}/reject"), &[]).await;

    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(field(&own.body, "status"), Some("rejected"), "{}", own.body);
    let reasons: Vec<&str> = own
        .body
        .lines()
        .filter_map(|line| line.strip_prefix("reason: "))
        .collect();
    assert_eq!(reasons, vec!["rejected in review"]);
    let own = app.get_as_author(&format!("/e/{held}")).await;
    assert_eq!(field(&own.body, "status"), Some("rejected"), "{}", own.body);
    assert_eq!(field(&own.body, "reason"), Some("rejected in review"));

    let again = app.post("/entries", entry("Fine", "slug: risky\n")).await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
}

#[tokio::test]
async fn a_decision_on_an_entry_not_in_review_or_held_is_refused() {
    let app = app().await;
    let cookie = owner(&app).await;
    let (public, status) = publish(&app, &entry("Fine", "")).await;
    assert_eq!(status, "public");
    for action in ["publish", "hold", "reject"] {
        let reply = app
            .form(&format!("/admin/e/{public}/{action}"), &[], Some(&cookie))
            .await;
        assert_eq!(
            reply.status,
            StatusCode::CONFLICT,
            "{action}: {}",
            reply.body
        );
    }
    // Held can't be held again.
    let id = in_review(&app, "").await;
    decide(&app, &cookie, &format!("/admin/e/{id}/hold"), &[]).await;
    let again = app
        .form(&format!("/admin/e/{id}/hold"), &[], Some(&cookie))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    let missing = app
        .form("/admin/e/e-nothing/publish", &[], Some(&cookie))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

async fn batch(app: &TestApp, entries: &[String]) -> Vec<String> {
    batch_body(app, serde_json::json!({ "runs": [], "entries": entries })).await
}

/// Sends a batch, publishes it, and returns its members' IDs.
async fn batch_body(app: &TestApp, body: serde_json::Value) -> Vec<String> {
    let body = body.to_string();
    let reply = app
        .send(
            axum::http::Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let reply: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    let id = reply["id"].as_str().unwrap();
    let published = app.post(&format!("/batches/{id}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    reply["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| member["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_batch_publishes_once_its_last_member_in_review_is_published() {
    let app = app().await;
    let cookie = owner(&app).await;
    let ids = batch(
        &app,
        &[
            entry("Fine", "slug: fine\n"),
            entry(&format!("One {HAZARD}"), "slug: one\n"),
            entry(&format!("Two {HAZARD}"), "slug: two\n"),
        ],
    )
    .await;
    assert_eq!(author_status(&app, &ids[1]).await, "review");
    decide(&app, &cookie, &format!("/admin/e/{}/publish", ids[1]), &[]).await;
    for id in &ids {
        assert_eq!(
            app.get(&format!("/e/{id}")).await.status,
            StatusCode::NOT_FOUND,
            "{id} waits for its batch"
        );
    }
    decide(&app, &cookie, &format!("/admin/e/{}/hold", ids[2]), &[]).await;
    decide(&app, &cookie, &format!("/admin/e/{}/publish", ids[2]), &[]).await;
    for id in &ids {
        assert_eq!(
            app.get(&format!("/e/{id}")).await.status,
            StatusCode::OK,
            "{id}"
        );
    }
}

#[tokio::test]
async fn rejecting_a_batch_member_sends_back_the_rest_but_leaves_flagged_ones_to_the_owner() {
    let app = app().await;
    let cookie = owner(&app).await;
    let ids = batch(
        &app,
        &[
            entry("Fine", "slug: fine\n"),
            entry(&format!("One {HAZARD}"), "slug: one\n"),
            entry(&format!("Two {HAZARD}"), "slug: two\n"),
            entry(&format!("Three {HAZARD}"), "slug: three\n"),
        ],
    )
    .await;
    // Two is held; Three stays in review.
    decide(&app, &cookie, &format!("/admin/e/{}/hold", ids[2]), &[]).await;
    assert_eq!(author_status(&app, &ids[3]).await, "review");
    decide(&app, &cookie, &format!("/admin/e/{}/reject", ids[1]), &[]).await;
    assert_eq!(author_status(&app, &ids[0]).await, "rejected");
    assert_eq!(author_status(&app, &ids[1]).await, "rejected");
    // Flagged siblings stay for the owner to see.
    assert_eq!(author_status(&app, &ids[2]).await, "held");
    assert_eq!(author_status(&app, &ids[3]).await, "review");
    let page = app
        .page(&format!("/admin/e/{}", ids[3]), Some(&cookie))
        .await;
    assert!(
        page.body.contains("batch has already failed"),
        "{}",
        page.body
    );

    // Publishing one now sends it back with its batch.
    decide(&app, &cookie, &format!("/admin/e/{}/publish", ids[3]), &[]).await;
    let own = app.get_as_author(&format!("/e/{}", ids[3])).await;
    assert_eq!(field(&own.body, "status"), Some("rejected"), "{}", own.body);
    let reason = field(&own.body, "reason").unwrap();
    assert!(reason.starts_with("another entry in its batch"), "{reason}");
    let again = app.post("/entries", entry("Fine", "slug: fine\n")).await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
}

#[tokio::test]
async fn every_admin_write_refuses_a_token_and_another_accounts_session() {
    let app = app().await;
    let id = in_review(&app, "").await;
    let (someone, _) = browser_account(&app, "grace@example.com").await;
    for path in [
        format!("/admin/e/{id}/publish"),
        format!("/admin/e/{id}/hold"),
        format!("/admin/e/{id}/reject"),
        format!("/admin/e/{id}/doubt-seen"),
        format!("/admin/accounts/{}/clear-guard", app.account),
    ] {
        let forged = app.form(&path, &[], Some(&someone)).await;
        assert_eq!(forged.status, StatusCode::NOT_FOUND, "{path}");
        // A token, even with this site's Origin, isn't a session.
        let with_token = app
            .send(
                axum::http::Request::post(&path)
                    .header("authorization", format!("Bearer {}", app.token))
                    .header("origin", common::PUBLIC_URL)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(with_token.location(), Some("/login"), "{path}");
    }
    assert_eq!(author_status(&app, &id).await, "review");
}

#[tokio::test]
async fn the_owners_reject_or_hold_guards_the_account_until_cleared() {
    let app = app().await;
    let cookie = owner(&app).await;
    let clear = format!("/admin/accounts/{}/clear-guard", app.account);
    let guarded = async || {
        app.page("/admin", Some(&cookie))
            .await
            .body
            .contains(&clear)
    };

    // Reaching review alone doesn't, and neither does publishing from it.
    let published = in_review(&app, "").await;
    assert!(!guarded().await);
    decide(&app, &cookie, &format!("/admin/e/{published}/publish"), &[]).await;
    assert!(!guarded().await);

    let rejected = in_review(&app, "").await;
    decide(&app, &cookie, &format!("/admin/e/{rejected}/reject"), &[]).await;
    assert!(guarded().await);
    decide(&app, &cookie, &clear, &[]).await;
    assert!(!guarded().await);

    let held = in_review(&app, "").await;
    decide(&app, &cookie, &format!("/admin/e/{held}/hold"), &[]).await;
    assert!(guarded().await);
}

#[tokio::test]
async fn the_owner_sees_entries_that_passed_with_a_hazard_doubt_until_marked_seen() {
    let app = app().await;
    let cookie = owner(&app).await;
    let held = app
        .post(
            "/entries",
            entry("Steering FAKE-HAZARD FAKE-SONNET-UNSURE", ""),
        )
        .await;
    let id = id_in(&held.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
    let seen = format!("/admin/e/{id}/doubt-seen");
    let queue = app.page("/admin", Some(&cookie)).await;
    assert!(queue.body.contains(&seen), "{}", queue.body);
    assert!(queue.body.contains("working_attack"), "{}", queue.body);
    assert!(queue.body.contains("the quoted passage"), "{}", queue.body);

    decide(&app, &cookie, &seen, &[]).await;
    let queue = app.page("/admin", Some(&cookie)).await;
    assert!(!queue.body.contains(&seen), "{}", queue.body);
}
