//! Seam: the router, with the owner's browser session, for removing and restoring an
//! entry (design.md, Admin; #43): a removed entry is served nowhere, with no public
//! notice, its author's token reads only that it was removed, and a restore puts back
//! the status it had, unless that would fork its chain or break its batch.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::series::{citing, create, make_public, submit_with};
use common::{Reply, TestApp, browser_account, date_in, entry, field, id_in, owner, test_app};
use filedrawer_server::jobs::publish_due;
use serde_json::json;
use time::OffsetDateTime;

async fn remove(app: &TestApp, cookie: &str, id: &str) -> Reply {
    app.form(&format!("/admin/e/{id}/remove"), &[], Some(cookie))
        .await
}

async fn restore(app: &TestApp, cookie: &str, id: &str) -> Reply {
    app.form(&format!("/admin/e/{id}/restore"), &[], Some(cookie))
        .await
}

/// Posts `form` and checks it went back to the entry's admin page.
async fn done(reply: Reply, id: &str) {
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    assert_eq!(
        reply.location(),
        Some(format!("/admin/e/{id}").as_str()),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn a_removed_entry_is_served_nowhere_and_a_restore_brings_it_back() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    let kept = app.submit(&entry("Probes", "")).await;

    done(remove(&app, &cookie, &id).await, &id).await;

    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let search = app.get("/search?q=steering").await.body;
    assert!(!search.contains(&id), "{search}");
    assert!(search.contains(&kept), "{search}");
    let cards = app.get("/cards.txt").await.body;
    assert!(!cards.contains(&id), "{cards}");
    assert!(cards.contains(&kept), "{cards}");
    // Only the kept entry counts toward its tags now.
    assert_eq!(app.get("/tags").await.body, "refusal 1\nsteering 1\n");

    // Its author reads that it was removed, and where to write, but not its text.
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(field(&own.body, "status"), Some("removed"));
    assert!(own.body.contains("contact@filedrawer.dev"), "{}", own.body);
    assert!(!own.body.contains("## Reproduce"), "{}", own.body);

    let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains(&format!("/admin/e/{id}/restore")),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains(&format!("/admin/e/{id}/remove\"")),
        "{}",
        page.body
    );

    done(restore(&app, &cookie, &id).await, &id).await;
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
    assert!(app.get("/search?q=steering").await.body.contains(&id));
}

#[tokio::test]
async fn only_the_admins_session_removes_or_restores() {
    let app = test_app().await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    let (someone, _) = browser_account(&app, "grace@example.com").await;
    for path in [
        format!("/admin/e/{id}/remove"),
        format!("/admin/e/{id}/restore"),
    ] {
        let anonymous = app.form(&path, &[], None).await;
        assert_eq!(anonymous.location(), Some("/login"), "{path}");
        let forged = app.form(&path, &[], Some(&someone)).await;
        assert_eq!(forged.status, StatusCode::NOT_FOUND, "{path}");
        let with_token = app.post(&path, "").await;
        assert_ne!(with_token.status, StatusCode::SEE_OTHER, "{path}");
    }
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn removing_twice_restoring_a_kept_entry_and_unknown_ids_are_refused() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    assert_eq!(
        restore(&app, &cookie, &id).await.status,
        StatusCode::CONFLICT
    );
    done(remove(&app, &cookie, &id).await, &id).await;
    assert_eq!(
        remove(&app, &cookie, &id).await.status,
        StatusCode::CONFLICT
    );
    let unknown = "e-0000000000000000";
    assert_eq!(
        remove(&app, &cookie, unknown).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        restore(&app, &cookie, unknown).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.page(&format!("/admin/e/{unknown}"), Some(&cookie))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_removed_pending_entry_stays_removed_when_its_hold_ends_and_restores_to_pending() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let id = app.submit_held(&entry("Steering vectors", "")).await;
    done(remove(&app, &cookie, &id).await, &id).await;

    publish_due(
        &app.state,
        OffsetDateTime::now_utc() + time::Duration::days(1),
    )
    .await
    .unwrap();
    assert_eq!(
        common::author_status(&app, &app.token, &id)
            .await
            .as_deref(),
        Some("removed")
    );

    done(restore(&app, &cookie, &id).await, &id).await;
    assert_eq!(
        common::author_status(&app, &app.token, &id)
            .await
            .as_deref(),
        Some("pending")
    );
}

#[tokio::test]
async fn the_admin_page_lists_the_chains_other_versions() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let first = app.submit(&entry("Steering vectors", "")).await;
    let second = id_in(
        &app.post(
            &format!("/e/{first}/versions"),
            entry("Steering vectors, again", ""),
        )
        .await
        .body,
    );
    app.post(&format!("/e/{second}/publish"), "").await;

    let page = app.page(&format!("/admin/e/{second}"), Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains(&format!("href=\"/admin/e/{first}\"")),
        "{}",
        page.body
    );
    assert!(
        page.body.contains(&format!("/admin/e/{second}/remove")),
        "{}",
        page.body
    );
    let page = app.page(&format!("/admin/e/{first}"), Some(&cookie)).await;
    assert!(
        page.body.contains(&format!("href=\"/admin/e/{second}\"")),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn a_restore_that_would_fork_a_chain_is_refused() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let first = app.submit(&entry("Steering vectors", "")).await;
    let second = id_in(
        &app.post(&format!("/e/{first}/versions"), entry("Second", ""))
            .await
            .body,
    );
    done(remove(&app, &cookie, &second).await, &second).await;
    let third = app
        .post(&format!("/e/{first}/versions"), entry("Third", ""))
        .await;
    assert_eq!(third.status, StatusCode::CREATED, "{}", third.body);

    let refused = restore(&app, &cookie, &second).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(
        refused.body.contains(&id_in(&third.body)),
        "{}",
        refused.body
    );
    assert_eq!(
        common::author_status(&app, &app.token, &second)
            .await
            .as_deref(),
        Some("removed")
    );
}

async fn batch(app: &TestApp, token: &str) -> serde_json::Value {
    let member = |slug: &str| entry(slug, &format!("slug: {slug}\n"));
    let reply = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(Body::from(
                    json!({"entries": [member("a"), member("b")]}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    serde_json::from_str(&reply.body).unwrap()
}

#[tokio::test]
async fn an_unpublished_batch_member_is_not_restored_and_does_not_block_deleting_its_batch() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let reply = batch(&app, &app.token).await;
    let members = common::batch_member_ids(&reply);
    let page = app
        .page(&format!("/admin/e/{}", members[0]), Some(&cookie))
        .await;
    assert!(page.has_class("removal-final"), "{}", page.body);
    let single = app.submit_held(&entry("Alone", "")).await;
    let page = app.page(&format!("/admin/e/{single}"), Some(&cookie)).await;
    assert!(!page.has_class("removal-final"), "{}", page.body);
    done(remove(&app, &cookie, &members[0]).await, &members[0]).await;

    let refused = restore(&app, &cookie, &members[0]).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);

    let batch_id = reply["id"].as_str().unwrap();
    let deleted = app
        .delete_as(&format!("/batches/{batch_id}"), &app.token, false)
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(
        common::author_status(&app, &app.token, &members[1]).await,
        None
    );
    assert_eq!(
        common::author_status(&app, &app.token, &members[0])
            .await
            .as_deref(),
        Some("removed")
    );
}

#[tokio::test]
async fn the_author_withdraws_the_rest_of_a_batch_but_not_through_its_removed_member() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (author, token) = browser_account(&app, "grace@example.com").await;
    let members = common::batch_member_ids(&batch(&app, &token).await);
    done(remove(&app, &cookie, &members[0]).await, &members[0]).await;

    let through_removed = app
        .form(
            &format!("/account/e/{}/withdraw", members[0]),
            &[],
            Some(&author),
        )
        .await;
    assert_eq!(
        through_removed.status,
        StatusCode::NOT_FOUND,
        "{}",
        through_removed.body
    );
    assert_eq!(
        common::author_status(&app, &token, &members[1])
            .await
            .as_deref(),
        Some("pending")
    );

    let rest = app
        .form(
            &format!("/account/e/{}/withdraw", members[1]),
            &[],
            Some(&author),
        )
        .await;
    assert_eq!(rest.status, StatusCode::OK, "{}", rest.body);
    assert_eq!(common::author_status(&app, &token, &members[1]).await, None);
    assert_eq!(
        common::author_status(&app, &token, &members[0])
            .await
            .as_deref(),
        Some("removed")
    );
}

#[tokio::test]
async fn removing_the_entry_that_held_a_series_version_back_makes_it_readable() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (author, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    make_public(&app, &author, &series).await;
    let embargoed = submit_with(
        &app,
        &token,
        &citing(
            "Held",
            &format!("{series}@1"),
            &format!("embargo_until: {}\n", date_in(30)),
        ),
    )
    .await;
    submit_with(&app, &token, &citing("Public", &format!("{series}@1"), "")).await;
    let page = app.get(&format!("/series/{series}@1")).await;
    assert_eq!(field(&page.body, "description"), Some("not public"));

    done(remove(&app, &cookie, &embargoed).await, &embargoed).await;
    let page = app.get(&format!("/series/{series}@1")).await;
    assert!(
        page.body.contains("judge-b scores each reply"),
        "{}",
        page.body
    );
}

/// Whether an admin page offers to restore `id`.
fn offers_restore(page: &Reply, id: &str) -> bool {
    page.body
        .contains(&format!("action=\"/admin/e/{id}/restore\""))
}

#[tokio::test]
async fn an_entry_whose_account_is_deleted_stays_removed() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (author, token) = browser_account(&app, "grace@example.com").await;
    let id = submit_with(&app, &token, &entry("Steering vectors", "")).await;
    done(remove(&app, &cookie, &id).await, &id).await;
    let deleted = app
        .form(
            "/account/delete",
            &[("password", common::PASSWORD)],
            Some(&author),
        )
        .await;
    assert!(
        deleted.status.is_redirection() || deleted.status.is_success(),
        "{}",
        deleted.body
    );

    let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(!offers_restore(&page, &id), "{}", page.body);
    assert!(page.has_class("restore-refused"), "{}", page.body);
    assert_eq!(
        restore(&app, &cookie, &id).await.status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_removed_batch_member_offers_no_restore() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let members = common::batch_member_ids(&batch(&app, &app.token).await);
    done(remove(&app, &cookie, &members[0]).await, &members[0]).await;
    let page = app
        .page(&format!("/admin/e/{}", members[0]), Some(&cookie))
        .await;
    assert!(!offers_restore(&page, &members[0]), "{}", page.body);
    assert!(page.has_class("restore-refused"), "{}", page.body);
}

#[tokio::test]
async fn a_series_page_leaves_out_a_removed_entry() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (author, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    make_public(&app, &author, &series).await;
    let gone = submit_with(&app, &token, &citing("Gone", &format!("{series}@1"), "")).await;
    let kept = submit_with(&app, &token, &citing("Kept", &format!("{series}@1"), "")).await;
    assert!(
        app.get(&format!("/series/{series}"))
            .await
            .body
            .contains(&gone)
    );

    done(remove(&app, &cookie, &gone).await, &gone).await;
    let page = app.get(&format!("/series/{series}")).await.body;
    assert!(!page.contains(&gone), "{page}");
    assert!(page.contains(&kept), "{page}");
}

#[tokio::test]
async fn a_rejected_version_restores_though_another_has_replaced_its_predecessor() {
    let app = common::test_app_with(
        common::fake_models::FakeModels::default().screener(),
        1_000_000_000,
    )
    .await;
    let cookie = owner(&app).await;
    let first = app.submit(&entry("Steering vectors", "")).await;
    let second = id_in(
        &app.post(
            &format!("/e/{first}/versions"),
            entry("Steering FAKE-HAZARD FAKE-SONNET-HAZARD", ""),
        )
        .await
        .body,
    );
    let placed = app.post(&format!("/e/{second}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("review"),
        "{}",
        placed.body
    );
    let rejected = app
        .form(&format!("/admin/e/{second}/reject"), &[], Some(&cookie))
        .await;
    assert_eq!(rejected.status, StatusCode::SEE_OTHER, "{}", rejected.body);
    let third = app
        .post(&format!("/e/{first}/versions"), entry("Third", ""))
        .await;
    assert_eq!(third.status, StatusCode::CREATED, "{}", third.body);

    done(remove(&app, &cookie, &second).await, &second).await;
    let page = app.page(&format!("/admin/e/{second}"), Some(&cookie)).await;
    assert!(offers_restore(&page, &second), "{}", page.body);
    done(restore(&app, &cookie, &second).await, &second).await;
    assert_eq!(
        common::author_status(&app, &app.token, &second)
            .await
            .as_deref(),
        Some("rejected")
    );
}
