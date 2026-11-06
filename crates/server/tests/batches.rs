//! Seam: the router and `jobs::publish_due`, for `POST /entries/batch`, slugs and
//! runs (design.md, Agent interface).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Reply, TestApp, date_in, entry, field, test_app};
use filedrawer_server::jobs::publish_due;
use serde_json::{Value, json};
use time::Duration;

/// An entry with a slug and extra front matter.
fn member(slug: &str, title: &str, front: &str) -> String {
    entry(title, &format!("slug: {slug}\n{front}"))
}

async fn batch(app: &TestApp, body: Value) -> Reply {
    app.send(
        Request::post("/entries/batch")
            .header("authorization", format!("Bearer {}", app.token))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

fn json(reply: &Reply) -> Value {
    serde_json::from_str(&reply.body).unwrap_or_else(|_| panic!("not JSON: {}", reply.body))
}

/// The ID of the member with `slug` in a batch reply.
fn id_of(reply: &Value, slug: &str) -> String {
    reply["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["slug"] == slug)
        .unwrap_or_else(|| panic!("no member {slug} in {reply}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

const RUN: &str = "sweep-1";

fn run() -> Value {
    json!({"id": RUN, "description": "Sweep over layers 10 to 18, 520 prompts each.", "n": 520,
           "commit": "0123456789abcdef0123456789abcdef01234567", "code_state": "committed",
           "route": "weights", "dates": ["2026-10-20", "2026-10-21"]})
}

#[tokio::test]
async fn a_batch_is_held_then_published_together_and_its_slugs_resolve() {
    let app = test_app().await;
    let reply = batch(
        &app,
        json!({"runs": [run()], "entries": [
            member("pilot", "Pilot sweep", "uses_runs: [sweep-1]\nsuperseded_by: slug:main\n"),
            member("main", "Main sweep", "uses_runs: [sweep-1]\n"),
        ]}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let body = json(&reply);
    assert_eq!(body["status"], "pending");
    let (pilot, main) = (id_of(&body, "pilot"), id_of(&body, "main"));
    let batch_id = body["id"].as_str().unwrap();
    assert_eq!(
        app.get(&format!("/e/{pilot}")).await.status,
        StatusCode::NOT_FOUND
    );

    let published = app.post(&format!("/batches/{batch_id}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    let page = app.get(&format!("/e/{pilot}")).await.body;
    assert!(page.contains(&format!("superseded by {main}")), "{page}");
    assert!(
        page.contains(&format!("link: slug:main = {main}")),
        "{page}"
    );
    assert!(
        page.contains(&format!("run: {RUN}: Sweep over layers")),
        "{page}"
    );
    assert_eq!(app.get(&format!("/e/{main}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn one_refused_member_refuses_the_batch_and_keeps_none_of_it() {
    let app = test_app().await;
    let broken =
        member("b", "T", "").replace("hypothesis: Steering at layer 14 raises refusals.\n", "");
    let reply = batch(&app, json!({"entries": [member("a", "A", ""), broken]})).await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("entry 2 (slug b)"), "{}", reply.body);
    assert!(
        reply.body.contains("`hypothesis` is missing"),
        "{}",
        reply.body
    );
    // Nothing was kept, so slug `a` is still free.
    let again = batch(&app, json!({"entries": [member("a", "A", "")]})).await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
}

#[tokio::test]
async fn slugs_must_be_given_and_unique_in_the_batch_and_the_account() {
    let app = test_app().await;
    let unslugged = batch(&app, json!({"entries": [entry("T", "")]})).await;
    assert_eq!(unslugged.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(unslugged.body.contains("slug"), "{}", unslugged.body);
    let twice = batch(
        &app,
        json!({"entries": [member("a", "A", ""), member("a", "B", "")]}),
    )
    .await;
    assert_eq!(twice.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(twice.body.contains("`a`"), "{}", twice.body);
    batch(&app, json!({"entries": [member("taken", "A", "")]})).await;
    let reused = batch(&app, json!({"entries": [member("taken", "B", "")]})).await;
    assert_eq!(reused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(reused.body.contains("`taken`"), "{}", reused.body);
}

#[tokio::test]
async fn a_run_is_declared_once_and_cited_after() {
    let app = test_app().await;
    batch(
        &app,
        json!({"runs": [run()], "entries": [member("a", "A", "uses_runs: [sweep-1]\n")]}),
    )
    .await;
    let cited = batch(
        &app,
        json!({"entries": [member("b", "B", "uses_runs: [sweep-1]\n")]}),
    )
    .await;
    assert_eq!(cited.status, StatusCode::CREATED, "{}", cited.body);
    let again = batch(
        &app,
        json!({"runs": [run()], "entries": [member("c", "C", "")]}),
    )
    .await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(again.body.contains(RUN), "{}", again.body);
    let unknown = batch(
        &app,
        json!({"entries": [member("d", "D", "uses_runs: [nope]\n")]}),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(unknown.body.contains("`nope`"), "{}", unknown.body);
}

#[tokio::test]
async fn a_runs_description_waits_until_every_entry_using_it_is_public() {
    let app = test_app().await;
    let first = json(
        &batch(
            &app,
            json!({"runs": [run()], "entries": [member("a", "A", "uses_runs: [sweep-1]\n")]}),
        )
        .await,
    );
    let a = id_of(&first, "a");
    app.post(
        &format!("/batches/{}/publish", first["id"].as_str().unwrap()),
        "",
    )
    .await;
    // A second entry on the run, still held: the description stays hidden.
    batch(
        &app,
        json!({"entries": [member("b", "B", "uses_runs: [sweep-1]\n")]}),
    )
    .await;
    let page = app.get(&format!("/e/{a}")).await.body;
    assert!(
        page.contains(&format!("run: {RUN}: description not public yet")),
        "{page}"
    );
    assert!(!page.contains("Sweep over layers"), "{page}");
}

#[tokio::test]
async fn a_public_member_may_not_refer_to_an_embargoed_one() {
    let app = test_app().await;
    let reply = batch(
        &app,
        json!({"entries": [
            member("open", "Open", "replicates: slug:secret\n"),
            member("secret", "Secret", &format!("embargo_until: {}\n", date_in(30))),
        ]}),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("embargoed"), "{}", reply.body);
}

#[tokio::test]
async fn a_put_on_one_member_restarts_the_whole_batch_hold_and_the_hour_publishes_it() {
    let app = test_app().await;
    let body = json(
        &batch(
            &app,
            json!({"entries": [member("a", "A", ""), member("b", "B", "")]}),
        )
        .await,
    );
    let (a, b) = (id_of(&body, "a"), id_of(&body, "b"));
    let wrong_slug = app
        .put_as(&format!("/e/{a}"), &app.token, member("other", "A2", ""))
        .await;
    assert_eq!(
        wrong_slug.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong_slug.body
    );
    let put = app
        .put_as(&format!("/e/{a}"), &app.token, member("a", "A2", ""))
        .await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.body);
    let restarted = time::OffsetDateTime::parse(
        field(&put.body, "publish_at").unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    // Both members wait for the restarted hour, and publish together when it ends.
    assert_eq!(
        publish_due(&app.state, restarted - Duration::seconds(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.get(&format!("/e/{b}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        publish_due(&app.state, restarted + Duration::seconds(1))
            .await
            .unwrap(),
        2
    );
    assert_eq!(app.get(&format!("/e/{a}")).await.status, StatusCode::OK);
    assert_eq!(app.get(&format!("/e/{b}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_single_entrys_slug_can_be_cited_by_a_later_batch() {
    let app = test_app().await;
    let earlier = app.submit(&member("baseline", "Baseline", "")).await;
    let body = json(
        &batch(
            &app,
            json!({"entries": [member("follow", "Follow-up", "replicates: slug:baseline\n")]}),
        )
        .await,
    );
    let follow = id_of(&body, "follow");
    app.post(
        &format!("/batches/{}/publish", body["id"].as_str().unwrap()),
        "",
    )
    .await;
    let page = app.get(&format!("/e/{follow}")).await.body;
    assert!(
        page.contains(&format!("link: slug:baseline = {earlier}")),
        "{page}"
    );
}

#[tokio::test]
async fn a_slug_or_run_id_with_odd_characters_is_refused() {
    let app = test_app().await;
    let forged = entry("T", "slug: \"x\\nstatus: public\\nid: forged\"\n");
    let reply = batch(&app, json!({"entries": [forged]})).await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("slug"), "{}", reply.body);
    let mut bad_run = run();
    bad_run["id"] = json!("sweep\nstatus: public");
    let reply = batch(
        &app,
        json!({"runs": [bad_run], "entries": [member("a", "A", "")]}),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn a_put_runs_the_link_checks() {
    let app = test_app().await;
    app.submit(&member(
        "secret",
        "Secret",
        &format!("embargo_until: {}\n", date_in(30)),
    ))
    .await;
    let body = json(&batch(&app, json!({"entries": [member("open", "Open", "")]})).await);
    let open = id_of(&body, "open");
    let refers = app
        .put_as(
            &format!("/e/{open}"),
            &app.token,
            member("open", "Open", "replicates: slug:secret\n"),
        )
        .await;
    assert_eq!(
        refers.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refers.body
    );
    let unknown_run = app
        .put_as(
            &format!("/e/{open}"),
            &app.token,
            member("open", "Open", "uses_runs: [nope]\n"),
        )
        .await;
    assert_eq!(
        unknown_run.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        unknown_run.body
    );
}

#[tokio::test]
async fn an_entry_cant_take_an_embargo_while_an_earlier_public_entry_refers_to_it() {
    let app = test_app().await;
    let target = app.submit_held(&member("target", "Target", "")).await;
    app.submit_held(&member("referrer", "Referrer", "replicates: slug:target\n"))
        .await;
    let embargoed = app
        .put_as(
            &format!("/e/{target}"),
            &app.token,
            member(
                "target",
                "Target",
                &format!("embargo_until: {}\n", date_in(30)),
            ),
        )
        .await;
    assert_eq!(
        embargoed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        embargoed.body
    );
    assert!(
        embargoed.body.contains("refers to it"),
        "{}",
        embargoed.body
    );
}

#[tokio::test]
async fn a_batch_member_publishes_only_with_its_batch() {
    let app = test_app().await;
    let body = json(
        &batch(
            &app,
            json!({"entries": [member("a", "A", ""), member("b", "B", "")]}),
        )
        .await,
    );
    let a = id_of(&body, "a");
    let alone = app.post(&format!("/e/{a}/publish"), "").await;
    assert_eq!(alone.status, StatusCode::CONFLICT, "{}", alone.body);
    assert!(alone.body.contains("batch"), "{}", alone.body);
}

#[tokio::test]
async fn a_version_that_adds_a_slug_registers_it() {
    let app = test_app().await;
    let old = app.submit(&entry("T", "")).await;
    let version = common::id_in(
        &app.post(&format!("/e/{old}/versions"), member("named", "U", ""))
            .await
            .body,
    );
    app.post(&format!("/e/{version}/publish"), "").await;
    let reuse = batch(&app, json!({"entries": [member("named", "Other", "")]})).await;
    assert_eq!(
        reuse.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reuse.body
    );
}

#[tokio::test]
async fn a_runs_entries_are_listed_once_they_are_all_public() {
    let app = test_app().await;
    let body = json(
        &batch(
            &app,
            json!({"runs": [run()], "entries": [
        member("a", "A", "uses_runs: [sweep-1]\n"), member("b", "B", "uses_runs: [sweep-1]\n")]}),
        )
        .await,
    );
    app.post(
        &format!("/batches/{}/publish", body["id"].as_str().unwrap()),
        "",
    )
    .await;
    let (a, b) = (id_of(&body, "a"), id_of(&body, "b"));
    let page = app.get(&format!("/e/{a}")).await.body;
    let (_, below) = page.split_once("=== Author text follows").unwrap();
    assert!(below.contains("Sweep over layers"), "{page}");
    assert!(
        page.contains(&format!("run: {RUN}: entries {a} {b}"))
            || page.contains(&format!("run: {RUN}: entries {b} {a}")),
        "{page}"
    );
}
