//! Seam: the router, for deleting an account from a browser session (design.md,
//! Accounts): everything not yet public goes, public entries stay without the name.

use crate::common;

use axum::http::StatusCode;
use common::series::{citing, create, submit_with};
use common::{DELIMITER, PASSWORD, TestApp, account_id, browser_account, entry, test_app};

const EMAIL: &str = "grace@example.com";

async fn delete(app: &TestApp, cookie: &str, password: &str) -> common::Reply {
    app.form("/account/delete", &[("password", password)], Some(cookie))
        .await
}

#[tokio::test]
async fn deleting_an_account_keeps_its_public_entries_without_its_name() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let public = submit_with(&app, &token, &entry("Public result", "")).await;
    let held = app
        .post_as("/entries", Some(&token), entry("Held result", ""))
        .await;
    let held = common::id_in(&held.body);
    let pending = app
        .post_as("/entries", Some(&token), entry("Draft", ""))
        .await;
    assert_eq!(pending.status, StatusCode::CREATED);

    let page = app.page("/account/delete", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains(r#"<span class="unpublished">2</span>"#),
        "{}",
        page.body
    );
    assert!(
        page.body.contains(r#"<span class="public">1</span>"#),
        "{}",
        page.body
    );

    let wrong = delete(&app, &cookie, "not the password").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.body);
    assert_eq!(
        app.get(&format!("/e/{public}")).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.get_with_token(&format!("/e/{held}"), &token)
            .await
            .status,
        StatusCode::OK,
        "a wrong password deletes nothing"
    );

    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);

    let read = app.get(&format!("/e/{public}")).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    let (above, _) = read.body.split_once(DELIMITER).unwrap();
    assert!(
        above.contains("author: account deleted, account a-"),
        "{}",
        read.body
    );
    assert!(!read.body.contains("Grace"), "{}", read.body);
    assert!(
        read.body.contains("account deleted, a-"),
        "the card says so: {}",
        read.body
    );
    for path in ["/cards.txt", "/search?q=result"] {
        let body = app.get(path).await.body;
        assert!(body.contains("account deleted"), "{path}: {body}");
        assert!(!body.contains("Grace"), "{path}: {body}");
    }

    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(left, 1, "only the public entry is left");
    let names: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM accounts WHERE display_name = 'Grace' OR email IS NOT NULL AND email LIKE 'grace%'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(names, 0, "the name and email are gone");

    assert_eq!(
        app.get_with_token("/tags", &token).await.status,
        StatusCode::OK,
        "reading needs no token"
    );
    assert_eq!(
        app.post_as("/entries", Some(&token), entry("After", ""))
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "the token is gone"
    );
    assert_eq!(
        app.page("/account", Some(&cookie)).await.location(),
        Some("/login?next=%2Faccount")
    );
    let login = app
        .form("/login", &[("email", EMAIL), ("password", PASSWORD)], None)
        .await;
    assert_eq!(login.status, StatusCode::UNAUTHORIZED);

    // The address is free to sign up again.
    let (_, again) = browser_account(&app, EMAIL).await;
    assert!(again.starts_with("fdt_"));
}

#[tokio::test]
async fn deleting_needs_a_browser_session() {
    let app = test_app().await;
    let (_, token) = browser_account(&app, EMAIL).await;
    let by_token = app.post_as("/account/delete", Some(&token), "").await;
    assert_eq!(by_token.status, StatusCode::FORBIDDEN, "{}", by_token.body);
    let page = app.page("/account/delete", None).await;
    assert_eq!(page.location(), Some("/login?next=%2Faccount%2Fdelete"));
    let still = app
        .post_as("/entries", Some(&token), entry("Still here", ""))
        .await;
    assert_eq!(still.status, StatusCode::CREATED);
}

#[tokio::test]
async fn deletion_takes_the_series_no_one_can_read_unless_another_accounts_entry_cites_it() {
    // The owner's decision, 2026-11-17.
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let (unread, _) = create(&app, &token).await;
    let (theirs_cite, key) = create(&app, &token).await;
    let other = app.other_token().await;
    app.post_as(&format!("/series/{theirs_cite}/join"), Some(&other), key)
        .await;
    let pending = app
        .post_as(
            "/entries",
            Some(&other),
            citing("Theirs", &format!("{theirs_cite}@1"), ""),
        )
        .await;
    assert_eq!(pending.status, StatusCode::CREATED, "{}", pending.body);
    assert_eq!(
        app.get(&format!("/series/{unread}")).await.status,
        StatusCode::OK,
        "every series is public"
    );

    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(
        app.get(&format!("/series/{unread}")).await.status,
        StatusCode::NOT_FOUND,
        "a series no one could read outlived its creator"
    );
    // The one another account's entry cites stays, for that entry.
    let kept = app
        .get_with_token(&format!("/series/{theirs_cite}"), &other)
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.body);
    assert!(
        kept.body.contains("judge-b scores each reply"),
        "{}",
        kept.body
    );
    let theirs = common::id_in(&pending.body);
    let read = app.get_with_token(&format!("/e/{theirs}"), &other).await;
    assert!(
        read.body.contains(&format!("series: {theirs_cite}@1")),
        "{}",
        read.body
    );
}

#[tokio::test]
async fn another_accounts_entry_rejected_in_the_last_60_days_keeps_the_series() {
    // The owner's decision, 2026-11-17: a rejection may be the owner's mistake, and
    // its author fixes the entry and resubmits, so the series it cites stays.
    for (days_ago, kept) in [(59, true), (61, false)] {
        let app = test_app().await;
        let (cookie, token) = browser_account(&app, EMAIL).await;
        let (series, key) = create(&app, &token).await;
        let other = app.other_token().await;
        app.post_as(&format!("/series/{series}/join"), Some(&other), key)
            .await;
        let cited = app
            .post_as(
                "/entries",
                Some(&other),
                citing("Theirs", &format!("{series}@1"), ""),
            )
            .await;
        assert_eq!(cited.status, StatusCode::CREATED, "{}", cited.body);
        sqlx::query(
            "UPDATE entries SET status = 'rejected',
                 submitted_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)
             WHERE id = ?",
        )
        .bind(format!("-{days_ago} days"))
        .bind(common::id_in(&cited.body))
        .execute(&app.pool)
        .await
        .unwrap();

        let done = delete(&app, &cookie, PASSWORD).await;
        assert_eq!(done.status, StatusCode::OK, "{}", done.body);
        let status = app.get(&format!("/series/{series}")).await.status;
        let expected = if kept {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        };
        assert_eq!(status, expected, "rejected {days_ago} days ago");
    }
}

#[tokio::test]
async fn a_readable_series_outlives_its_creator() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let (series, _) = create(&app, &token).await;
    let id = submit_with(&app, &token, &citing("Cites", &format!("{series}@1"), "")).await;
    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let page = app.get(&format!("/series/{series}")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("judge-b scores each reply"),
        "{}",
        page.body
    );
    assert!(page.body.contains(&format!("entry: {id}")), "{}", page.body);
    assert!(page.body.contains("account deleted"), "{}", page.body);
    assert!(!page.body.contains("Grace"), "{}", page.body);
}

/// The account ID the account page shows for a session.
#[tokio::test]
async fn deleting_an_account_frees_a_version_only_its_entries_held_back() {
    let app = test_app().await;
    let (_, owner) = browser_account(&app, "ada@example.com").await;
    let (series, key) = create(&app, &owner).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    app.post_as(&format!("/series/{series}/join"), Some(&token), key)
        .await;
    let held = app
        .post_as(
            "/entries",
            Some(&token),
            citing("Held", &format!("{series}@1"), ""),
        )
        .await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    submit_with(&app, &owner, &citing("Public", &format!("{series}@1"), "")).await;
    let before = app.get(&format!("/series/{series}@1")).await;
    assert!(!before.body.contains("judge-b"), "{}", before.body);

    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let after = app.get(&format!("/series/{series}@1")).await;
    assert!(
        after.body.contains("judge-b scores each reply"),
        "{}",
        after.body
    );
}

#[tokio::test]
async fn a_deleted_account_gets_no_new_token() {
    let app = test_app().await;
    let (cookie, _) = browser_account(&app, EMAIL).await;
    let id = account_id(&app, &cookie).await;
    delete(&app, &cookie, PASSWORD).await;
    let made = filedrawer_server::admin::create_token(&app.pool, &id, "late").await;
    assert!(made.is_err(), "a token was made for a deleted account");
}

#[tokio::test]
async fn deleting_keeps_the_cost_log_of_its_screened_entries() {
    let models = common::fake_models::FakeModels::default();
    let app = common::test_app_with(models.screener(), 1_000_000_000).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let rejected = app
        .post_as("/entries", Some(&token), entry("FAKE-OFFTOPIC cake", ""))
        .await;
    let id = common::id_in(&rejected.body);
    app.post_as(&format!("/e/{id}/publish"), Some(&token), "")
        .await;
    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let logged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screenings")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(logged, 1, "the spend still counts against the day");
}

#[tokio::test]
async fn deleting_drops_its_undecided_and_dismissed_refused_series_but_keeps_confirmed_ones() {
    let models = common::fake_models::FakeModels::default();
    let app = common::test_app_with(models.screener(), 1_000_000_000).await;
    let owner = common::owner(&app).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let refuse = async |title: &str| {
        let reply = app
            .post_as(
                "/series",
                Some(&token),
                common::series::series_doc(title, "Setup FAKE-HAZARD FAKE-SONNET-HAZARD."),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
    };
    let first_refusal = async || {
        let page = app.page("/admin", Some(&owner)).await.body;
        let start = page.find("/admin/series-refusals/").unwrap() + 23;
        let end = start + page[start..].find('/').unwrap();
        page[start..end].to_string()
    };
    refuse("Confirmed family").await;
    let confirmed = first_refusal().await;
    app.form(
        &format!("/admin/series-refusals/{confirmed}/confirm"),
        &[],
        Some(&owner),
    )
    .await;
    // Confirming guards the account, which would refuse the rest unrecorded.
    let account =
        sqlx::query_scalar::<_, String>("SELECT account_id FROM refused_series WHERE id = ?")
            .bind(&confirmed)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    app.form(
        &format!("/admin/accounts/{account}/clear-guard"),
        &[],
        Some(&owner),
    )
    .await;
    refuse("Dismissed family").await;
    let dismissed = first_refusal().await;
    app.form(
        &format!("/admin/series-refusals/{dismissed}/dismiss"),
        &[],
        Some(&owner),
    )
    .await;
    refuse("Undecided family").await;

    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let queue = app.page("/admin", Some(&owner)).await;
    assert!(!queue.body.contains("Undecided family"), "{}", queue.body);
    let kept: Vec<(String,)> = sqlx::query_as("SELECT title FROM refused_series")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert_eq!(kept, vec![("Confirmed family".to_string(),)]);
}

#[tokio::test]
async fn deleting_drops_the_attachment_sets_no_kept_entry_cites_and_their_lone_files() {
    let app = common::test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let upload = async |text: &str| -> String {
        let reply = app
            .send(
                axum::http::Request::post("/attachments")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .header("accept", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({"files": [
                            {"origin": "author:shared.py", "text": "print('shared')\n"},
                            {"origin": "author:own.py", "text": text},
                        ]})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        serde_json::from_str::<serde_json::Value>(&reply.body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let kept = upload("print('kept')\n").await;
    let held = upload("print('held')\n").await;
    upload("print('unused')\n").await;
    let public = app
        .post_as(
            "/entries",
            Some(&token),
            entry("Public", &format!("slug: p\nattachments: {kept}\n")),
        )
        .await;
    let public = common::id_in(&public.body);
    app.post_as(&format!("/e/{public}/publish"), Some(&token), "")
        .await;
    // Publishing early leaves an entry citing a set to the worker.
    filedrawer_server::jobs::publish_due(&app.state, time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    app.post_as(
        "/entries",
        Some(&token),
        entry("Held", &format!("slug: h\nattachments: {held}\n")),
    )
    .await;

    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let sets: Vec<(String,)> = sqlx::query_as("SELECT id FROM attachment_sets")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert_eq!(sets, vec![(kept.clone(),)]);
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM files")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(records, 2);
    assert_eq!(
        common::stored_texts(&app),
        vec!["print('kept')\n", "print('shared')\n"]
    );
}
