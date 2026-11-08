//! Seam: the router, for `POST /e/<id>/versions` (design.md, Entry lifecycle).

mod common;

use axum::http::StatusCode;
use common::{date_in, entry, field, id_in, test_app};

const WITHOUT_TABLE: &str = "---
title: Now part of a paper
safety_motivation: Made up for a test.
hypothesis: Steering at layer 14 raises refusals.
outcome: no_effect
published_in: arXiv:2507.01410
---

## Setup

See the paper.

## Reproduce

See the paper.
";

#[tokio::test]
async fn a_version_replaces_its_entry_in_search_once_it_is_public() {
    let app = test_app().await;
    let old = app.submit(&entry("Steering first try", "")).await;
    let reply = app
        .post(
            &format!("/e/{old}/versions"),
            entry("Steering corrected", ""),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(field(&reply.body, "status"), Some("pending"));
    assert_eq!(field(&reply.body, "previous"), Some(old.as_str()));
    let new = id_in(&reply.body);
    assert_ne!(new, old);

    // Held: the old version is still the one search shows.
    assert!(app.get("/search?q=steering").await.body.contains(&old));
    let published = app.post(&format!("/e/{new}/publish"), "").await;
    assert_eq!(
        field(&published.body, "status"),
        Some("public"),
        "{}",
        published.body
    );

    let search = app.get("/search?q=steering").await.body;
    assert!(search.contains(&new) && !search.contains(&old), "{search}");
    let cards = app.get("/cards.txt").await.body;
    assert!(cards.contains(&new) && !cards.contains(&old), "{cards}");

    let page = app.get(&format!("/e/{old}")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(field(&page.body, "superseded_by"), Some(new.as_str()));
    assert!(
        page.body.contains(&format!("superseded by {new}")),
        "{}",
        page.body
    );
    let newer = app.get(&format!("/e/{new}")).await.body;
    assert_eq!(field(&newer, "previous"), Some(old.as_str()));
}

#[tokio::test]
async fn only_the_author_may_post_a_version() {
    let app = test_app().await;
    let old = app.submit(&entry("T", "")).await;
    let stranger = app.other_token().await;
    let reply = app
        .post_as(
            &format!("/e/{old}/versions"),
            Some(&stranger),
            entry("U", ""),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_pending_entry_takes_a_put_not_a_version() {
    let app = test_app().await;
    let held = app.submit_held(&entry("T", "")).await;
    let reply = app
        .post(&format!("/e/{held}/versions"), entry("U", ""))
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
    assert!(reply.body.contains("PUT"), "{}", reply.body);
}

#[tokio::test]
async fn a_version_goes_on_the_latest_version_only() {
    let app = test_app().await;
    let first = app.submit(&entry("T", "")).await;
    let second = id_in(
        &app.post(&format!("/e/{first}/versions"), entry("U", ""))
            .await
            .body,
    );
    // While the second is pending, a version of the first is refused too.
    let early = app
        .post(&format!("/e/{first}/versions"), entry("V", ""))
        .await;
    assert_eq!(early.status, StatusCode::CONFLICT, "{}", early.body);
    assert!(early.body.contains(&second), "{}", early.body);
    app.post(&format!("/e/{second}/publish"), "").await;
    let stale = app
        .post(&format!("/e/{first}/versions"), entry("V", ""))
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert!(stale.body.contains(&second), "{}", stale.body);
    let third = app
        .post(&format!("/e/{second}/versions"), entry("V", ""))
        .await;
    assert_eq!(third.status, StatusCode::CREATED, "{}", third.body);
}

#[tokio::test]
async fn a_paper_version_without_a_table_keeps_the_chains_numbers_in_search() {
    let app = test_app().await;
    let old = app.submit(&entry("T", "")).await;
    let new = id_in(
        &app.post(&format!("/e/{old}/versions"), WITHOUT_TABLE)
            .await
            .body,
    );
    app.post(&format!("/e/{new}/publish"), "").await;
    let found = app
        .get("/search?row_model=model-c&row_claim=effect")
        .await
        .body;
    assert!(found.contains(&new), "{found}");
    assert!(
        app.get(&format!("/e/{new}"))
            .await
            .body
            .contains("now part of a paper arXiv:2507.01410")
    );
}

#[tokio::test]
async fn a_version_of_an_embargoed_entry_that_leaves_the_date_out_keeps_it() {
    // design.md, Embargo (the owner's decision, 2026-11-07): a new version keeps its
    // chain's date, and leaving `embargo_until` out never drops an embargo.
    let app = test_app().await;
    let date = date_in(30);
    let old = app
        .submit(&entry("T", &format!("embargo_until: {date}\n")))
        .await;
    let held = app
        .post(&format!("/e/{old}/versions"), entry("U", ""))
        .await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    assert_eq!(field(&held.body, "embargo_until"), Some(date.as_str()));
    let new = id_in(&held.body);
    let placed = app.post(&format!("/e/{new}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("embargoed"),
        "{}",
        placed.body
    );
    assert_eq!(field(&placed.body, "release"), Some(date.as_str()));
    let own = app.get_as_author(&format!("/e/{new}")).await;
    assert_eq!(
        field(&own.body, "release"),
        Some(date.as_str()),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn a_version_of_an_embargoed_entry_naming_another_date_is_refused() {
    let app = test_app().await;
    let date = date_in(30);
    let old = app
        .submit(&entry("T", &format!("embargo_until: {date}\n")))
        .await;
    for other in [date_in(20), date_in(40)] {
        let reply = app
            .post(
                &format!("/e/{old}/versions"),
                entry("U", &format!("embargo_until: {other}\n")),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{other}: {}",
            reply.body
        );
        assert!(reply.body.contains(&date), "{other}: {}", reply.body);
    }
    let repeated = app
        .post(
            &format!("/e/{old}/versions"),
            entry("U", &format!("embargo_until: {date}\n")),
        )
        .await;
    assert_eq!(repeated.status, StatusCode::CREATED, "{}", repeated.body);
}

#[tokio::test]
async fn a_version_may_repeat_a_chain_date_past_three_months_from_the_first_submission() {
    // The browser page (build 2b) can extend a chain to 3 months from the day it
    // moves it; until then the test writes the moved date itself.
    let app = test_app().await;
    let old = app
        .submit(&entry("T", &format!("embargo_until: {}\n", date_in(30))))
        .await;
    let moved = date_in(120);
    sqlx::query("UPDATE entries SET embargo_until = ? WHERE id = ?")
        .bind(&moved)
        .bind(&old)
        .execute(&app.pool)
        .await
        .unwrap();
    let reply = app
        .post(
            &format!("/e/{old}/versions"),
            entry("U", &format!("embargo_until: {moved}\n")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[tokio::test]
async fn a_paper_version_whose_table_cant_be_read_carries_no_old_numbers() {
    // It has a Results section of its own, so it doesn't inherit the chain's rows,
    // and a version that retracts its numbers stops showing them.
    let app = test_app().await;
    let old = app.submit(&entry("T", "")).await;
    let retracting = WITHOUT_TABLE.replace(
        "## Reproduce",
        "## Results\n\nWe retract every number.\n\n| model | value |\n|---|---|\n| m | 1 |\n\n## Reproduce",
    );
    let new = id_in(
        &app.post(&format!("/e/{old}/versions"), retracting)
            .await
            .body,
    );
    app.post(&format!("/e/{new}/publish"), "").await;
    let found = app
        .get("/search?row_model=model-c&row_claim=effect")
        .await
        .body;
    assert!(!found.contains(&new), "{found}");
}

#[tokio::test]
async fn versions_posted_at_once_give_one_201_and_409s_naming_it() {
    let app = test_app().await;
    let old = app.submit(&entry("T", "")).await;
    let path = format!("/e/{old}/versions");
    let replies = futures_join(&app, &path).await;
    let created: Vec<&common::Reply> = replies
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .collect();
    assert_eq!(
        created.len(),
        1,
        "{:?}",
        replies.iter().map(|r| r.status).collect::<Vec<_>>()
    );
    let winner = id_in(&created[0].body);
    for reply in replies.iter().filter(|r| r.status != StatusCode::CREATED) {
        assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
        assert!(reply.body.contains(&winner), "{}", reply.body);
    }
}

async fn futures_join(app: &common::TestApp, path: &str) -> Vec<common::Reply> {
    let posts = (0..6).map(|n| app.post(path, entry(&format!("V{n}"), "")));
    let mut replies = Vec::new();
    for reply in futures::future::join_all(posts).await {
        replies.push(reply);
    }
    replies
}

#[tokio::test]
async fn the_author_sees_previous_on_a_held_version_and_put_keeps_it() {
    let app = test_app().await;
    let old = app.submit(&entry("T", "")).await;
    let new = id_in(
        &app.post(&format!("/e/{old}/versions"), entry("U", ""))
            .await
            .body,
    );
    let mine = app.get_as_author(&format!("/e/{new}")).await.body;
    assert_eq!(field(&mine, "previous"), Some(old.as_str()), "{mine}");
    let put = app
        .put_as(&format!("/e/{new}"), &app.token, entry("U2", ""))
        .await;
    assert_eq!(
        field(&put.body, "previous"),
        Some(old.as_str()),
        "{}",
        put.body
    );
}

#[tokio::test]
async fn tags_count_each_chain_once() {
    let app = test_app().await;
    let old = app.submit(&entry("T", "")).await;
    let new = id_in(
        &app.post(&format!("/e/{old}/versions"), entry("U", ""))
            .await
            .body,
    );
    app.post(&format!("/e/{new}/publish"), "").await;
    assert_eq!(app.get("/tags").await.body, "refusal 1\nsteering 1\n");
}

#[tokio::test]
async fn a_version_that_leaves_its_date_out_may_refer_to_entries_held_as_long() {
    // The reference rule (design.md, Agent interface) compares the date the version
    // keeps, not the empty field.
    let app = test_app().await;
    let date = date_in(30);
    app.submit(&entry(
        "Target",
        &format!("slug: target\nembargo_until: {date}\n"),
    ))
    .await;
    let old = app
        .submit(&entry(
            "Referrer",
            &format!("embargo_until: {date}\nreplicates: slug:target\n"),
        ))
        .await;
    let reply = app
        .post(
            &format!("/e/{old}/versions"),
            entry("Referrer, corrected", "replicates: slug:target\n"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}
