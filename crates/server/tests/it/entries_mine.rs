//! Seam: the router, for `GET /entries/mine` (design.md, Agent interface): a token's
//! account's entries, newest first, one line each, so an agent can skip results the
//! account already posted.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{DELIMITER, TestApp, entry, test_app};
use serde_json::{Value, json};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn code(repo: &str) -> String {
    format!("https://github.com/example/{repo}/tree/{COMMIT}")
}

/// The lines below the delimiter of the account's listing at `path`.
async fn listed(app: &TestApp, path: &str) -> Vec<String> {
    let reply = app.get_as_author(path).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (_, below) = reply.body.split_once(DELIMITER).expect("a delimiter");
    below
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_token_lists_its_accounts_entries_newest_first() {
    let app = test_app().await;
    let first = app
        .submit(&entry(
            "First result",
            &format!("slug: first\ncode: {}\n", code("steering")),
        ))
        .await;
    let second = app.submit_held(&entry("Second result", "")).await;
    let removed = app.submit_held(&entry("Removed one", "")).await;
    // Only the archive's owner removes an entry.
    sqlx::query("UPDATE entries SET status = 'removed' WHERE id = ?")
        .bind(&removed)
        .execute(&app.pool)
        .await
        .unwrap();
    let other = app.other_token().await;
    let theirs = app
        .post_as("/entries", Some(&other), entry("Not mine", ""))
        .await;
    assert_eq!(theirs.status, StatusCode::CREATED);

    let reply = app.get_as_author("/entries/mine").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (above, _) = reply.body.split_once(DELIMITER).unwrap();
    assert!(
        above.contains(&format!("account: {} (Ada)\n", app.account)),
        "{}",
        reply.body
    );
    assert_eq!(
        listed(&app, "/entries/mine").await,
        vec![
            format!("{second}\tpending\t-\tSecond result"),
            format!("{first}\tpublic\tfirst\tFirst result\t{}", code("steering")),
        ]
    );

    let reply = app
        .send(
            Request::get("/entries/mine")
                .header("authorization", format!("Bearer {}", app.token))
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let body: Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(
        body["account"],
        json!({"id": app.account, "display_name": "Ada"})
    );
    assert_eq!(
        body["entries"],
        json!([
            {"id": second, "status": "pending", "slug": null, "title": "Second result", "code": null},
            {"id": first, "status": "public", "slug": "first", "title": "First result", "code": code("steering")},
        ])
    );
}

#[tokio::test]
async fn limit_and_page_step_through_the_list() {
    let app = test_app().await;
    let mut ids = Vec::new();
    for n in 1..=3 {
        ids.push(app.submit_held(&entry(&format!("Result {n}"), "")).await);
    }
    let ids_in = |lines: Vec<String>| -> Vec<String> {
        lines
            .iter()
            .map(|line| line.split('\t').next().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        ids_in(listed(&app, "/entries/mine?limit=2").await),
        vec![ids[2].clone(), ids[1].clone()]
    );
    assert_eq!(
        ids_in(listed(&app, "/entries/mine?limit=2&page=2").await),
        vec![ids[0].clone()]
    );
}

#[tokio::test]
async fn code_keeps_the_entries_whose_code_link_starts_with_that_repo() {
    let app = test_app().await;
    let steering = app
        .submit_held(&entry("Steering", &format!("code: {}\n", code("steering"))))
        .await;
    app.submit_held(&entry(
        "Other repo",
        &format!("code: {}\n", code("steering-two")),
    ))
    .await;
    app.submit_held(&entry("No code", "")).await;
    for repo in [
        "https%3A%2F%2Fgithub.com%2Fexample%2Fsteering",
        "https%3A%2F%2Fgithub.com%2Fexample%2Fsteering%2F",
    ] {
        let lines = listed(&app, &format!("/entries/mine?code={repo}")).await;
        assert_eq!(lines.len(), 1, "{repo}: {lines:?}");
        assert!(lines[0].starts_with(&steering), "{lines:?}");
    }
}

#[tokio::test]
async fn listing_needs_a_token() {
    let app = test_app().await;
    let reply = app.get("/entries/mine").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
}
