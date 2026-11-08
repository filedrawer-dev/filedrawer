//! Seam: the router, for `DELETE /e/<id>` and `DELETE /batches/<id>` (design.md,
//! Accounts; Agent interface): a token deletes a pending entry, or a whole pending
//! batch, that it submitted itself; every other request is refused and changes
//! nothing; and what a later read, retry, slug, upload or allowance shows.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::attachments::{citing, one_file, set_with, upload};
use common::deletion::state_of;
use common::fake_models::FakeModels;
use common::series::{DESCRIPTION, citing as cites, create, series_doc, submit_with};
use common::{
    Reply, TestApp, author_status, batch_member_ids, date_in, entry, field, id_in, owner,
    stored_texts, test_app, test_app_with,
};
use filedrawer_server::Screener;
use serde_json::{Value, json};

fn json_of(reply: &Reply) -> Value {
    serde_json::from_str(&reply.body).unwrap_or_else(|_| panic!("not JSON: {}", reply.body))
}

/// Submits the batch `body` with `token`, which must be accepted; returns the batch's
/// ID and its members' IDs.
async fn held_batch(app: &TestApp, token: &str, body: Value) -> (String, Vec<String>) {
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
    let reply = json_of(&reply);
    (
        reply["id"].as_str().unwrap().to_string(),
        batch_member_ids(&reply),
    )
}

fn two_members() -> Value {
    json!({"entries": [entry("A", "slug: a\n"), entry("B", "slug: b\n")]})
}

/// A run declaration with `id`.
fn run(id: &str) -> Value {
    json!({"id": id, "description": format!("Run {id}, 520 prompts."), "n": 520,
           "commit": "0123456789abcdef0123456789abcdef01234567", "code_state": "committed",
           "route": "weights", "dates": ["2026-10-20", "2026-10-21"]})
}

const HAZARD: &str = "FAKE-HAZARD FAKE-SONNET-HAZARD";

async fn status(app: &TestApp, id: &str) -> Option<String> {
    author_status(app, &app.token, id).await
}

#[tokio::test]
async fn a_token_deletes_its_pending_entry_and_a_retry_is_told_when() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Draft", "slug: draft\n")).await;
    let path = format!("/e/{id}");
    let deleted = app.delete_as(&path, &app.token, false).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(field(&deleted.body, "id"), Some(id.as_str()));
    assert_eq!(field(&deleted.body, "status"), Some("deleted"));
    let at = field(&deleted.body, "deleted")
        .expect("the reply gives the time")
        .to_string();
    assert!(
        deleted
            .body
            .lines()
            .any(|line| line == format!("account: {} (Ada)", app.account)),
        "{}",
        deleted.body
    );

    // The account's read and retry name the time; nobody else learns anything.
    let read = app.get_as_author(&path).await;
    assert_eq!(read.status, StatusCode::GONE, "{}", read.body);
    assert!(read.body.contains(&at), "{}", read.body);
    let again = app.delete_as(&path, &app.token, false).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(field(&again.body, "deleted"), Some(at.as_str()));
    assert_eq!(app.get(&path).await.status, StatusCode::NOT_FOUND);
    let other = app.other_token().await;
    let theirs = app.get_with_token(&path, &other).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND, "{}", theirs.body);
    let their_delete = app.delete_as(&path, &other, false).await;
    assert_eq!(their_delete.status, StatusCode::NOT_FOUND);
    assert!(!their_delete.body.contains(&at), "{}", their_delete.body);
    let mine = app.get_as_author("/entries/mine").await;
    assert!(!mine.body.contains(&id), "{}", mine.body);

    // Its slug is free again, as after a rejection.
    let reused = app
        .post("/entries", entry("Draft, fixed", "slug: draft\n"))
        .await;
    assert_eq!(reused.status, StatusCode::CREATED, "{}", reused.body);
}

#[tokio::test]
async fn a_deletion_answers_json_when_asked() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Draft", "")).await;
    let reply = app.delete_as(&format!("/e/{id}"), &app.token, true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let body = json_of(&reply);
    assert_eq!(body["id"], id.as_str());
    assert_eq!(body["status"], "deleted");
    assert!(body["deleted"].is_string(), "{body}");
    assert_eq!(
        body["account"],
        json!({"id": app.account, "display_name": "Ada"})
    );
}

/// Sends `DELETE path` with `token`, which must be refused with `status`, and checks
/// that nothing about entries `ids` changed: their rows, slugs, runs, sets, units
/// and deletion records. Returns the reply.
async fn refused(
    app: &TestApp,
    path: &str,
    token: &str,
    status: StatusCode,
    ids: &[&str],
) -> Reply {
    let before = state_of(app, ids).await;
    let reply = app.delete_as(path, token, false).await;
    assert_eq!(reply.status, status, "{path}: {}", reply.body);
    assert_eq!(state_of(app, ids).await, before, "{path} changed something");
    reply
}

fn ids(members: &[String]) -> Vec<&str> {
    members.iter().map(String::as_str).collect()
}

#[tokio::test]
async fn a_public_entry_is_refused_as_public() {
    let app = test_app().await;
    let id = app.submit(&entry("Out", "slug: out\n")).await;
    let reply = refused(
        &app,
        &format!("/e/{id}"),
        &app.token,
        StatusCode::CONFLICT,
        &[&id],
    )
    .await;
    assert!(reply.body.contains("public"), "{}", reply.body);
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn past_its_hold_an_entry_is_refused_with_its_status() {
    let models = FakeModels::default();
    let app = test_app_with(models.screener(), 1_000_000_000).await;
    let token = app.token.clone();
    let embargo = format!("embargo_until: {}\n", date_in(30));
    let embargoed = submit_with(
        &app,
        &token,
        &entry("Held back", &format!("slug: e\n{embargo}")),
    )
    .await;
    // A model outage leaves an entry in screening.
    let screening = submit_with(&app, &token, &entry("Steering FAKE-ERROR", "slug: s\n")).await;
    let rejected = submit_with(&app, &token, &entry("Steering FAKE-OFFTOPIC", "")).await;
    let hazard = entry(&format!("Steering {HAZARD}"), "");
    let review = submit_with(&app, &token, &hazard).await;
    let held = submit_with(&app, &token, &hazard).await;
    let (series, _) = create(&app, &token).await;
    let waiting = submit_with(&app, &token, &cites("Waits", &format!("{series}@1"), "")).await;
    let frozen = app.submit_held(&entry("Frozen", "slug: f\n")).await;
    // A revoked invite code freezes an entry; here the row is set as that leaves it.
    sqlx::query("UPDATE entries SET status = 'frozen', hold_until = NULL WHERE id = ?")
        .bind(&frozen)
        .execute(&app.pool)
        .await
        .unwrap();
    let admin = owner(&app).await;
    let hold = app
        .form(&format!("/admin/e/{held}/hold"), &[], Some(&admin))
        .await;
    assert_eq!(hold.status, StatusCode::SEE_OTHER, "{}", hold.body);
    for (id, what) in [
        (&embargoed, "embargoed"),
        (&screening, "screening"),
        (&rejected, "rejected"),
        (&review, "review"),
        (&held, "held"),
        (&waiting, "waiting_on_series"),
        (&frozen, "frozen"),
    ] {
        assert_eq!(status(&app, id).await.as_deref(), Some(what), "{id}");
        let reply = refused(
            &app,
            &format!("/e/{id}"),
            &token,
            StatusCode::CONFLICT,
            &[id],
        )
        .await;
        assert!(reply.body.contains(&format!("is {what}")), "{}", reply.body);
        assert!(reply.body.contains("pending"), "{}", reply.body);
    }
}

#[tokio::test]
async fn an_entry_another_token_submitted_is_refused() {
    let app = test_app().await;
    let second = app.second_token().await;
    let id = app.submit_held(&entry("Draft", "slug: draft\n")).await;
    let path = format!("/e/{id}");
    let reply = refused(&app, &path, &second, StatusCode::FORBIDDEN, &[&id]).await;
    assert!(reply.body.contains("another token"), "{}", reply.body);
    let own = app.delete_as(&path, &app.token, false).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
}

#[tokio::test]
async fn a_put_makes_its_token_the_one_that_deletes_the_entry() {
    let app = test_app().await;
    let second = app.second_token().await;
    let id = app.submit_held(&entry("Draft", "slug: draft\n")).await;
    let path = format!("/e/{id}");
    let put = app
        .put_as(&path, &second, entry("Draft, fixed", "slug: draft\n"))
        .await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.body);
    let reply = refused(&app, &path, &app.token, StatusCode::FORBIDDEN, &[&id]).await;
    assert!(reply.body.contains("another token"), "{}", reply.body);
    let theirs = app.delete_as(&path, &second, false).await;
    assert_eq!(theirs.status, StatusCode::OK, "{}", theirs.body);
}

#[tokio::test]
async fn an_entry_stored_before_tokens_were_recorded_isnt_deleted_by_token() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Old", "slug: old\n")).await;
    // As migration 23 leaves an entry stored before it.
    sqlx::query("UPDATE entries SET token_id = NULL WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await
        .unwrap();
    let reply = refused(
        &app,
        &format!("/e/{id}"),
        &app.token,
        StatusCode::FORBIDDEN,
        &[&id],
    )
    .await;
    assert!(reply.body.contains("another token"), "{}", reply.body);
}

#[tokio::test]
async fn a_batch_member_is_refused_naming_its_batch() {
    let app = test_app().await;
    let (batch, members) = held_batch(&app, &app.token, two_members()).await;
    let reply = refused(
        &app,
        &format!("/e/{}", members[0]),
        &app.token,
        StatusCode::CONFLICT,
        &ids(&members),
    )
    .await;
    assert!(
        reply.body.contains(&format!("DELETE /batches/{batch}")),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn what_the_tokens_account_cant_see_is_not_found_naming_that_account() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Mine", "slug: mine\n")).await;
    let other = app.other_token().await;
    let (theirs, members) = held_batch(&app, &other, two_members()).await;
    let mut seen = ids(&members);
    seen.push(&id);
    for (path, token, name) in [
        (format!("/e/{id}"), &other, " (Mallory)"),
        ("/e/0000000000".to_string(), &other, " (Mallory)"),
        (format!("/batches/{id}"), &other, " (Mallory)"),
        (format!("/batches/{theirs}"), &app.token, " (Ada)"),
        (format!("/e/{}", members[0]), &app.token, " (Ada)"),
    ] {
        let reply = refused(&app, &path, token, StatusCode::NOT_FOUND, &seen).await;
        assert!(
            reply.body.contains("account: a-") && reply.body.contains(name),
            "{path}: {}",
            reply.body
        );
    }
}

#[tokio::test]
async fn deleting_needs_a_token() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Mine", "slug: mine\n")).await;
    for path in [format!("/e/{id}"), format!("/batches/{id}")] {
        let before = state_of(&app, &[&id]).await;
        let reply = app
            .send(Request::delete(&path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(state_of(&app, &[&id]).await, before, "{path}");
    }
}

#[tokio::test]
async fn a_token_deletes_its_whole_pending_batch() {
    let app = test_app().await;
    let (batch, members) = held_batch(&app, &app.token, two_members()).await;
    let path = format!("/batches/{batch}");
    let deleted = app.delete_as(&path, &app.token, false).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(field(&deleted.body, "id"), Some(batch.as_str()));
    assert_eq!(field(&deleted.body, "status"), Some("deleted"));
    let at = field(&deleted.body, "deleted").unwrap().to_string();
    for member in &members {
        assert!(
            deleted.body.contains(&format!("entry: {member}\n")),
            "{}",
            deleted.body
        );
        let read = app.get_as_author(&format!("/e/{member}")).await;
        assert_eq!(read.status, StatusCode::GONE, "{}", read.body);
        assert!(read.body.contains(&at), "{}", read.body);
    }
    let again = app.delete_as(&path, &app.token, false).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(field(&again.body, "deleted"), Some(at.as_str()));
    // The slugs are free again.
    held_batch(&app, &app.token, two_members()).await;
}

#[tokio::test]
async fn a_batch_is_refused_unless_every_member_is_pending_and_this_tokens() {
    let app = test_app().await;
    let second = app.second_token().await;
    let (theirs, members) = held_batch(&app, &app.token, two_members()).await;
    let path = format!("/batches/{theirs}");
    let reply = refused(&app, &path, &second, StatusCode::FORBIDDEN, &ids(&members)).await;
    assert!(reply.body.contains("another token"), "{}", reply.body);

    let published = app.post(&format!("/batches/{theirs}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    let reply = refused(
        &app,
        &path,
        &app.token,
        StatusCode::CONFLICT,
        &ids(&members),
    )
    .await;
    assert!(reply.body.contains("public"), "{}", reply.body);

    let embargo = format!("embargo_until: {}\n", date_in(30));
    let held_back = json!({"entries": [
        entry("C", &format!("slug: c\n{embargo}")),
        entry("D", &format!("slug: d\n{embargo}")),
    ]});
    let (embargoed, members) = held_batch(&app, &app.token, held_back).await;
    let published = app.post(&format!("/batches/{embargoed}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    assert_eq!(
        status(&app, &members[1]).await.as_deref(),
        Some("embargoed")
    );
    let reply = refused(
        &app,
        &format!("/batches/{embargoed}"),
        &app.token,
        StatusCode::CONFLICT,
        &ids(&members),
    )
    .await;
    assert!(reply.body.contains("is embargoed"), "{}", reply.body);

    // With no screening budget, a batch past its hold waits in screening.
    let waiting = test_app_with(Screener::PassAll, 0).await;
    let (screening, members) = held_batch(&waiting, &waiting.token, two_members()).await;
    let published = waiting
        .post(&format!("/batches/{screening}/publish"), "")
        .await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    assert_eq!(
        status(&waiting, &members[1]).await.as_deref(),
        Some("screening")
    );
    let reply = refused(
        &waiting,
        &format!("/batches/{screening}"),
        &waiting.token,
        StatusCode::CONFLICT,
        &ids(&members),
    )
    .await;
    assert!(reply.body.contains("is screening"), "{}", reply.body);
}

#[tokio::test]
async fn a_pending_version_is_deleted_by_the_token_that_sent_it() {
    let app = test_app().await;
    let second = app.second_token().await;
    let first = app.submit(&entry("First", "slug: first\n")).await;
    let posted = app
        .post_as(
            &format!("/e/{first}/versions"),
            Some(&second),
            entry("First, corrected", ""),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    let version = id_in(&posted.body);
    let deleted = app
        .delete_as(&format!("/e/{version}"), &second, false)
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(app.get(&format!("/e/{first}")).await.status, StatusCode::OK);
    // The chain keeps its slug, and takes a new version again.
    let again = app
        .post(&format!("/e/{first}/versions"), entry("First, again", ""))
        .await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
    let slug = app
        .post("/entries", entry("Another", "slug: first\n"))
        .await;
    assert_eq!(
        slug.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        slug.body
    );
}

#[tokio::test]
async fn a_deleted_entrys_unit_isnt_given_back_nor_counted_as_a_series_unit() {
    let app = test_app().await;
    let mut ids = Vec::new();
    for n in 0..20 {
        ids.push(app.submit_held(&entry(&format!("Steering {n}"), "")).await);
    }
    let deleted = app
        .delete_as(&format!("/e/{}", ids[19]), &app.token, false)
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    let next = app.post("/entries", entry("Steering 21", "")).await;
    assert_eq!(next.status, StatusCode::CREATED, "{}", next.body);
    assert_eq!(
        field(&next.body, "screen_from"),
        Some(date_in(1).as_str()),
        "the deleted entry's unit still counts: {}",
        next.body
    );
    // A day's allowance of series still fits.
    let created = app
        .post("/series", series_doc("Layer sweep", DESCRIPTION))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let series = id_in(&created.body);
    for n in 1..20 {
        let version = app
            .post(
                &format!("/series/{series}/versions"),
                series_doc(&format!("Layer sweep {n}"), DESCRIPTION),
            )
            .await;
        assert_eq!(version.status, StatusCode::CREATED, "{n}: {}", version.body);
    }
}

#[tokio::test]
async fn deleting_takes_only_the_set_runs_and_batch_the_deleted_entries_alone_used() {
    let app = test_app().await;
    let used = set_with(&app, &app.token, "run.py", "print('used')\n").await;
    let later = set_with(&app, &app.token, "next.py", "print('later')\n").await;
    let shared = set_with(&app, &app.token, "shared.py", "print('shared')\n").await;
    let body = json!({
        "runs": [run("r-used"), run("r-later")],
        "entries": [entry("A", &format!("slug: a\nuses_runs: [r-used]\nattachments: {used}\n"))],
    });
    let (batch, _) = held_batch(&app, &app.token, body).await;
    let one = app.submit_held(&citing("One", &shared)).await;
    let two = app.submit_held(&citing("Two", &shared)).await;
    for path in [format!("/batches/{batch}"), format!("/e/{one}")] {
        let reply = app.delete_as(&path, &app.token, false).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
    }

    // The set only the batch cited is gone, with its file; the one uploaded for
    // later and the one another entry still cites stay.
    let refused = app.post("/entries", citing("Again", &used)).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.body
    );
    assert!(
        refused.body.contains("names no attachment set"),
        "{}",
        refused.body
    );
    let next = app.post("/entries", citing("Next", &later)).await;
    assert_eq!(next.status, StatusCode::CREATED, "{}", next.body);
    let kept = app.get_as_author(&format!("/e/{two}")).await;
    assert!(kept.body.contains("src/shared.py"), "{}", kept.body);
    assert_eq!(
        stored_texts(&app),
        vec!["print('later')\n", "print('shared')\n"]
    );

    // The run only the batch used is gone; the one declared for later stays.
    let (_, _) = held_batch(
        &app,
        &app.token,
        json!({"entries": [entry("B", "slug: b\nuses_runs: [r-later]\n")]}),
    )
    .await;
    let gone = app
        .post(
            "/entries/batch",
            json!({"entries": [entry("C", "slug: c\nuses_runs: [r-used]\n")]}).to_string(),
        )
        .await;
    assert_eq!(
        gone.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        gone.body
    );
    assert!(
        gone.body
            .contains("neither this batch nor an earlier one declares"),
        "{}",
        gone.body
    );
}

#[tokio::test]
async fn a_deleted_sets_bytes_still_count_toward_the_days_uploads() {
    let app = test_app().await;
    let mebibyte = "y".repeat(1_048_000);
    let mut sets = Vec::new();
    for n in 0..9 {
        sets.push(set_with(&app, &app.token, "big.txt", &format!("{n}{mebibyte}")).await);
    }
    let id = app.submit_held(&citing("Big", &sets[0])).await;
    let deleted = app.delete_as(&format!("/e/{id}"), &app.token, false).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    let reply = upload(&app, &app.token, one_file("more.txt", &"z".repeat(600_000))).await;
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.body
    );
}
