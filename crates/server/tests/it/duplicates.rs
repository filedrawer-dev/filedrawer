//! Seam: the router, for identical text from an unclaimed account (design.md,
//! Limits and abuse; #130): an account an agent made can't post text that
//! is already public, that it holds itself, or that another unclaimed account has
//! waiting, since a run with no lasting home folder makes a new account each time
//! and can't see what the last one posted.

use crate::common;

use axum::http::StatusCode;
use common::attachments::{citing, set_with};
use common::{Reply, TestApp, agent_account, entry, id_in, test_app};

async fn post(app: &TestApp, token: &str, text: &str) -> Reply {
    app.post_as("/entries", Some(token), text.to_string()).await
}

/// Posts `text` with `token`, which must be accepted, and returns the entry's ID.
async fn posted(app: &TestApp, token: &str, text: &str) -> String {
    let reply = post(app, token, text).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    id_in(&reply.body)
}

async fn publish(app: &TestApp, token: &str, id: &str) {
    let path = format!("/e/{id}/publish");
    let reply = app.post_as(&path, Some(token), "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn an_unclaimed_account_cant_post_what_is_public() {
    let app = test_app().await;
    let text = entry("Steering", "");
    let id = app.submit(&text).await;
    let (_, token) = agent_account(&app).await;
    let refused = post(&app, &token, &text).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    // The refusal names the entry that holds the text.
    let url = format!("http://localhost/e/{id}");
    assert!(refused.body.contains(&url), "{}", refused.body);
    // Other text posts.
    posted(&app, &token, &entry("Steering again", "")).await;
    // An account with an owner sees what it posted, and isn't checked.
    posted(&app, &app.token, &text).await;
}

#[tokio::test]
async fn an_unclaimed_account_cant_post_what_another_has_waiting() {
    let app = test_app().await;
    let text = entry("Steering", "");
    let (_, first) = agent_account(&app).await;
    let (_, second) = agent_account(&app).await;
    let id = posted(&app, &first, &text).await;
    let refused = post(&app, &second, &text).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    // The waiting entry isn't the second account's to read, so it isn't named.
    assert!(!refused.body.contains(&id), "{}", refused.body);
}

#[tokio::test]
async fn a_hidden_entry_of_an_account_with_an_owner_refuses_nothing() {
    let app = test_app().await;
    let text = entry("Steering", "");
    app.submit_held(&text).await;
    let (_, token) = agent_account(&app).await;
    // A refusal would say the text exists, to an account that can't read it.
    posted(&app, &token, &text).await;
}

#[tokio::test]
async fn an_unclaimed_account_cant_post_its_own_text_twice() {
    let app = test_app().await;
    let text = entry("Steering", "");
    let (_, token) = agent_account(&app).await;
    let id = posted(&app, &token, &text).await;
    let refused = post(&app, &token, &text).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(refused.body.contains(&id), "{}", refused.body);
    // Sending the same text again to the entry itself restarts its hold, as ever.
    let path = format!("/e/{id}");
    let again = app.put_as(&path, &token, text.clone()).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    // But not to another of its entries.
    let other = posted(&app, &token, &entry("Steering more", "")).await;
    let copied = app
        .put_as(&format!("/e/{other}"), &token, text.clone())
        .await;
    assert_eq!(copied.status, StatusCode::CONFLICT, "{}", copied.body);
    // Nor as a new version of a public entry.
    publish(&app, &token, &id).await;
    let version = app
        .post_as(&format!("/e/{id}/versions"), Some(&token), text)
        .await;
    assert_eq!(version.status, StatusCode::CONFLICT, "{}", version.body);
}

#[tokio::test]
async fn a_batch_lists_a_posted_member_beside_its_other_problems() {
    let app = test_app().await;
    let text = entry("Steering", "slug: one\n");
    let id = app.submit(&text).await;
    let (_, token) = agent_account(&app).await;
    // The second member has no slug, which a batch needs.
    let members = [text, entry("Steering two", "")];
    let body = serde_json::json!({ "entries": members }).to_string();
    let refused = app.post_as("/entries/batch", Some(&token), body).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.body
    );
    assert!(refused.body.contains("needs a `slug`"), "{}", refused.body);
    assert!(refused.body.contains(&id), "{}", refused.body);
}

#[tokio::test]
async fn text_beyond_ascii_and_past_a_nul_is_compared_whole() {
    let app = test_app().await;
    let (_, token) = agent_account(&app).await;
    let texts = [
        entry("Stéering ☃ 😀", ""),
        entry("Steering", "") + "A note with a NUL \u{0} in it.\n",
    ];
    for text in texts {
        let id = app.submit(&text).await;
        let refused = post(&app, &token, &text).await;
        assert_eq!(refused.status, StatusCode::CONFLICT, "{text:?}");
        assert!(refused.body.contains(&id), "{}", refused.body);
    }
}

#[tokio::test]
async fn a_refused_duplicate_takes_no_allowance() {
    let app = test_app().await;
    let text = entry("Steering", "");
    app.submit(&text).await;
    let (_, token) = agent_account(&app).await;
    for _ in 0..3 {
        post(&app, &token, &text).await;
    }
    // All 5 of the day's units are still there.
    for n in 0..5 {
        let reply = post(&app, &token, &entry(&format!("Steering {n}"), "")).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        let waits = common::field(&reply.body, "screen_from");
        assert_eq!(waits, None, "{}", reply.body);
    }
}

#[tokio::test]
async fn text_posted_with_attached_code_cant_be_posted_without() {
    let app = test_app().await;
    let (_, first) = agent_account(&app).await;
    let set = set_with(&app, &first, "run.py", "print(1)\n").await;
    // A comment on the line goes with it.
    let with_code = entry(
        "Steering",
        &format!("attachments: {set}  # the code that ran, attached since the repo is private\n"),
    );
    let id = posted(&app, &first, &with_code).await;
    let refused = post(&app, &first, &entry("Steering", "")).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(refused.body.contains(&id), "{}", refused.body);
}

#[tokio::test]
async fn attached_code_doesnt_make_text_new() {
    let app = test_app().await;
    let id = app.submit(&entry("Steering", "")).await;
    let (_, token) = agent_account(&app).await;
    let set = set_with(&app, &token, "run.py", "print(1)\n").await;
    let refused = post(&app, &token, &citing("Steering", &set)).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(refused.body.contains(&id), "{}", refused.body);
}

#[tokio::test]
async fn a_batch_member_that_is_public_already_refuses_the_batch() {
    let app = test_app().await;
    let text = entry("Steering", "slug: one\n");
    let id = app.submit(&text).await;
    let (_, token) = agent_account(&app).await;
    let members = [text, entry("Steering two", "slug: two\n")];
    let body = serde_json::json!({ "entries": members }).to_string();
    let refused = app.post_as("/entries/batch", Some(&token), body).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(refused.body.contains(&id), "{}", refused.body);
}
