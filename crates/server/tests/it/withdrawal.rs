//! Seams: the account pages as a browser uses them, for withdrawing an entry that was
//! never public (design.md, Entry lifecycle; Accounts): which entries the page offers
//! to withdraw, the confirm page, a batch withdrawn as one, and that only the
//! account's session withdraws. And the screening queue with a fake model client
//! held mid-call, for an entry withdrawn while it is screened.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::deletion::state_of;
use common::fake_models::FakeModels;
use common::series::{citing, create, submit_with};
use common::{
    PUBLIC_URL, Reply, TestApp, author_status, batch_member_ids, browser_account, entry, id_in,
    owner, test_app, test_app_with,
};
use filedrawer_server::jobs::publish_due;
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

const EMAIL: &str = "grace@example.com";
const HAZARD: &str = "FAKE-HAZARD FAKE-SONNET-HAZARD";
const DOLLAR: i64 = 1_000_000_000;

/// Submits `text` with `token` and leaves it in its hour's hold; returns its ID.
async fn pending(app: &TestApp, token: &str, text: &str) -> String {
    let reply = app.post_as("/entries", Some(token), text.to_string()).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    id_in(&reply.body)
}

async fn models_app(models: &FakeModels) -> TestApp {
    test_app_with(models.screener(), DOLLAR).await
}

async fn withdraw(app: &TestApp, cookie: &str, id: &str) -> Reply {
    app.form(&format!("/account/e/{id}/withdraw"), &[], Some(cookie))
        .await
}

/// Withdraws `id`, which must be refused with a 409, and checks that nothing about
/// entries `ids` changed. Returns the reply.
async fn refused(app: &TestApp, cookie: &str, id: &str, ids: &[&str]) -> Reply {
    let before = state_of(app, ids).await;
    let reply = withdraw(app, cookie, id).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{id}: {}", reply.body);
    assert_eq!(
        state_of(app, ids).await,
        before,
        "withdrawing {id} changed something"
    );
    reply
}

/// Whether `page` holds a form that withdraws `id`, or opens its confirm page.
fn offers(page: &str, id: &str) -> bool {
    page.contains(&format!("action=\"/account/e/{id}/withdraw\""))
}

async fn status(app: &TestApp, token: &str, id: &str) -> String {
    author_status(app, token, id).await.unwrap_or_default()
}

/// Submits the batch `body` with `token` and leaves it in its hold; returns the
/// batch's ID and its members' IDs.
async fn pending_batch(app: &TestApp, token: &str, body: Value) -> (String, Vec<String>) {
    let reply = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let reply: Value = serde_json::from_str(&reply.body).unwrap();
    (
        reply["id"].as_str().unwrap().to_string(),
        batch_member_ids(&reply),
    )
}

/// Submits the batch `body` with `token` and ends its hold; returns the batch's ID
/// and its members' IDs.
async fn published_batch(app: &TestApp, token: &str, body: Value) -> (String, Vec<String>) {
    let (id, members) = pending_batch(app, token, body).await;
    let published = app
        .post_as(&format!("/batches/{id}/publish"), Some(token), "")
        .await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    (id, members)
}

#[tokio::test]
async fn the_account_page_offers_to_withdraw_only_entries_that_were_never_public() {
    let models = FakeModels::default();
    let app = models_app(&models).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let pending = pending(&app, &token, &entry("Pending one", "")).await;
    // A model outage leaves an entry in screening.
    let screening = submit_with(&app, &token, &entry("Steering FAKE-ERROR", "")).await;
    let public = submit_with(&app, &token, &entry("Public one", "")).await;
    let rejected = submit_with(&app, &token, &entry("Steering FAKE-OFFTOPIC", "")).await;
    let review = submit_with(&app, &token, &entry(&format!("Steering {HAZARD}"), "")).await;
    for (id, expected) in [
        (&pending, "pending"),
        (&screening, "screening"),
        (&public, "public"),
        (&rejected, "rejected"),
        (&review, "review"),
    ] {
        assert_eq!(status(&app, &token, id).await, expected, "{id}");
    }

    let page = app.page("/account", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    for id in [&pending, &screening] {
        assert!(offers(&page.body, id), "{id}: {}", page.body);
    }
    for id in [&public, &rejected, &review] {
        assert!(!offers(&page.body, id), "{id}: {}", page.body);
    }
    assert!(page.body.contains("Pending one"), "{}", page.body);
}

#[tokio::test]
async fn withdrawing_from_the_confirm_page_deletes_the_entry() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let id = pending(&app, &token, &entry("Held back", "")).await;
    let path = format!("/account/e/{id}/withdraw");
    let confirm = app.page(&path, Some(&cookie)).await;
    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.body);
    assert!(confirm.body.contains("Held back"), "{}", confirm.body);
    assert!(
        confirm.body.contains(r#"<p class="final">"#),
        "{}",
        confirm.body
    );
    assert!(
        confirm
            .body
            .contains(&format!("method=\"post\" action=\"{path}\"")),
        "{}",
        confirm.body
    );

    let done = withdraw(&app, &cookie, &id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let read = app.get_with_token(&format!("/e/{id}"), &token).await;
    assert_eq!(read.status, StatusCode::GONE, "{}", read.body);
    let page = app.page("/account", Some(&cookie)).await;
    assert!(!offers(&page.body, &id), "{}", page.body);
}

#[tokio::test]
async fn only_the_accounts_browser_session_withdraws() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let held = app
        .post_as("/entries", Some(&token), entry("Draft", "slug: draft\n"))
        .await;
    let id = id_in(&held.body);
    let path = format!("/account/e/{id}/withdraw");
    let before = state_of(&app, &[&id]).await;
    let to_login = format!("/login?next=%2Faccount%2Fe%2F{id}%2Fwithdraw");
    assert_eq!(
        app.page(&path, None).await.location(),
        Some(to_login.as_str())
    );
    assert_eq!(app.form(&path, &[], None).await.location(), Some("/login"));
    let by_token = app
        .send(
            Request::post(&path)
                .header("authorization", format!("Bearer {token}"))
                .header("origin", PUBLIC_URL)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(by_token.location(), Some("/login"), "{}", by_token.body);
    let forged = app
        .send(
            Request::post(&path)
                .header("cookie", &cookie)
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN, "{}", forged.body);
    let (theirs, _) = browser_account(&app, "mallory@example.com").await;
    assert_eq!(
        app.page(&path, Some(&theirs)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.form(&path, &[], Some(&theirs)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(state_of(&app, &[&id]).await, before);
}

#[tokio::test]
async fn entries_with_the_owner_or_past_screening_are_not_withdrawn() {
    let models = FakeModels::default();
    let app = models_app(&models).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let public = submit_with(&app, &token, &entry("Public one", "")).await;
    let rejected = submit_with(&app, &token, &entry("Steering FAKE-OFFTOPIC", "")).await;
    let hazard = entry(&format!("Steering {HAZARD}"), "");
    let review = submit_with(&app, &token, &hazard).await;
    let passed = submit_with(&app, &token, &hazard).await;
    let held = submit_with(&app, &token, &hazard).await;
    let frozen = id_in(
        &app.post_as("/entries", Some(&token), entry("Frozen", ""))
            .await
            .body,
    );
    let failed = id_in(
        &app.post_as("/entries", Some(&token), entry("Failed", ""))
            .await
            .body,
    );
    let admin = owner(&app).await;
    let hold = app
        .form(&format!("/admin/e/{held}/hold"), &[], Some(&admin))
        .await;
    assert_eq!(hold.status, StatusCode::SEE_OTHER, "{}", hold.body);
    // States that last only until the queue's next tick, or that need a revoked
    // invite code: the owner's pass of an entry in review, the owner's rejection of
    // a frozen entry, and the freeze itself.
    for (id, statement) in [
        (
            &passed,
            "UPDATE entries SET status = 'screening', screen_verdict = 'pass' WHERE id = ?",
        ),
        (
            &failed,
            "UPDATE entries SET status = 'screening', screen_verdict = 'reject' WHERE id = ?",
        ),
        (
            &frozen,
            "UPDATE entries SET status = 'frozen', hold_until = NULL WHERE id = ?",
        ),
    ] {
        sqlx::query(statement)
            .bind(id)
            .execute(&app.pool)
            .await
            .unwrap();
    }

    let page = app.page("/account", Some(&cookie)).await;
    for id in [
        &public, &rejected, &review, &passed, &held, &frozen, &failed,
    ] {
        assert!(!offers(&page.body, id), "{id}: {}", page.body);
        let before = status(&app, &token, id).await;
        let confirm = app
            .page(&format!("/account/e/{id}/withdraw"), Some(&cookie))
            .await;
        assert!(
            !confirm.body.contains("method=\"post\""),
            "{id}: {}",
            confirm.body
        );
        refused(&app, &cookie, id, &[id]).await;
        assert_eq!(status(&app, &token, id).await, before, "{id}");
    }
}

#[tokio::test]
async fn a_batch_waits_whole_while_a_member_is_with_the_owner() {
    let models = FakeModels::default();
    let app = models_app(&models).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let body = json!({"entries": [
        entry("Member a", "slug: a\n"),
        entry(&format!("Member b {HAZARD}"), "slug: b\n"),
    ]});
    let (_, members) = published_batch(&app, &token, body).await;
    assert_eq!(status(&app, &token, &members[0]).await, "screening");
    assert_eq!(status(&app, &token, &members[1]).await, "review");
    let page = app.page("/account", Some(&cookie)).await;
    assert!(offers(&page.body, &members[0]), "{}", page.body);
    let all: Vec<&str> = members.iter().map(String::as_str).collect();
    let reply = refused(&app, &cookie, &members[0], &all).await;
    assert!(reply.body.contains(&members[1]), "{}", reply.body);
    assert!(reply.body.contains("owner"), "{}", reply.body);
}

#[tokio::test]
async fn a_batch_member_is_withdrawn_with_its_whole_batch() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let two = |a: &str, b: &str| {
        json!({"entries": [
            entry(&format!("Member {a}"), &format!("slug: {a}\n")),
            entry(&format!("Member {b}"), &format!("slug: {b}\n")),
        ]})
    };
    let (batch, members) = pending_batch(&app, &token, two("a", "b")).await;
    let confirm = app
        .page(
            &format!("/account/e/{}/withdraw", members[0]),
            Some(&cookie),
        )
        .await;
    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.body);
    assert!(confirm.body.contains(&batch), "{}", confirm.body);
    for member in &members {
        assert!(confirm.body.contains(member.as_str()), "{}", confirm.body);
    }
    let done = withdraw(&app, &cookie, &members[0]).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    for member in &members {
        let read = app.get_with_token(&format!("/e/{member}"), &token).await;
        assert_eq!(read.status, StatusCode::GONE, "{}", read.body);
    }
}

#[tokio::test]
async fn the_confirm_page_names_text_the_withdrawal_would_make_public() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let run = json!({"id": "r-1", "description": "Sweep over layers 10 to 18.", "n": 520,
                     "commit": "0123456789abcdef0123456789abcdef01234567",
                     "code_state": "committed", "route": "weights",
                     "dates": ["2026-10-20", "2026-10-21"]});
    let (_, public) = published_batch(
        &app,
        &token,
        json!({"runs": [run], "entries": [entry("Public", "slug: a\nuses_runs: [r-1]\n")]}),
    )
    .await;
    let run_user = pending(&app, &token, &entry("Run user", "uses_runs: [r-1]\n")).await;
    let (series, _) = create(&app, &token).await;
    let cite = format!("{series}@1");
    let series_user = pending(&app, &token, &citing("Series user", &cite, "")).await;
    submit_with(&app, &token, &citing("Cites it too", &cite, "")).await;

    let confirm = app
        .page(&format!("/account/e/{run_user}/withdraw"), Some(&cookie))
        .await;
    assert!(
        confirm.body.contains("Sweep over layers 10 to 18."),
        "{}",
        confirm.body
    );
    assert!(confirm.body.contains("r-1"), "{}", confirm.body);
    let confirm = app
        .page(&format!("/account/e/{series_user}/withdraw"), Some(&cookie))
        .await;
    assert!(confirm.body.contains(&cite), "{}", confirm.body);
    assert!(confirm.body.contains("Layer sweep"), "{}", confirm.body);

    for id in [&run_user, &series_user] {
        assert_eq!(withdraw(&app, &cookie, id).await.status, StatusCode::OK);
    }
    let read = app.get(&format!("/e/{}", public[0])).await;
    assert!(
        read.body.contains("Sweep over layers 10 to 18."),
        "{}",
        read.body
    );
    let version = app.get(&format!("/series/{cite}")).await;
    assert!(
        version.body.contains("judge-b scores each reply"),
        "{}",
        version.body
    );
}

#[tokio::test]
async fn an_entry_withdrawn_while_screened_is_dropped_its_cost_kept_and_the_queue_goes_on() {
    let models = FakeModels::default();
    let app = models_app(&models).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let slow = app
        .post_as("/entries", Some(&token), entry("Steering FAKE-SLOW", ""))
        .await;
    let slow = id_in(&slow.body);
    let next = app
        .post_as("/entries", Some(&token), entry("Steering at 14", ""))
        .await;
    let next = id_in(&next.body);
    // The hour runs out; the worker screens the slow one first, and waits on it.
    let worker = tokio::spawn({
        let state = app.state.clone();
        async move { publish_due(&state, OffsetDateTime::now_utc() + Duration::hours(2)).await }
    });
    models.wait_until_asked(1).await;
    assert_eq!(status(&app, &token, &slow).await, "screening");
    let withdrawn = withdraw(&app, &cookie, &slow).await;
    assert_eq!(withdrawn.status, StatusCode::OK, "{}", withdrawn.body);
    models.release();
    let tick = worker.await.unwrap();
    assert!(tick.is_ok(), "{tick:?}");

    let read = app.get_with_token(&format!("/e/{slow}"), &token).await;
    assert_eq!(read.status, StatusCode::GONE, "{}", read.body);
    assert_eq!(status(&app, &token, &next).await, "public");
    let (calls, unattached): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(entry_seq IS NULL), 0) FROM screenings")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(
        (calls, unattached),
        (2, 1),
        "both calls' costs are kept, the withdrawn entry's with no entry"
    );
}

#[tokio::test]
async fn the_account_page_lists_only_the_accounts_own_entries() {
    let app = test_app().await;
    let (_, author) = browser_account(&app, "author@example.com").await;
    let held = app
        .post_as("/entries", Some(&author), entry("Zettel-draft-41", ""))
        .await;
    let theirs = id_in(&held.body);
    let (cookie, token) = browser_account(&app, "mallory@example.com").await;

    let page = app.page("/account", Some(&cookie)).await;

    assert!(!offers(&page.body, &theirs), "{}", page.body);
    assert!(!page.body.contains("Zettel-draft-41"), "{}", page.body);

    let held = app
        .post_as("/entries", Some(&token), entry("Mine", ""))
        .await;
    let mine = id_in(&held.body);
    let page = app.page("/account", Some(&cookie)).await;

    assert!(offers(&page.body, &mine), "{}", page.body);
    assert!(!offers(&page.body, &theirs), "{}", page.body);
    assert!(!page.body.contains("Zettel-draft-41"), "{}", page.body);
}
