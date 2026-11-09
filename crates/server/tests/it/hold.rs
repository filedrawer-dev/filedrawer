//! Seam: the router and `jobs::publish_due`, for the hour an accepted submission
//! waits so its author can fix it (design.md, Entry status: `pending`).

use crate::common;

use axum::http::StatusCode;
use common::{date_in, entry, field, id_in, test_app};
use filedrawer_server::jobs::publish_due;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

fn publish_at(body: &str) -> OffsetDateTime {
    let text = field(body, "publish_at").unwrap_or_else(|| panic!("no publish_at in\n{body}"));
    OffsetDateTime::parse(text, &Rfc3339).unwrap()
}

#[tokio::test]
async fn a_submission_is_held_for_an_hour_and_shown_only_to_its_author() {
    let app = test_app().await;
    let before = OffsetDateTime::now_utc();
    let reply = app
        .post("/entries", entry("Held steering result", "compute: 3h\n"))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(field(&reply.body, "status"), Some("pending"));
    let at = publish_at(&reply.body);
    // Stored to the second, so up to a second before `before + 60 min`.
    let hour = Duration::minutes(60);
    assert!(
        at > before + hour - Duration::seconds(1) && at < before + Duration::minutes(61),
        "{at}"
    );
    let id = id_in(&reply.body);

    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let stranger = app.other_token().await;
    let theirs = app.get_with_token(&format!("/e/{id}"), &stranger).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    let mine = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.body);
    assert_eq!(field(&mine.body, "status"), Some("pending"));
    assert_eq!(publish_at(&mine.body), at);
    assert!(
        mine.body.contains("warning: `compute` is not a field"),
        "{}",
        mine.body
    );

    assert!(!app.get("/search?q=held").await.body.contains(&id));
    assert!(!app.get("/cards.txt").await.body.contains(&id));
    assert!(!app.get("/tags").await.body.contains("steering"));
}

#[tokio::test]
async fn a_put_replaces_the_held_entry_and_restarts_the_hour() {
    let app = test_app().await;
    let first = app
        .post("/entries", entry("First title", "compute: 3h\n"))
        .await;
    let id = id_in(&first.body);
    let first_at = publish_at(&first.body);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let reply = app
        .put_as(
            &format!("/e/{id}"),
            &app.token,
            entry("Fixed title", "").replace("[steering, refusal]", "[probes]"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(id_in(&reply.body), id);
    assert_eq!(field(&reply.body, "status"), Some("pending"));
    assert!(publish_at(&reply.body) > first_at);
    assert!(!reply.body.contains("warning:"), "{}", reply.body);

    let published = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&published.body, "status"),
        Some("public"),
        "{}",
        published.body
    );
    let page = app.get(&format!("/e/{id}")).await.body;
    assert!(page.contains("title: Fixed title"), "{page}");
    assert!(app.get("/search?q=fixed").await.body.contains(&id));
    assert!(!app.get("/search?q=first").await.body.contains(&id));
    assert_eq!(app.get("/tags").await.body, "probes 1\n");
}

#[tokio::test]
async fn a_refused_put_leaves_the_held_entry_as_it_was() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Kept title", "")).await;
    let reply = app
        .put_as(
            &format!("/e/{id}"),
            &app.token,
            entry("T", "").replace("title: T\n", ""),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    let mine = app.get_as_author(&format!("/e/{id}")).await.body;
    assert!(mine.contains("title: Kept title"), "{mine}");
}

#[tokio::test]
async fn only_the_author_may_put_or_publish() {
    let app = test_app().await;
    let id = app.submit_held(&entry("T", "")).await;
    let stranger = app.other_token().await;
    let put = app
        .put_as(&format!("/e/{id}"), &stranger, entry("Hijacked", ""))
        .await;
    assert_eq!(put.status, StatusCode::NOT_FOUND);
    let publish = app
        .post_as(&format!("/e/{id}/publish"), Some(&stranger), "")
        .await;
    assert_eq!(publish.status, StatusCode::NOT_FOUND);
    let unauthenticated = app.post_as(&format!("/e/{id}/publish"), None, "").await;
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_public_entry_takes_no_put_and_no_second_publish() {
    let app = test_app().await;
    let id = app.submit(&entry("T", "")).await;
    let put = app
        .put_as(&format!("/e/{id}"), &app.token, entry("U", ""))
        .await;
    assert_eq!(put.status, StatusCode::CONFLICT, "{}", put.body);
    assert!(put.body.contains("public"), "{}", put.body);
    let again = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(again.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn publishing_an_embargoed_entry_embargoes_it_and_its_author_sees_the_release_date() {
    let app = test_app().await;
    let id = app
        .submit_held(&entry("T", &format!("embargo_until: {}\n", date_in(30))))
        .await;
    let reply = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&reply.body, "status"),
        Some("embargoed"),
        "{}",
        reply.body
    );
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let mine = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(mine.status, StatusCode::OK);
    assert_eq!(field(&mine.body, "status"), Some("embargoed"));
    assert_eq!(field(&mine.body, "release"), Some(date_in(30).as_str()));
}

#[tokio::test]
async fn the_hour_running_out_publishes_the_entry_by_itself() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Unattended", "")).await;
    let now = OffsetDateTime::now_utc();
    assert_eq!(
        publish_due(&app.state, now + Duration::minutes(30))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        publish_due(&app.state, now + Duration::minutes(61))
            .await
            .unwrap(),
        1
    );
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
    assert!(app.get("/search?q=unattended").await.body.contains(&id));
    assert_eq!(
        publish_due(&app.state, now + Duration::minutes(62))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_draft_may_drop_or_shorten_its_own_embargo() {
    // Nothing has been held back yet, so a first submission's draft may move or drop
    // its own embargo (the owner's decision, 2026-11-05).
    let app = test_app().await;
    let id = app
        .submit_held(&entry("T", &format!("embargo_until: {}\n", date_in(30))))
        .await;
    let url = format!("/e/{id}");
    let sooner = app
        .put_as(
            &url,
            &app.token,
            entry("T", &format!("embargo_until: {}\n", date_in(2))),
        )
        .await;
    assert_eq!(sooner.status, StatusCode::OK, "{}", sooner.body);
    let dropped = app.put_as(&url, &app.token, entry("T", "")).await;
    assert_eq!(dropped.status, StatusCode::OK, "{}", dropped.body);
    let published = app.post(&format!("{url}/publish"), "").await;
    assert_eq!(
        field(&published.body, "status"),
        Some("public"),
        "{}",
        published.body
    );
}

#[tokio::test]
async fn a_pending_version_of_an_embargoed_entry_keeps_its_embargo_and_lists_every_reason() {
    // design.md, Agent interface: "resubmit without the embargo" would otherwise be
    // an early release by token.
    let app = test_app().await;
    let embargoed = app
        .submit(&entry("T", &format!("embargo_until: {}\n", date_in(30))))
        .await;
    let version = id_in(
        &app.post(
            &format!("/e/{embargoed}/versions"),
            entry("U", &format!("embargo_until: {}\n", date_in(30))),
        )
        .await
        .body,
    );
    let url = format!("/e/{version}");
    // A version keeps its chain's date (design.md, Embargo): another date, earlier or
    // later, is refused with the chain's date.
    for other in [date_in(10), date_in(60)] {
        let reply = app
            .put_as(
                &url,
                &app.token,
                entry("U", &format!("embargo_until: {other}\n")),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{other}: {}",
            reply.body
        );
        assert!(reply.body.contains(&date_in(30)), "{other}: {}", reply.body);
    }
    let text = entry("U", &format!("embargo_until: {}\n", date_in(3))).replace("title: U\n", "");
    let both = app.put_as(&url, &app.token, text).await;
    assert_eq!(both.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(both.body.contains("`title` is missing"), "{}", both.body);
    assert!(both.body.contains(&date_in(30)), "{}", both.body);
    // Leaving the date out keeps it; so does repeating it.
    for kept in [String::new(), format!("embargo_until: {}\n", date_in(30))] {
        let reply = app.put_as(&url, &app.token, entry("U", &kept)).await;
        assert_eq!(reply.status, StatusCode::OK, "{kept}: {}", reply.body);
        assert_eq!(
            field(&reply.body, "embargo_until"),
            Some(date_in(30).as_str()),
            "{}",
            reply.body
        );
    }
}
