//! Seam: the router, for deleting an account from a browser session (design.md,
//! Accounts): everything not yet public goes, public entries stay without the name.

mod common;

use axum::http::StatusCode;
use common::series::{citing, create, make_public, submit_with};
use common::{DELIMITER, PASSWORD, TestApp, browser_account, date_in, entry, test_app};

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
    let embargoed = submit_with(
        &app,
        &token,
        &entry("Held result", &format!("embargo_until: {}\n", date_in(30))),
    )
    .await;
    let pending = app
        .post_as("/entries", Some(&token), entry("Draft", ""))
        .await;
    assert_eq!(pending.status, StatusCode::CREATED);

    let page = app.page("/account/delete", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains("2 entries that aren't public"),
        "{}",
        page.body
    );
    assert!(page.body.contains("1 public entry"), "{}", page.body);

    let wrong = delete(&app, &cookie, "not the password").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.body);
    assert_eq!(
        app.get(&format!("/e/{public}")).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.get_with_token(&format!("/e/{embargoed}"), &token)
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
        Some("/login")
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
    assert_eq!(page.location(), Some("/login"));
    let still = app
        .post_as("/entries", Some(&token), entry("Still here", ""))
        .await;
    assert_eq!(still.status, StatusCode::CREATED);
}

#[tokio::test]
async fn deletion_waits_while_a_hidden_series_holds_another_accounts_entries() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let (series, key) = create(&app, &token).await;
    let (empty, _) = create(&app, &token).await;
    let other = app.other_token().await;
    app.post_as(&format!("/series/{series}/join"), Some(&other), key)
        .await;
    let theirs = submit_with(&app, &other, &citing("Theirs", &format!("{series}@1"), "")).await;

    let page = app.page("/account/delete", Some(&cookie)).await;
    assert!(page.body.contains(&series), "{}", page.body);
    let refused = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(refused.body.contains(&series), "{}", refused.body);
    assert!(!refused.body.contains(&empty), "{}", refused.body);

    app.post_as(
        &format!("/series/{series}/entries/{theirs}/remove"),
        Some(&token),
        "",
    )
    .await;
    let done = delete(&app, &cookie, PASSWORD).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    // Its hidden series went with it; the other account's entry stays, out of it.
    assert_eq!(
        app.get_with_token(&format!("/series/{series}"), &other)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get(&format!("/e/{theirs}")).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_public_series_outlives_its_creator() {
    let app = test_app().await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    let (series, _) = create(&app, &token).await;
    let id = submit_with(&app, &token, &citing("Cites", &format!("{series}@1"), "")).await;
    make_public(&app, &cookie, &series).await;
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
async fn account_id(app: &TestApp, cookie: &str) -> String {
    let page = app.page("/account", Some(cookie)).await;
    let start = page
        .body
        .find("<code>a-")
        .expect("the page shows the account ID")
        + 6;
    let end = start + page.body[start..].find('<').unwrap();
    page.body[start..end].to_string()
}

#[tokio::test]
async fn deleting_an_account_frees_a_version_only_its_entries_held_back() {
    let app = test_app().await;
    let (owner_cookie, owner) = browser_account(&app, "ada@example.com").await;
    let (series, key) = create(&app, &owner).await;
    make_public(&app, &owner_cookie, &series).await;
    let (cookie, token) = browser_account(&app, EMAIL).await;
    app.post_as(&format!("/series/{series}/join"), Some(&token), key)
        .await;
    submit_with(
        &app,
        &token,
        &citing(
            "Held",
            &format!("{series}@1"),
            &format!("embargo_until: {}\n", date_in(30)),
        ),
    )
    .await;
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
