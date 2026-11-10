//! Seam: the router, with the owner's browser session, for removing and restoring a
//! series (design.md, Admin; #57): a removed series answers 404 on every route, its
//! title and description show nowhere, its members' entries stay as they are, and a
//! restore brings it back.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::series::{citing, create, make_public, submit_with};
use common::{Reply, TestApp, browser_account, field, id_in, owner, test_app};
use filedrawer_server::admin;
use serde_json::json;

async fn remove(app: &TestApp, cookie: &str, id: &str) -> Reply {
    app.form(&format!("/admin/series/{id}/remove"), &[], Some(cookie))
        .await
}

async fn restore(app: &TestApp, cookie: &str, id: &str) -> Reply {
    app.form(&format!("/admin/series/{id}/restore"), &[], Some(cookie))
        .await
}

/// Checks `reply` went back to the series' admin page.
fn done(reply: &Reply, id: &str) {
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    assert_eq!(
        reply.location(),
        Some(format!("/admin/series/{id}").as_str()),
        "{}",
        reply.body
    );
}

/// The entry's page as a browser gets it.
async fn entry_page(app: &TestApp, id: &str) -> Reply {
    app.send(
        Request::get(format!("/e/{id}"))
            .header("sec-fetch-dest", "document")
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

/// A public series made by a browser account for `email`, with a public entry citing
/// its version 1: the account's cookie and token, the series and the entry.
async fn public_series_with_an_entry(
    app: &TestApp,
    email: &str,
) -> (String, String, String, String) {
    let (cookie, token) = browser_account(app, email).await;
    let (series, _) = create(app, &token).await;
    make_public(app, &cookie, &series).await;
    let entry = submit_with(app, &token, &citing("First", &format!("{series}@1"), "")).await;
    (cookie, token, series, entry)
}

#[tokio::test]
async fn a_removed_series_shows_nowhere_and_a_restore_brings_it_back() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (author, token, series, entry) =
        public_series_with_an_entry(&app, "grace@example.com").await;
    let account = common::account_id(&app, &author).await;

    // The creator's admin page links the series' admin page, which shows its text.
    let page = app
        .page(&format!("/admin/accounts/{account}"), Some(&owner_cookie))
        .await;
    assert!(
        page.body
            .contains(&format!("href=\"/admin/series/{series}\"")),
        "{}",
        page.body
    );
    let page = app
        .page(&format!("/admin/series/{series}"), Some(&owner_cookie))
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("Layer sweep"), "{}", page.body);
    assert!(
        page.body.contains("judge-b scores each reply"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains(&format!("href=\"/admin/e/{entry}\"")),
        "{}",
        page.body
    );

    done(&remove(&app, &owner_cookie, &series).await, &series);

    for path in [
        format!("/series/{series}"),
        format!("/series/{series}@1"),
        format!("/series/{series}.txt"),
    ] {
        assert_eq!(app.get(&path).await.status, StatusCode::NOT_FOUND, "{path}");
        let creator = app.get_with_token(&path, &token).await;
        assert_eq!(creator.status, StatusCode::NOT_FOUND, "{path}");
    }

    // The entry stays, its cite with it, but the series' text shows nowhere.
    let read = app.get(&format!("/e/{entry}")).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(
        field(&read.body, "series"),
        Some(format!("{series}@1").as_str())
    );
    assert_eq!(field(&read.body, "series_description"), Some("not public"));
    assert!(!read.body.contains("Layer sweep"), "{}", read.body);
    let json = app.get_json(&format!("/e/{entry}")).await;
    assert_eq!(json["series"]["id"], json!(series));
    assert!(json["series"]["title"].is_null(), "{json}");
    assert!(json["series"]["description"].is_null(), "{json}");
    let page = entry_page(&app, &entry).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(!page.has_class("series"), "{}", page.body);
    assert!(!page.body.contains("Layer sweep"), "{}", page.body);

    // Its creator's account page says the archive's owner removed it.
    let own = app.page("/account", Some(&author)).await;
    assert!(own.has_class("series-removed"), "{}", own.body);
    assert!(own.body.contains("contact@filedrawer.dev"), "{}", own.body);

    let page = app
        .page(&format!("/admin/series/{series}"), Some(&owner_cookie))
        .await;
    assert!(
        page.body
            .contains(&format!("action=\"/admin/series/{series}/restore\"")),
        "{}",
        page.body
    );
    done(&restore(&app, &owner_cookie, &series).await, &series);
    let back = app.get(&format!("/series/{series}")).await;
    assert_eq!(back.status, StatusCode::OK, "{}", back.body);
    assert!(
        back.body.contains("judge-b scores each reply"),
        "{}",
        back.body
    );
    let read = app.get(&format!("/e/{entry}")).await;
    assert!(read.body.contains("Layer sweep"), "{}", read.body);
    assert!(entry_page(&app, &entry).await.has_class("series"));
    let own = app.page("/account", Some(&author)).await;
    assert!(!own.has_class("series-removed"), "{}", own.body);
}

#[tokio::test]
async fn every_series_route_answers_404_for_a_removed_series() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (_, token) = browser_account(&app, "grace@example.com").await;
    let (series, key) = create(&app, &token).await;
    let member_id = admin::create_account(&app.pool, "Mallory").await.unwrap();
    let member = admin::create_token(&app.pool, &member_id, "member")
        .await
        .unwrap();
    let joined = app
        .post_as(
            &format!("/series/{series}/join"),
            Some(&member),
            key.clone(),
        )
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    let entry = submit_with(
        &app,
        &member,
        &citing("Member's", &format!("{series}@1"), ""),
    )
    .await;

    done(&remove(&app, &owner_cookie, &series).await, &series);

    let third = common::series::third_token(&app).await;
    let doc = common::series::series_doc("Layer sweep", "Version two.");
    for (path, by, body) in [
        (format!("/series/{series}/join"), &third, key.clone()),
        (format!("/series/{series}/versions"), &token, doc),
        (format!("/series/{series}/key"), &token, String::new()),
        (
            format!("/series/{series}/members/{member_id}/remove"),
            &token,
            String::new(),
        ),
        (
            format!("/series/{series}/entries/{entry}/remove"),
            &token,
            String::new(),
        ),
    ] {
        let reply = app.post_as(&path, Some(by), body).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{path}: {}",
            reply.body
        );
    }
    for by in [&token, &member] {
        let read = app.get_with_token(&format!("/series/{series}"), by).await;
        assert_eq!(read.status, StatusCode::NOT_FOUND, "{}", read.body);
    }
}

#[tokio::test]
async fn citing_a_removed_series_is_refused_as_citing_one_the_account_isnt_in() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (_, token, series, _) = public_series_with_an_entry(&app, "grace@example.com").await;
    let pending = app
        .post_as("/entries", Some(&token), common::entry("Pending", ""))
        .await;
    assert_eq!(pending.status, StatusCode::CREATED, "{}", pending.body);
    let pending = id_in(&pending.body);
    let outsider = app.other_token().await;
    let not_member = app
        .post_as(
            "/entries",
            Some(&outsider),
            citing("Outsider", &format!("{series}@1"), ""),
        )
        .await;
    assert_eq!(not_member.status, StatusCode::UNPROCESSABLE_ENTITY);

    done(&remove(&app, &owner_cookie, &series).await, &series);

    let new = app
        .post_as(
            "/entries",
            Some(&token),
            citing("Second", &format!("{series}@1"), ""),
        )
        .await;
    assert_eq!(new.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", new.body);
    assert_eq!(new.body, not_member.body);
    let put = app
        .put_as(
            &format!("/e/{pending}"),
            &token,
            citing("Pending, again", &format!("{series}@1"), ""),
        )
        .await;
    assert_eq!(put.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", put.body);
    assert!(put.body.contains("isn't a member of"), "{}", put.body);
}

/// Whether version `version` of `series` is marked readable.
async fn readable(app: &TestApp, series: &str, version: i64) -> bool {
    sqlx::query_scalar(
        "SELECT v.readable_at IS NOT NULL FROM series_versions v
         JOIN series s ON s.seq = v.series_seq WHERE s.id = ? AND v.version = ?",
    )
    .bind(series)
    .bind(version)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_removed_series_versions_stay_unreadable_until_it_is_restored() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (_, token, series, _) = public_series_with_an_entry(&app, "grace@example.com").await;
    let added = app
        .post_as(
            &format!("/series/{series}/versions"),
            Some(&token),
            common::series::series_doc("Layer sweep", "Version two."),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    let held = app
        .post_as(
            "/entries",
            Some(&token),
            citing("Second", &format!("{series}@2"), ""),
        )
        .await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let second = id_in(&held.body);

    done(&remove(&app, &owner_cookie, &series).await, &series);
    let placed = app
        .post_as(&format!("/e/{second}/publish"), Some(&token), "")
        .await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
    assert!(!readable(&app, &series, 2).await);

    done(&restore(&app, &owner_cookie, &series).await, &series);
    assert!(readable(&app, &series, 2).await);
}

#[tokio::test]
async fn an_entry_waiting_on_a_removed_hidden_series_stops_waiting() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (_, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let entry = submit_with(&app, &token, &citing("Waits", &format!("{series}@1"), "")).await;
    assert_eq!(
        common::author_status(&app, &token, &entry).await.as_deref(),
        Some("waiting_on_series")
    );

    done(&remove(&app, &owner_cookie, &series).await, &series);
    assert_eq!(app.get(&format!("/e/{entry}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_removed_hidden_series_doesnt_stop_its_creator_deleting_the_account_and_outlives_it() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (author, token) = browser_account(&app, "grace@example.com").await;
    let (series, key) = create(&app, &token).await;
    let member = app.other_token().await;
    app.post_as(&format!("/series/{series}/join"), Some(&member), key)
        .await;
    let entry = submit_with(
        &app,
        &member,
        &citing("Member's", &format!("{series}@1"), ""),
    )
    .await;
    let page = app.page("/account/delete", Some(&author)).await;
    assert!(page.body.contains(&series), "{}", page.body);

    done(&remove(&app, &owner_cookie, &series).await, &series);
    let page = app.page("/account/delete", Some(&author)).await;
    assert!(!page.body.contains(&series), "{}", page.body);
    let deleted = app
        .form(
            "/account/delete",
            &[("password", common::PASSWORD)],
            Some(&author),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);

    // The removed series stays for the owner, and the member's entry keeps its cite.
    let page = app
        .page(&format!("/admin/series/{series}"), Some(&owner_cookie))
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert_eq!(
        field(&app.get(&format!("/e/{entry}")).await.body, "series"),
        Some(format!("{series}@1").as_str())
    );
}

#[tokio::test]
async fn its_creator_cant_make_a_removed_hidden_series_public() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (author, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    done(&remove(&app, &owner_cookie, &series).await, &series);

    let made = make_public(&app, &author, &series).await;
    assert_ne!(made.status, StatusCode::SEE_OTHER, "{}", made.body);
    done(&restore(&app, &owner_cookie, &series).await, &series);
    let read = app.get(&format!("/series/{series}")).await;
    assert_eq!(read.status, StatusCode::NOT_FOUND, "{}", read.body);
}

#[tokio::test]
async fn a_restore_waits_until_the_creators_suspension_is_lifted() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (author, _, series, _) = public_series_with_an_entry(&app, "grace@example.com").await;
    let account = common::account_id(&app, &author).await;
    done(&remove(&app, &owner_cookie, &series).await, &series);
    app.form(
        &format!("/admin/accounts/{account}/suspend"),
        &[],
        Some(&owner_cookie),
    )
    .await;

    let refused = restore(&app, &owner_cookie, &series).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(refused.body.contains("suspended"), "{}", refused.body);
    let page = app
        .page(&format!("/admin/series/{series}"), Some(&owner_cookie))
        .await;
    assert!(page.has_class("restore-refused"), "{}", page.body);

    app.form(
        &format!("/admin/accounts/{account}/lift"),
        &[],
        Some(&owner_cookie),
    )
    .await;
    done(&restore(&app, &owner_cookie, &series).await, &series);
}

#[tokio::test]
async fn only_the_admins_session_reaches_a_series_admin_page() {
    let app = test_app().await;
    let (someone, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let page = format!("/admin/series/{series}");
    assert_eq!(
        app.page(&page, Some(&someone)).await.status,
        StatusCode::NOT_FOUND
    );
    for path in [format!("{page}/remove"), format!("{page}/restore")] {
        let anonymous = app.form(&path, &[], None).await;
        assert_eq!(anonymous.location(), Some("/login"), "{path}");
        let forged = app.form(&path, &[], Some(&someone)).await;
        assert_eq!(forged.status, StatusCode::NOT_FOUND, "{path}");
        let with_token = app.post_as(&path, Some(&token), "").await;
        assert_ne!(with_token.status, StatusCode::SEE_OTHER, "{path}");
    }
    let read = app
        .get_with_token(&format!("/series/{series}"), &token)
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
}

#[tokio::test]
async fn removing_twice_restoring_a_kept_series_and_unknown_ids_are_refused() {
    let app = test_app().await;
    let owner_cookie = owner(&app).await;
    let (_, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    assert_eq!(
        restore(&app, &owner_cookie, &series).await.status,
        StatusCode::CONFLICT
    );
    done(&remove(&app, &owner_cookie, &series).await, &series);
    assert_eq!(
        remove(&app, &owner_cookie, &series).await.status,
        StatusCode::CONFLICT
    );
    let unknown = "s-0000000000000000";
    for reply in [
        remove(&app, &owner_cookie, unknown).await,
        restore(&app, &owner_cookie, unknown).await,
        app.page(&format!("/admin/series/{unknown}"), Some(&owner_cookie))
            .await,
    ] {
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
    }
}
