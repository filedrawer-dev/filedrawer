//! Seam: `github_accounts::refresh_logins`, with GitHub's API faked, and what cards
//! show after it (design.md, Accounts; #119): cards show a linked account's current
//! login, confirmed within a day, through conditional calls that cost no call when
//! nothing changed.

use crate::common;

use common::{TestApp, field, test_app};
use filedrawer_server::github_accounts::refresh_logins;
use serde_json::json;
use time::{Duration, OffsetDateTime};

/// Links the app's account to GitHub account 7 as `ada-gh`, last confirmed
/// `hours_ago`, with ETag `e1`, and gives it a public entry. Returns the entry's ID.
async fn linked(app: &TestApp, hours_ago: i64) -> String {
    let confirmed = common::stamp(OffsetDateTime::now_utc() - Duration::hours(hours_ago));
    sqlx::query(
        "INSERT INTO github_links
           (account_id, github_id, login, linked_at, refreshed_at, etag)
         VALUES (?, 7, 'ada-gh', ?, ?, 'e1')",
    )
    .bind(&app.account)
    .bind(&confirmed)
    .bind(&confirmed)
    .execute(&app.pool)
    .await
    .unwrap();
    app.submit(&common::entry("Steering", "")).await
}

async fn author(app: &TestApp, id: &str) -> String {
    let page = app.get(&format!("/e/{id}")).await;
    field(&page.body, "author").unwrap().to_string()
}

async fn refresh(app: &TestApp) -> filedrawer_server::github_accounts::Refreshed {
    refresh_logins(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_rename_reaches_the_card_and_the_next_call_sends_its_etag() {
    let app = test_app().await;
    // Confirmed just now, so publishing the entry asks GitHub nothing.
    let id = linked(&app, 0).await;
    app.web
        .api_tagged("/user/7", json!({"id": 7, "login": "ada-renamed"}), "e2");
    refresh(&app).await;
    assert_eq!(
        author(&app, &id).await,
        format!("account {}, github:ada-renamed, Ada", app.account)
    );
    // Unchanged since: GitHub answers 304, and the login stays confirmed.
    refresh(&app).await;
    let sent = app.web.sent_etags();
    assert_eq!(
        sent,
        vec![
            ("/user/7".to_string(), Some("e1".to_string())),
            ("/user/7".to_string(), Some("e2".to_string())),
        ]
    );
}

#[tokio::test]
async fn a_login_unconfirmed_for_a_day_shows_on_no_card_until_github_confirms_it() {
    let app = test_app().await;
    let id = linked(&app, 25).await;
    let plain = format!("account {}, Ada", app.account);
    assert_eq!(author(&app, &id).await, plain);
    app.web
        .api_tagged("/user/7", json!({"id": 7, "login": "ada-gh"}), "e1");
    refresh(&app).await;
    assert_eq!(
        author(&app, &id).await,
        format!("account {}, github:ada-gh, Ada", app.account)
    );
}

#[tokio::test]
async fn a_404_hides_the_handle_and_a_later_answer_brings_it_back() {
    let app = test_app().await;
    let id = linked(&app, 1).await;
    app.web
        .api("/user/7", 404, json!({"message": "Not Found"}), 4999);
    refresh(&app).await;
    assert_eq!(
        author(&app, &id).await,
        format!("account {}, Ada", app.account)
    );
    app.web
        .api_tagged("/user/7", json!({"id": 7, "login": "ada-gh"}), "e3");
    refresh(&app).await;
    assert_eq!(
        author(&app, &id).await,
        format!("account {}, github:ada-gh, Ada", app.account)
    );
}

#[tokio::test]
async fn an_answer_for_another_id_or_with_no_login_changes_nothing() {
    for body in [json!({"id": 8, "login": "someone"}), json!({"id": 7})] {
        let app = test_app().await;
        let id = linked(&app, 1).await;
        app.web.api_tagged("/user/7", body, "e2");
        refresh(&app).await;
        let expected = format!("account {}, github:ada-gh, Ada", app.account);
        assert_eq!(author(&app, &id).await, expected);
    }
}

#[tokio::test]
async fn an_unreadable_answer_for_one_account_doesnt_stop_the_others() {
    let app = test_app().await;
    linked(&app, 2).await;
    let other_token = app.other_token().await;
    let other: String =
        sqlx::query_scalar("SELECT id FROM accounts WHERE display_name = 'Mallory'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO github_links (account_id, github_id, login, linked_at, refreshed_at)
         VALUES (?, 9, 'mallory-gh', '2026-01-01T00:00:00Z', ?)",
    )
    .bind(&other)
    .bind(common::stamp(OffsetDateTime::now_utc()))
    .execute(&app.pool)
    .await
    .unwrap();
    let reply = app
        .post_as("/entries", Some(&other_token), common::entry("Other", ""))
        .await;
    assert_eq!(
        reply.status,
        axum::http::StatusCode::CREATED,
        "{}",
        reply.body
    );
    // Account 7, confirmed longest ago, is asked first, and answers for another ID.
    app.web
        .api_tagged("/user/7", json!({"id": 8, "login": "someone"}), "e2");
    app.web
        .api_tagged("/user/9", json!({"id": 9, "login": "mallory-new"}), "e9");
    let done = refresh(&app).await;
    assert_eq!((done.asked, done.stopped), (2, false));
    let login: String = sqlx::query_scalar("SELECT login FROM github_links WHERE github_id = 9")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(login, "mallory-new");
}

#[tokio::test]
async fn an_account_whose_entry_waits_in_the_hold_is_refreshed_too() {
    let app = test_app().await;
    let confirmed = common::stamp(OffsetDateTime::now_utc() - Duration::hours(2));
    sqlx::query(
        "INSERT INTO github_links (account_id, github_id, login, linked_at, refreshed_at)
         VALUES (?, 7, 'ada-gh', ?, ?)",
    )
    .bind(&app.account)
    .bind(&confirmed)
    .bind(&confirmed)
    .execute(&app.pool)
    .await
    .unwrap();
    app.submit_held(&common::entry("Steering", "")).await;
    app.web
        .api_tagged("/user/7", json!({"id": 7, "login": "ada-gh"}), "e1");
    assert_eq!(refresh(&app).await.asked, 1);
}

#[tokio::test]
async fn an_entry_that_publishes_refreshes_its_accounts_login_first() {
    let app = test_app().await;
    let confirmed = common::stamp(OffsetDateTime::now_utc() - Duration::hours(2));
    sqlx::query(
        "INSERT INTO github_links (account_id, github_id, login, linked_at, refreshed_at, etag)
         VALUES (?, 7, 'ada-gh', ?, ?, 'e1')",
    )
    .bind(&app.account)
    .bind(&confirmed)
    .bind(&confirmed)
    .execute(&app.pool)
    .await
    .unwrap();
    app.web
        .api_tagged("/user/7", json!({"id": 7, "login": "ada-renamed"}), "e2");
    let id = app.submit(&common::entry("Steering", "")).await;
    assert_eq!(
        author(&app, &id).await,
        format!("account {}, github:ada-renamed, Ada", app.account)
    );
}

#[tokio::test]
async fn the_refresh_stops_once_a_quarter_of_the_hours_calls_are_spent() {
    let app = test_app().await;
    linked(&app, 1).await;
    // 3,000 of 5,000 left: the link checks' rechecks stop here, and so does this.
    app.web.api("/user/7", 304, json!({}), 3000);
    assert_eq!(refresh(&app).await.asked, 1);
    assert_eq!(refresh(&app).await.asked, 0);
}

#[tokio::test]
async fn with_link_checks_off_nothing_is_asked() {
    let mut app = test_app().await;
    app.state.link_checker =
        std::sync::Arc::new(filedrawer_server::link_checks::LinkChecker::off());
    linked(&app, 1).await;
    assert_eq!(refresh(&app).await.asked, 0);
}

#[tokio::test]
async fn only_accounts_with_an_entry_are_asked_about() {
    let app = test_app().await;
    let other = filedrawer_server::admin::create_account(&app.pool, "Other")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO github_links (account_id, github_id, login, linked_at, refreshed_at)
         VALUES (?, 9, 'other', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(&other)
    .execute(&app.pool)
    .await
    .unwrap();
    let done = refresh(&app).await;
    assert_eq!(done.asked, 0);
    assert!(app.web.api_calls().is_empty());
}

#[tokio::test]
async fn without_a_token_or_while_github_asks_to_wait_nothing_is_asked() {
    // 60 an hour is GitHub's allowance without a token.
    let app = test_app().await.with_github_hourly(60);
    linked(&app, 1).await;
    assert_eq!(refresh(&app).await.asked, 0);
    let app = test_app().await;
    linked(&app, 1).await;
    app.web
        .api_after("/user/7", 429, json!({}), 4000, Some(600));
    let done = refresh(&app).await;
    assert!(done.stopped);
    assert_eq!(refresh(&app).await.asked, 0, "the pause holds");
}
