//! Seam: the router, for the warnings only the server can give a submission: files
//! the body names that a reader can't open, and `replicates`/`reanalyzes` items that
//! name no entry (design.md, Agent interface).

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::attachments::{one_file, set_with, sha256, upload};
use common::{TestApp, entry, id_in, owner, test_app};
use serde_json::{Value, json};

const SCRIPT: &str = "print('benchmarks')\n";

/// `entry`, with `setup` as its Setup section.
fn with_setup(front: &str, setup: &str) -> String {
    entry("Benchmarks", front).replacen("not recorded", setup, 1)
}

/// The `warning: ` lines of a plain-text reply.
fn warnings(body: &str) -> Vec<&str> {
    body.lines()
        .filter_map(|line| line.strip_prefix("warning: "))
        .collect()
}

/// The warning about files a reader can't open, if there is one.
fn file_warning(body: &str) -> Option<String> {
    warnings(body)
        .into_iter()
        .find(|warning| warning.contains("can open only"))
        .map(str::to_string)
}

#[tokio::test]
async fn files_named_but_not_attached_are_warned_in_the_reply_and_the_author_s_read() {
    let app = test_app().await;
    let set = set_with(&app, &app.token, "logit_nash.py", SCRIPT).await;
    let text = with_setup(
        &format!("attachments: {set}\n"),
        "Recorded in `research/plan.md` and `notes/S.md`; `logit_nash.py` and \
         `src/logit_nash.py` print the prices, and `results.jsonl` holds them.",
    );
    let reply = app.post("/entries", text).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let warned = file_warning(&reply.body).expect("a file warning");
    assert!(
        warned.contains("`research/plan.md`, `notes/S.md`"),
        "{warned}"
    );
    assert!(!warned.contains("logit_nash"), "{warned}");
    assert!(!warned.contains("results.jsonl"), "{warned}");
    assert!(warned.contains("if the entry relies on them"), "{warned}");
    let id = id_in(&reply.body);
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(file_warning(&own.body), Some(warned));
}

#[tokio::test]
async fn with_no_set_and_no_code_link_every_file_named_is_warned() {
    let app = test_app().await;
    let names: String = (1..=12).map(|n| format!("`step{n}.py` ")).collect();
    let reply = app.post("/entries", with_setup("", &names)).await;
    let warned = file_warning(&reply.body).expect("a file warning");
    assert!(warned.contains("`step1.py`"), "{warned}");
    assert!(warned.contains("`step10.py` and 2 more"), "{warned}");
    assert!(!warned.contains("step11"), "{warned}");
    let clean = app
        .post("/entries", with_setup("", "No files named."))
        .await;
    assert_eq!(file_warning(&clean.body), None);
}

#[tokio::test]
async fn a_live_code_link_answers_for_the_files_and_a_dead_one_does_not() {
    let app = test_app().await;
    let live = "https://github.com/ada/steering/tree/0123456789abcdef0123456789abcdef01234567";
    let dead = "https://github.com/ada/gone/tree/0123456789abcdef0123456789abcdef01234567";
    app.web.answer(dead, 404);
    let setup = "Run `judge.py`.";
    let with_live = app
        .post("/entries", with_setup(&format!("code: {live}\n"), setup))
        .await;
    assert_eq!(file_warning(&with_live.body), None, "{}", with_live.body);
    let with_dead = app
        .post("/entries", with_setup(&format!("code: {dead}\n"), setup))
        .await;
    assert!(
        file_warning(&with_dead.body).is_some(),
        "{}",
        with_dead.body
    );
}

#[tokio::test]
async fn a_file_the_owner_removed_counts_as_not_attached() {
    let app = test_app().await;
    let set = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    let cookie = owner(&app).await;
    let removed = app
        .form(
            &format!("/admin/f/{}/remove", sha256(SCRIPT)),
            &[],
            Some(&cookie),
        )
        .await;
    assert_eq!(removed.status, StatusCode::SEE_OTHER, "{}", removed.body);
    let reply = app
        .post(
            "/entries",
            with_setup(&format!("attachments: {set}\n"), "Run `steer.py`."),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let warned = file_warning(&reply.body).expect("a file warning");
    assert!(warned.contains("`steer.py`"), "{warned}");
}

#[tokio::test]
async fn each_batch_member_is_checked_against_its_own_set() {
    let app = test_app().await;
    let first = set_with(&app, &app.token, "a.py", SCRIPT).await;
    let other = upload(&app, &app.token, one_file("b.py", "print('b')\n")).await;
    let second: Value = serde_json::from_str(&other.body).unwrap();
    let second = second["id"].as_str().unwrap();
    let body = json!({"entries": [
        with_setup(&format!("slug: a\nattachments: {first}\n"), "Run `a.py`."),
        with_setup(&format!("slug: b\nattachments: {second}\n"), "Run `a.py`."),
    ]});
    let reply = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let reply: Value = serde_json::from_str(&reply.body).unwrap();
    let file_warnings: Vec<usize> = reply["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| {
            member["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|warning| warning.as_str().unwrap().contains("can open only"))
                .count()
        })
        .collect();
    assert_eq!(file_warnings, vec![0, 1]);
}

#[tokio::test]
async fn a_batch_member_may_name_another_member_by_its_slug() {
    let app = test_app().await;
    let body = json!({"entries": [
        entry("Pilot", "slug: pilot\n"),
        entry("Main run", "slug: main\nreplicates: slug:pilot\n"),
    ]});
    let reply = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let reply: Value = serde_json::from_str(&reply.body).unwrap();
    let warned: Vec<&str> = reply["entries"][1]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|warning| warning.contains("names no"))
        .collect();
    assert_eq!(warned, Vec::<&str>::new());
}

async fn reference_warnings(app: &TestApp, front: &str) -> Vec<String> {
    let reply = app.post("/entries", entry("Follow-up", front)).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    warnings(&reply.body)
        .into_iter()
        .filter(|warning| warning.contains("names no"))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn references_that_name_no_entry_are_warned_without_telling_whose_it_is() {
    let app = test_app().await;
    let public = app.submit(&entry("Baseline", "slug: baseline\n")).await;
    let own_held = app.submit_held(&entry("Held", "")).await;
    let other = app.other_token().await;
    let theirs = app
        .post_as("/entries", Some(&other), entry("Theirs, held", ""))
        .await;
    let theirs_held = id_in(&theirs.body);
    for named in [
        "slug:baseline".to_string(),
        public.clone(),
        own_held,
        "arXiv:2603.20281".to_string(),
    ] {
        assert_eq!(
            // Quoted: YAML reads an ID such as `1e23456789` as a number.
            reference_warnings(&app, &format!("replicates: \"{named}\"\n")).await,
            Vec::<String>::new(),
            "{named}"
        );
    }
    let unknown_slug = reference_warnings(&app, "replicates: slug:nope\n").await;
    assert_eq!(unknown_slug.len(), 1, "{unknown_slug:?}");
    assert!(
        unknown_slug[0].contains("`slug:nope`"),
        "{}",
        unknown_slug[0]
    );
    assert!(
        unknown_slug[0].contains("use the entry's ID"),
        "{}",
        unknown_slug[0]
    );
    let unknown = reference_warnings(&app, "reanalyzes: [zzzzzzzzzz, x]\n").await;
    let hidden = reference_warnings(&app, &format!("reanalyzes: \"{theirs_held}\"\n")).await;
    assert_eq!(unknown.len(), 1, "{unknown:?}");
    assert!(
        unknown[0].contains("`zzzzzzzzzz` names no public entry"),
        "{}",
        unknown[0]
    );
    assert_eq!(
        hidden,
        vec![unknown[0].replace("zzzzzzzzzz", &theirs_held)],
        "another account's hidden entry reads as no entry at all"
    );
}
