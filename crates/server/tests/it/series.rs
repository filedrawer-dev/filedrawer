//! Seam: the router and `jobs::publish_due`, for series (design.md, Series): create,
//! join, cite a version, versions of the description, made public from a browser
//! session, and `waiting_on_series`.

use crate::common;

use axum::http::StatusCode;
use common::series::{DESCRIPTION, citing, create, make_public, series_doc, submit_with};
use common::{DELIMITER, TestApp, browser_account, date_in, entry, field, id_in, test_app};
use filedrawer_server::jobs::publish_due;
use serde_json::json;
use time::Duration;

#[tokio::test]
async fn a_new_series_is_hidden_from_all_but_its_members() {
    let app = test_app().await;
    let reply = app
        .post("/series", series_doc("Layer sweep", DESCRIPTION))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let id = id_in(&reply.body);
    assert!(id.starts_with("s-"), "{id}");
    assert_eq!(field(&reply.body, "status"), Some("hidden"));
    assert_eq!(field(&reply.body, "version"), Some("1"));
    assert!(field(&reply.body, "key").unwrap().starts_with("fdj_"));
    let (_, below) = reply.body.split_once(DELIMITER).unwrap();
    assert!(below.contains("Layer sweep"), "{}", reply.body);

    assert_eq!(
        app.get(&format!("/series/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let other = app.other_token().await;
    assert_eq!(
        app.get_with_token(&format!("/series/{id}"), &other)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let own = app.get_as_author(&format!("/series/{id}")).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(field(&own.body, "status"), Some("hidden"));
    assert!(
        own.body.contains("judge-b scores each reply"),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn a_series_document_needs_a_title_and_a_description() {
    let app = test_app().await;
    for (body, needle) in [
        ("no front matter at all".to_string(), "title"),
        ("---\ntitle: Sweep\n---\n\n".to_string(), "description"),
        ("---\nname: Sweep\n---\n\nText.".to_string(), "only `title`"),
        (
            format!("---\ntitle: Sweep\n---\n\n{}", "x".repeat(102_401)),
            "102400",
        ),
    ] {
        let reply = app.post("/series", body).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
        assert!(reply.body.contains(needle), "{needle}: {}", reply.body);
    }
    assert_eq!(
        app.post_as("/series", None, series_doc("Sweep", "Text."))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn only_members_cite_a_series_and_the_key_makes_a_member() {
    let app = test_app().await;
    let (id, key) = create(&app, &app.token).await;
    let other = app.other_token().await;
    let cite = format!("{id}@1");

    let refused = app
        .post_as("/entries", Some(&other), citing("Teammate", &cite, ""))
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.body
    );
    assert!(refused.body.contains("join"), "{}", refused.body);

    let wrong = app
        .post_as(&format!("/series/{id}/join"), Some(&other), "fdj_wrong")
        .await;
    assert_eq!(wrong.status, StatusCode::NOT_FOUND, "{}", wrong.body);
    let unknown = app
        .post_as("/series/s-0000000000/join", Some(&other), key.clone())
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body, wrong.body, "a wrong key looks like no series");

    let joined = app
        .post_as(
            &format!("/series/{id}/join"),
            Some(&other),
            format!("{key}\n"),
        )
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    let again = app
        .post_as(&format!("/series/{id}/join"), Some(&other), key.clone())
        .await;
    assert_eq!(again.status, StatusCode::OK, "joining twice is harmless");

    let accepted = app
        .post_as("/entries", Some(&other), citing("Teammate", &cite, ""))
        .await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    assert_eq!(
        app.get_with_token(&format!("/series/{id}"), &other)
            .await
            .status,
        StatusCode::OK,
        "a member reads the hidden series"
    );
}

#[tokio::test]
async fn a_cite_must_name_a_version_and_a_heading_that_exist() {
    let app = test_app().await;
    let (id, _) = create(&app, &app.token).await;
    for (cite, needle) in [
        (id.clone(), "<id>@<version>"),
        (format!("{id}@0"), "<id>@<version>"),
        (format!("{id}@2"), "latest is 1"),
        (format!("{id}@1#method"), "method"),
    ] {
        let reply = app.post("/entries", citing("Bad cite", &cite, "")).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{cite}: {}",
            reply.body
        );
        assert!(reply.body.contains(needle), "{cite}: {}", reply.body);
    }
    let reply = app
        .post(
            "/entries",
            citing("Good cite", &format!("{id}@1#prompts"), ""),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[tokio::test]
async fn an_entry_waits_on_a_hidden_series_and_publishes_when_its_creator_makes_it_public() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let held = app
        .post_as(
            "/entries",
            Some(&token),
            citing("Steering at 14", &format!("{series}@1#models"), ""),
        )
        .await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let placed = app
        .post_as(&format!("/e/{id}/publish"), Some(&token), "")
        .await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    assert_eq!(field(&placed.body, "status"), Some("waiting_on_series"));
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let own = app.get_with_token(&format!("/e/{id}"), &token).await;
    assert_eq!(
        field(&own.body, "status"),
        Some("waiting_on_series"),
        "{}",
        own.body
    );
    assert_eq!(
        field(&own.body, "series"),
        Some(format!("{series}@1#models").as_str())
    );
    assert!(app.get("/cards.txt").await.body.is_empty());

    // The account page lists the series with its button, and the POST makes it public.
    let page = app.page("/account", Some(&cookie)).await;
    assert!(page.body.contains(&series), "{}", page.body);
    assert!(
        page.body
            .contains(&format!("/account/series/{series}/publish")),
        "{}",
        page.body
    );
    let made = make_public(&app, &cookie, &series).await;
    assert_eq!(made.status, StatusCode::SEE_OTHER, "{}", made.body);

    let read = app.get(&format!("/e/{id}")).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(
        field(&read.body, "series"),
        Some(format!("{series}@1#models").as_str())
    );
    let (above, below) = read.body.split_once(DELIMITER).unwrap();
    assert!(!above.contains("not public"), "{}", read.body);
    // Only the cited section, its subsection included, and the series' title.
    assert!(below.contains("Layer sweep"), "{}", read.body);
    assert!(
        below.contains("520 prompts from the sweep set"),
        "{}",
        read.body
    );
    assert!(
        !below.contains("judge-b scores each reply"),
        "{}",
        read.body
    );
    // The stored entry no longer carries the field.
    assert!(!below.contains("series: s-"), "{}", read.body);
    let json = app.get_json(&format!("/e/{id}")).await;
    assert_eq!(json["series"]["id"], json!(series));
    assert_eq!(json["series"]["version"], json!(1));

    let page = app.get(&format!("/series/{series}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert_eq!(field(&page.body, "status"), Some("public"));
    assert!(page.body.contains(&format!("entry: {id}")), "{}", page.body);
    assert!(
        page.body.contains("judge-b scores each reply"),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn making_a_series_public_needs_its_creators_browser_session() {
    let app = test_app().await;
    let (_, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;

    let by_token = app
        .post_as(
            &format!("/account/series/{series}/publish"),
            Some(&token),
            "",
        )
        .await;
    assert_ne!(by_token.status, StatusCode::SEE_OTHER, "{}", by_token.body);
    let no_session = make_public(&app, "fd_session=nothing", &series).await;
    assert_eq!(no_session.location(), Some("/login"));

    let (mallory, _) = browser_account(&app, "mallory@example.com").await;
    let not_theirs = make_public(&app, &mallory, &series).await;
    assert_eq!(
        not_theirs.status,
        StatusCode::NOT_FOUND,
        "{}",
        not_theirs.body
    );

    assert_eq!(
        app.get(&format!("/series/{series}")).await.status,
        StatusCode::NOT_FOUND,
        "the series is still hidden"
    );
}

#[tokio::test]
async fn a_new_version_stays_unreadable_until_every_entry_citing_it_is_public() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (series, key) = create(&app, &token).await;
    let first = app
        .post_as(
            "/entries",
            Some(&token),
            citing("First", &format!("{series}@1"), ""),
        )
        .await;
    let first = id_in(&first.body);
    app.post_as(&format!("/e/{first}/publish"), Some(&token), "")
        .await;
    make_public(&app, &cookie, &series).await;

    // A member who isn't the creator can't make a version.
    let other = app.other_token().await;
    app.post_as(&format!("/series/{series}/join"), Some(&other), key)
        .await;
    let refused = app
        .post_as(
            &format!("/series/{series}/versions"),
            Some(&other),
            series_doc("Hijacked", "Other text."),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);

    let second = app
        .post_as(
            &format!("/series/{series}/versions"),
            Some(&token),
            series_doc("Layer sweep, embargoed half", "The secret method."),
        )
        .await;
    assert_eq!(second.status, StatusCode::CREATED, "{}", second.body);
    assert_eq!(field(&second.body, "version"), Some("2"));
    let embargoed = app
        .post_as(
            "/entries",
            Some(&token),
            citing(
                "Second",
                &format!("{series}@2"),
                &format!("embargo_until: {}\n", date_in(30)),
            ),
        )
        .await;
    assert_eq!(embargoed.status, StatusCode::CREATED, "{}", embargoed.body);
    let embargoed = id_in(&embargoed.body);
    let placed = app
        .post_as(&format!("/e/{embargoed}/publish"), Some(&token), "")
        .await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("embargoed"),
        "{}",
        placed.body
    );

    let page = app.get(&format!("/series/{series}")).await;
    assert_eq!(field(&page.body, "version"), Some("1"), "{}", page.body);
    assert!(!page.body.contains("secret method"), "{}", page.body);
    let v2 = app.get(&format!("/series/{series}@2")).await;
    assert_eq!(v2.status, StatusCode::OK, "{}", v2.body);
    assert_eq!(
        field(&v2.body, "description"),
        Some("not public"),
        "{}",
        v2.body
    );
    assert!(!v2.body.contains("secret method"), "{}", v2.body);
    assert!(!v2.body.contains("embargoed half"), "{}", v2.body);
    let v1 = app.get(&format!("/series/{series}@1")).await;
    assert!(v1.body.contains("judge-b scores each reply"), "{}", v1.body);
    // The creator still reads it.
    let own = app
        .get_with_token(&format!("/series/{series}@2"), &token)
        .await;
    assert!(own.body.contains("secret method"), "{}", own.body);
    // The public entry keeps showing the version it cites.
    let read = app.get(&format!("/e/{first}")).await;
    assert!(
        read.body.contains("judge-b scores each reply"),
        "{}",
        read.body
    );
}

#[tokio::test]
async fn a_batch_waits_as_a_whole_on_a_hidden_series() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let body = json!({"entries": [
        citing("Cites the series", &format!("{series}@1"), "slug: cites\n"),
        entry("Cites nothing", "slug: plain\n"),
    ]});
    let reply = app
        .send(
            axum::http::Request::post("/entries/batch")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let ids = common::batch_member_ids(&serde_json::from_str(&reply.body).unwrap());

    publish_due(
        &app.state,
        time::OffsetDateTime::now_utc() + Duration::minutes(61),
    )
    .await
    .unwrap();
    for id in &ids {
        let own = app.get_with_token(&format!("/e/{id}"), &token).await;
        assert_eq!(
            field(&own.body, "status"),
            Some("waiting_on_series"),
            "{}",
            own.body
        );
    }

    make_public(&app, &cookie, &series).await;
    for id in &ids {
        assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
    }
}

#[tokio::test]
async fn publish_due_places_waiting_entries_whose_series_is_public() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let held = app
        .post_as(
            "/entries",
            Some(&token),
            citing("Waits", &format!("{series}@1"), ""),
        )
        .await;
    let id = id_in(&held.body);
    app.post_as(&format!("/e/{id}/publish"), Some(&token), "")
        .await;
    // Made public with the entry already waiting, then the job runs.
    make_public(&app, &cookie, &series).await;
    publish_due(&app.state, time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_put_can_change_or_drop_the_series() {
    let app = test_app().await;
    let (series, _) = create(&app, &app.token).await;
    let held = app
        .post("/entries", citing("Draft", &format!("{series}@1"), ""))
        .await;
    let id = id_in(&held.body);
    let dropped = app
        .put_as(&format!("/e/{id}"), &app.token, entry("Draft", ""))
        .await;
    assert_eq!(dropped.status, StatusCode::OK, "{}", dropped.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
}

/// A public series whose version 1 a public entry cites, made by a browser account
/// for `email`: its cookie, token and ID.
async fn public_series_with_an_entry(app: &TestApp, email: &str) -> (String, String, String) {
    let (cookie, token) = browser_account(app, email).await;
    let (series, _) = create(app, &token).await;
    let held = app
        .post_as(
            "/entries",
            Some(&token),
            citing("First", &format!("{series}@1"), ""),
        )
        .await;
    let id = id_in(&held.body);
    app.post_as(&format!("/e/{id}/publish"), Some(&token), "")
        .await;
    make_public(app, &cookie, &series).await;
    (cookie, token, series)
}

#[tokio::test]
async fn a_version_no_public_entry_cites_stays_unreadable() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (empty, _) = create(&app, &token).await;
    make_public(&app, &cookie, &empty).await;
    let page = app.get(&format!("/series/{empty}")).await;
    assert_eq!(
        field(&page.body, "description"),
        Some("not public"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("judge-b"), "{}", page.body);

    let (_, token, series) = public_series_with_an_entry(&app, "ada@example.com").await;
    let second = app
        .post_as(
            &format!("/series/{series}/versions"),
            Some(&token),
            series_doc("Embargoed family", "The secret method."),
        )
        .await;
    assert_eq!(second.status, StatusCode::CREATED, "{}", second.body);
    let v2 = app.get(&format!("/series/{series}@2")).await;
    assert_eq!(
        field(&v2.body, "description"),
        Some("not public"),
        "{}",
        v2.body
    );
    assert!(!v2.body.contains("secret method"), "{}", v2.body);
    let page = app.get(&format!("/series/{series}")).await;
    assert_eq!(field(&page.body, "version"), Some("1"), "{}", page.body);
}

#[tokio::test]
async fn a_readable_version_stays_readable() {
    let app = test_app().await;
    let (_, token, series) = public_series_with_an_entry(&app, "grace@example.com").await;
    let later = app
        .post_as(
            "/entries",
            Some(&token),
            citing(
                "Later",
                &format!("{series}@1"),
                &format!("embargo_until: {}\n", date_in(30)),
            ),
        )
        .await;
    let later = id_in(&later.body);
    app.post_as(&format!("/e/{later}/publish"), Some(&token), "")
        .await;
    let v1 = app.get(&format!("/series/{series}@1")).await;
    assert!(v1.body.contains("judge-b scores each reply"), "{}", v1.body);
}

#[tokio::test]
async fn a_section_anchor_may_be_longer_than_a_slug() {
    let app = test_app().await;
    let heading = "a".repeat(80);
    let reply = app
        .post(
            "/series",
            series_doc("Long", &format!("## {heading}\n\nText.\n")),
        )
        .await;
    let id = id_in(&reply.body);
    let cited = app
        .post(
            "/entries",
            citing("Long cite", &format!("{id}@1#{heading}"), ""),
        )
        .await;
    assert_eq!(cited.status, StatusCode::CREATED, "{}", cited.body);
}

/// The account ID the account page shows for a session.
async fn account_id(app: &TestApp, cookie: &str) -> String {
    let page = app.page("/account", Some(cookie)).await;
    let start = page
        .body
        .find("<code>a-")
        .expect("the page shows the account ID")
        + 6;
    let end = start + page.body[start..].find('<').unwrap();
    page.body[start..end].to_string()
}

#[tokio::test]
async fn the_creator_rotates_the_key_and_the_old_one_stops_working() {
    let app = test_app().await;
    let (series, old) = create(&app, &app.token).await;
    let other = app.other_token().await;
    let refused = app
        .post_as(&format!("/series/{series}/key"), Some(&other), "")
        .await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
    app.post_as(&format!("/series/{series}/join"), Some(&other), old.clone())
        .await;
    let member = app
        .post_as(&format!("/series/{series}/key"), Some(&other), "")
        .await;
    assert_eq!(member.status, StatusCode::FORBIDDEN, "{}", member.body);

    let rotated = app.post(&format!("/series/{series}/key"), "").await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
    let new = field(&rotated.body, "key").unwrap().to_string();
    assert!(new.starts_with("fdj_") && new != old, "{}", rotated.body);

    let third = common::series::third_token(&app).await;
    let with_old = app
        .post_as(&format!("/series/{series}/join"), Some(&third), old)
        .await;
    assert_eq!(with_old.status, StatusCode::NOT_FOUND);
    let with_new = app
        .post_as(&format!("/series/{series}/join"), Some(&third), new)
        .await;
    assert_eq!(with_new.status, StatusCode::OK, "{}", with_new.body);
}

#[tokio::test]
async fn a_removed_member_can_no_longer_cite_but_keeps_its_entries_in_the_series() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (series, key) = create(&app, &token).await;
    let (other_cookie, other) = browser_account(&app, "ada@example.com").await;
    let other_id = account_id(&app, &other_cookie).await;
    app.post_as(&format!("/series/{series}/join"), Some(&other), key.clone())
        .await;
    let listed = app
        .get_with_token(&format!("/series/{series}"), &token)
        .await;
    assert!(
        listed.body.contains(&format!("member: {other_id}")),
        "{}",
        listed.body
    );
    let theirs = submit_with(&app, &other, &citing("Theirs", &format!("{series}@1"), "")).await;
    make_public(&app, &cookie, &series).await;

    let removed = app
        .post_as(
            &format!("/series/{series}/members/{other_id}/remove"),
            Some(&token),
            "",
        )
        .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);
    // Removing a member rotates the key, so the key it kept doesn't let it back in.
    let new_key = field(&removed.body, "key").expect("the reply shows the new key");
    assert_ne!(new_key, key);
    let rejoin = app
        .post_as(&format!("/series/{series}/join"), Some(&other), key.clone())
        .await;
    assert_eq!(rejoin.status, StatusCode::NOT_FOUND, "{}", rejoin.body);
    let refused = app
        .post_as(
            "/entries",
            Some(&other),
            citing("Again", &format!("{series}@1"), ""),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.body
    );
    let page = app.get(&format!("/series/{series}")).await;
    assert!(
        page.body.contains(&format!("entry: {theirs}")),
        "{}",
        page.body
    );

    let own_id = account_id(&app, &cookie).await;
    let itself = app
        .post_as(
            &format!("/series/{series}/members/{own_id}/remove"),
            Some(&token),
            "",
        )
        .await;
    assert_eq!(itself.status, StatusCode::CONFLICT, "{}", itself.body);
    let unknown = app
        .post_as(
            &format!("/series/{series}/members/a-0000000000/remove"),
            Some(&token),
            "",
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_entry_taken_out_of_a_hidden_series_stops_waiting_and_keeps_its_cite() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (series, key) = create(&app, &token).await;
    let other = app.other_token().await;
    app.post_as(&format!("/series/{series}/join"), Some(&other), key)
        .await;
    let waiting = submit_with(
        &app,
        &other,
        &citing("Waits", &format!("{series}@1#models"), ""),
    )
    .await;
    let stays = submit_with(&app, &token, &citing("Stays", &format!("{series}@1"), "")).await;

    let by_member = app
        .post_as(
            &format!("/series/{series}/entries/{stays}/remove"),
            Some(&other),
            "",
        )
        .await;
    assert_eq!(
        by_member.status,
        StatusCode::FORBIDDEN,
        "{}",
        by_member.body
    );
    let removed = app
        .post_as(
            &format!("/series/{series}/entries/{waiting}/remove"),
            Some(&token),
            "",
        )
        .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);

    // It no longer waits, and its page keeps the cite while the series is hidden.
    let read = app.get(&format!("/e/{waiting}")).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(
        field(&read.body, "series"),
        Some(format!("{series}@1#models").as_str())
    );
    assert_eq!(field(&read.body, "series_description"), Some("not public"));
    assert!(!read.body.contains("Layer sweep"), "{}", read.body);

    make_public(&app, &cookie, &series).await;
    let page = app.get(&format!("/series/{series}")).await;
    assert!(
        page.body.contains(&format!("entry: {stays}")),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains(&format!("entry: {waiting}")),
        "{}",
        page.body
    );
    let read = app.get(&format!("/e/{waiting}")).await;
    assert!(read.body.contains("520 prompts"), "{}", read.body);

    let unknown = app
        .post_as(
            &format!("/series/{series}/entries/0000000000/remove"),
            Some(&token),
            "",
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_entry_taken_out_of_a_series_no_longer_holds_its_version_back() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    make_public(&app, &cookie, &series).await;
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

    app.post_as(
        &format!("/series/{series}/entries/{embargoed}/remove"),
        Some(&token),
        "",
    )
    .await;
    let page = app.get(&format!("/series/{series}@1")).await;
    assert!(
        page.body.contains("judge-b scores each reply"),
        "{}",
        page.body
    );
}
