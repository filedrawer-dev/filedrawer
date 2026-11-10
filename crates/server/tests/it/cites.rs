//! Seam: the router, for `GET /search?cites=` (design.md, Agent interface): the
//! entries whose `replicates` or `reanalyzes` name a work.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, entry, id_in, test_app};

/// The entry IDs a plain-text search answered, in order.
fn ids(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| line.split_once("; ").map(|(id, _)| id))
        .filter(|id| id.len() == 10 && !id.contains(' '))
        .map(str::to_string)
        .collect()
}

async fn cites(app: &TestApp, value: &str) -> Vec<String> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("cites", value)
        .finish();
    let reply = app.get(&format!("/search?{query}")).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    ids(&reply.body)
}

/// Submits `text` as the account holding `token` and publishes it.
async fn submit_as(app: &TestApp, token: &str, text: &str) -> String {
    let reply = app.post_as("/entries", Some(token), text.to_string()).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let id = id_in(&reply.body);
    let published = app
        .post_as(&format!("/e/{id}/publish"), Some(token), "")
        .await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    id
}

/// Posts `text` as a new version of `id`, held, and returns its ID.
async fn version_held(app: &TestApp, id: &str, text: &str) -> String {
    let reply = app
        .post(&format!("/e/{id}/versions"), text.to_string())
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    id_in(&reply.body)
}

/// A version that points to a paper and names nothing itself.
const PAPER_VERSION: &str = "---
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

async fn publish(app: &TestApp, id: &str) {
    let reply = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn cites_finds_the_entries_that_follow_up_a_paper_however_it_is_written() {
    let app = test_app().await;
    let audit = app
        .submit(&entry(
            "Five papers checked",
            "kind: audit\nreanalyzes:\n  - arXiv:2609.18346\n  - 10.2139/ssrn.7439840\n  - https://www.lesswrong.com/posts/abc/title\n",
        ))
        .await;
    let replication = app
        .submit(&entry("Keppo re-run", "replicates: arXiv:2603.20281\n"))
        .await;
    app.submit(&entry("Unrelated", "")).await;
    assert_eq!(
        cites(&app, "https://arxiv.org/abs/2609.18346v2").await,
        vec![audit.clone()]
    );
    assert_eq!(
        cites(&app, "doi:10.2139/SSRN.7439840").await,
        vec![audit.clone()]
    );
    assert_eq!(
        cites(&app, "https://lesswrong.com/posts/abc/title/").await,
        vec![audit.clone()]
    );
    assert_eq!(cites(&app, "arXiv:2603.20281").await, vec![replication]);
    assert_eq!(cites(&app, "arXiv:2507.99999").await, Vec::<String>::new());
    let both = app
        .get("/search?cites=arXiv:2609.18346&kind=experiment")
        .await;
    assert_eq!(ids(&both.body), Vec::<String>::new());
}

#[tokio::test]
async fn cites_in_no_known_form_or_naming_a_slug_is_a_bad_request() {
    let app = test_app().await;
    let free_text = app.get("/search?cites=Yun%20(2026)").await;
    assert_eq!(
        free_text.status,
        StatusCode::BAD_REQUEST,
        "{}",
        free_text.body
    );
    assert!(free_text.body.contains("arXiv:<id>"), "{}", free_text.body);
    let slug = app.get("/search?cites=slug:baseline").await;
    assert_eq!(slug.status, StatusCode::BAD_REQUEST, "{}", slug.body);
    assert!(slug.body.contains("ID"), "{}", slug.body);
}

#[tokio::test]
async fn an_entry_is_cited_by_any_public_version_s_id_or_its_slug_within_the_account() {
    let app = test_app().await;
    let first = app.submit(&entry("Baseline", "slug: baseline\n")).await;
    let second = version_held(
        &app,
        &first,
        &entry("Baseline, corrected", "slug: baseline\n"),
    )
    .await;
    publish(&app, &second).await;
    let by_slug = app
        .submit(&entry("Same account re-run", "replicates: slug:baseline\n"))
        .await;
    let other = app.other_token().await;
    let by_old_id = submit_as(
        &app,
        &other,
        &entry("Other account re-run", &format!("replicates: {first}\n")),
    )
    .await;
    // Another account's `slug:baseline` names its own entry, not this one.
    submit_as(
        &app,
        &other,
        &entry("Other account, own slug", "replicates: slug:baseline\n"),
    )
    .await;
    let mut expected = vec![by_slug, by_old_id];
    expected.sort();
    for named in [&first, &second] {
        let mut found = cites(&app, named).await;
        found.sort();
        assert_eq!(found, expected, "cites={named}");
    }
}

#[tokio::test]
async fn a_version_not_yet_public_is_no_way_into_its_chain() {
    let app = test_app().await;
    let first = app.submit(&entry("Baseline", "")).await;
    // The held version takes the chain's slug.
    let held = version_held(&app, &first, &entry("Baseline, corrected", "slug: late\n")).await;
    let other = app.other_token().await;
    let cites_held = submit_as(
        &app,
        &other,
        &entry("Cites the held version", &format!("replicates: {held}\n")),
    )
    .await;
    app.submit(&entry("Cites the held slug", "replicates: slug:late\n"))
        .await;
    assert_eq!(cites(&app, &first).await, Vec::<String>::new());
    // Named itself, the held ID matches only as written.
    assert_eq!(cites(&app, &held).await, vec![cites_held]);
}

#[tokio::test]
async fn a_version_that_points_to_a_paper_keeps_what_its_chain_followed_up() {
    let app = test_app().await;
    let first = app
        .submit(&entry("Keppo re-run", "replicates: arXiv:2603.20281\n"))
        .await;
    let second = version_held(&app, &first, PAPER_VERSION).await;
    publish(&app, &second).await;
    assert_eq!(cites(&app, "arXiv:2603.20281").await, vec![second]);
}

#[tokio::test]
async fn a_put_replaces_what_the_entry_cites() {
    let app = test_app().await;
    let id = app
        .submit_held(&entry("Re-run", "replicates: arXiv:2603.20281\n"))
        .await;
    let reply = app
        .put_as(
            &format!("/e/{id}"),
            &app.token,
            entry("Re-run", "replicates: arXiv:2609.18346\n"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    publish(&app, &id).await;
    assert_eq!(cites(&app, "arXiv:2603.20281").await, Vec::<String>::new());
    assert_eq!(cites(&app, "arXiv:2609.18346").await, vec![id]);
}

#[tokio::test]
async fn entries_stored_before_the_keys_were_are_found_after_the_rebuild() {
    let app = test_app().await;
    let id = app
        .submit(&entry("Re-run", "replicates: arXiv:2603.20281\n"))
        .await;
    // A chain whose newer version carries its first version's keys, which the
    // rebuild finds only when it reads the first version first.
    let first = app
        .submit(&entry("Keppo re-run", "replicates: arXiv:2609.18346\n"))
        .await;
    let second = version_held(&app, &first, PAPER_VERSION).await;
    publish(&app, &second).await;
    // As a database from before this table, or from an older key format, holds them.
    sqlx::query("DELETE FROM entry_references")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM derived_versions")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(cites(&app, "arXiv:2603.20281").await, Vec::<String>::new());
    filedrawer_server::db::refresh_derived(&app.pool)
        .await
        .unwrap();
    assert_eq!(cites(&app, "arXiv:2603.20281").await, vec![id]);
    assert_eq!(cites(&app, "arXiv:2609.18346").await, vec![second]);
}

#[tokio::test]
async fn the_older_link_keeps_the_cites_filter() {
    let app = test_app().await;
    for title in ["One", "Two"] {
        app.submit(&entry(title, "replicates: arXiv:2603.20281\n"))
            .await;
    }
    let page = app
        .send(
            Request::get("/search?cites=arXiv:2603.20281&limit=1")
                .header("sec-fetch-dest", "document")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        page.body
            .contains(r#"href="/search?cites=arXiv%3A2603.20281&#38;limit=1&#38;page=2""#),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn an_older_binary_leaves_a_newer_reference_format_as_it_is() {
    // A rollback starts the older binary on what the newer one built. Rebuilding every
    // entry's references at startup could outlast the install's health check, so it
    // keeps the rows and records its own format, since the rows it adds are in that
    // one; the next newer binary then rebuilds them all.
    let app = test_app().await;
    app.submit(&entry("Re-run", "replicates: arXiv:2603.20281\n"))
        .await;
    // Rows in a format this binary can't read, which a rebuild would replace.
    sqlx::query("DELETE FROM entry_references")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE derived_versions SET version = version + 1 WHERE name = 'entry_references'",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let newer: i64 =
        sqlx::query_scalar("SELECT version FROM derived_versions WHERE name = 'entry_references'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    filedrawer_server::db::refresh_derived(&app.pool)
        .await
        .unwrap();
    let after: i64 =
        sqlx::query_scalar("SELECT version FROM derived_versions WHERE name = 'entry_references'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(after, newer - 1);
    assert_eq!(
        cites(&app, "arXiv:2603.20281").await,
        Vec::<String>::new(),
        "rebuilt the newer binary's rows"
    );
}
