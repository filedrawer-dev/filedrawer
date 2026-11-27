//! Seam: the router, with GitHub faked behind `GithubOAuth`, for GitHub sign-in
//! (design.md, Accounts; #119): signing in and up with GitHub, linking and
//! unlinking, the handle on cards, deletion without a password, the exemption from
//! the new-account limit, and the limits on GitHub signups.

use crate::common;

use axum::http::StatusCode;
use common::fake_github::{CLIENT_ID, FakeGithub, WEB, octo};
use common::{PASSWORD, TestApp, browser_account, field, logged_in, test_app};
use filedrawer_server::github_login::Failure;
use filedrawer_server::limits::{Limits, Window};

/// The app with GitHub sign-in on, and its fake GitHub.
async fn github_app() -> (TestApp, FakeGithub) {
    let github = FakeGithub::default();
    let app = test_app().await.with_github(&github);
    (app, github)
}

/// The `state` GitHub would echo, from the authorize URL the start redirected to.
fn state_in(location: &str) -> String {
    let url = url::Url::parse(location).unwrap();
    url.query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("the authorize URL has a state")
}

/// Starts a sign-in (`path` `/login/github`) or a link (`/account/github/link`)
/// with `cookies`, and returns GitHub's state and the browser's start cookie.
async fn start(app: &TestApp, path: &str, cookies: Option<&str>) -> (String, String) {
    let reply = app.form(path, &[], cookies).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    let state = state_in(reply.location().unwrap());
    (state, reply.cookie().expect("the start sets its cookie"))
}

/// Comes back from GitHub with `code`, carrying `cookies`.
async fn come_back(app: &TestApp, state: &str, code: &str, cookies: &str) -> common::Reply {
    let path = format!("/login/github/callback?code={code}&state={state}");
    app.page(&path, Some(cookies)).await
}

/// Signs up with GitHub through `code`, which the test gave an identity, keeping
/// its first address, and returns the session cookie.
async fn github_signup(app: &TestApp, code: &str) -> String {
    let (state, browser) = start(app, "/login/github", None).await;
    let back = come_back(app, &state, code, &browser).await;
    assert_eq!(back.location(), Some("/signup/github"), "{}", back.body);
    let pending = back.cookie().unwrap();
    let done = app
        .form(
            "/signup/github",
            &[("display_name", "Octo"), ("email", "0")],
            Some(&pending),
        )
        .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER, "{}", done.body);
    assert_eq!(done.location(), Some("/account"));
    done.cookie().expect("the signup logs in")
}

#[tokio::test]
async fn a_start_sends_the_browser_to_github_with_pkce_the_email_scope_and_the_picker() {
    let (app, _) = github_app().await;
    let reply = app.form("/login/github", &[], None).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    let to = url::Url::parse(reply.location().unwrap()).unwrap();
    assert_eq!(format!("{}://{}", to.scheme(), to.host_str().unwrap()), WEB);
    assert_eq!(to.path(), "/login/oauth/authorize");
    let query: std::collections::HashMap<_, _> = to.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], CLIENT_ID);
    assert_eq!(
        query["redirect_uri"],
        format!("{}/login/github/callback", common::PUBLIC_URL)
    );
    assert_eq!(query["scope"], "user:email");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["prompt"], "select_account");
    assert!(!query["code_challenge"].is_empty());
    let cookie = reply.headers.get("set-cookie").unwrap().to_str().unwrap();
    // `__Host-` with Secure and Path=/, so no subdomain can plant one; that path
    // also sends it to the next start, and a second tab's start keeps the first
    // tab's sign-in.
    assert!(cookie.starts_with("__Host-fd_github="), "{cookie}");
    assert!(
        cookie.contains("Path=/;") && cookie.contains("Secure"),
        "{cookie}"
    );
    assert!(!cookie.contains("Domain"), "{cookie}");
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"),
        "{cookie}"
    );
    // START_MINUTES' 10 minutes, as long as the start it points to lasts.
    assert_eq!(reply.cookie_attribute("Max-Age").as_deref(), Some("600"));
}

#[tokio::test]
async fn the_code_is_exchanged_with_the_verifier_whose_challenge_the_start_sent() {
    let (app, github) = github_app().await;
    let reply = app.form("/login/github", &[], None).await;
    let to = url::Url::parse(reply.location().unwrap()).unwrap();
    let challenge = to
        .query_pairs()
        .find(|(key, _)| key == "code_challenge")
        .unwrap()
        .1
        .into_owned();
    let state = state_in(reply.location().unwrap());
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    come_back(&app, &state, "c1", &reply.cookie().unwrap()).await;
    let (code, verifier, redirect) = github.exchanges.lock().unwrap()[0].clone();
    assert_eq!(code, "c1");
    assert_eq!(
        filedrawer_server::github_login::challenge(&verifier),
        challenge
    );
    assert_eq!(
        redirect,
        format!("{}/login/github/callback", common::PUBLIC_URL)
    );
}

#[test]
fn the_challenge_is_the_verifiers_sha_256_in_base64url_without_padding() {
    // Computed apart, with Python's hashlib and base64.urlsafe_b64encode, the
    // padding stripped.
    assert_eq!(
        filedrawer_server::github_login::challenge("dBjftJeZ4CVP-mJ92K9GzIp6AQDCn8tXyGhS1nGB2CM"),
        "vCjHowWYyUOHJlYZYz14crd0Ow-OQULnjGQ_ezZjjEg"
    );
}

#[tokio::test]
async fn without_an_oauth_app_there_is_no_github_sign_in() {
    let app = test_app().await;
    assert_eq!(
        app.form("/login/github", &[], None).await.status,
        StatusCode::NOT_FOUND
    );
    let (session, _) = browser_account(&app, "grace@example.com").await;
    let unlink = app
        .form("/account/github/unlink", &[], Some(&session))
        .await;
    assert_eq!(unlink.status, StatusCode::NOT_FOUND);
    let login = app.page("/login", None).await;
    assert!(login.has_class("github-off"), "{}", login.body);
    assert!(!login.has_class("github-sign-in"), "{}", login.body);
    let (app, _) = github_app().await;
    let login = app.page("/login", None).await;
    assert!(login.has_class("github-sign-in"), "{}", login.body);
}

#[tokio::test]
async fn a_get_cant_start_a_sign_in() {
    let (app, _) = github_app().await;
    let reply = app.page("/login/github", None).await;
    assert_eq!(
        reply.status,
        StatusCode::METHOD_NOT_ALLOWED,
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn a_new_github_user_signs_up_verified_with_no_password_and_no_email() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let session = github_signup(&app, "c1").await;
    let account = app.page("/account", Some(&session)).await;
    assert!(account.has_class("github-linked"), "{}", account.body);
    assert!(account.body.contains("github:octo"), "{}", account.body);
    // Verified, so it may make a token at once.
    common::new_token(&app, &session).await;
    assert!(app.sent().await.is_empty());
    let id = common::account_id(&app, &session).await;
    let password: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM accounts WHERE id = ?")
            .bind(&id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(password, None);
}

#[tokio::test]
async fn the_signup_form_offers_githubs_addresses_and_keeps_the_one_chosen() {
    let (app, github) = github_app().await;
    let mut identity = octo(1, "octo", "first@example.com");
    identity.emails.push("second@example.com".into());
    github.code_for("c1", identity);
    let (state, browser) = start(&app, "/login/github", None).await;
    let back = come_back(&app, &state, "c1", &browser).await;
    let pending = back.cookie().unwrap();
    // The waiting signup's cookie lasts START_MINUTES' 10 minutes, as the signup does.
    assert_eq!(back.cookie_attribute("Max-Age").as_deref(), Some("600"));
    let form = app.page("/signup/github", Some(&pending)).await;
    assert!(form.body.contains("first@example.com") && form.body.contains("second@example.com"));
    // GitHub's name is the suggestion.
    assert!(form.body.contains("value=\"Octo Cat\""), "{}", form.body);
    let done = app
        .form(
            "/signup/github",
            &[("display_name", "Octo"), ("email", "1")],
            Some(&pending),
        )
        .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER, "{}", done.body);
    // The first cookie is the session's, which lasts as long as the session.
    let max_age = done.cookie_attribute("Max-Age");
    assert_eq!(max_age.as_deref(), Some(common::SESSION_MAX_AGE));
    let email: String = sqlx::query_scalar(
        "SELECT a.email FROM accounts a JOIN github_links l ON l.account_id = a.id",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(email, "second@example.com");
    // The waiting signup is spent.
    let again = app.page("/signup/github", Some(&pending)).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_long_github_name_is_suggested_cut_to_50_characters() {
    let (app, github) = github_app().await;
    let mut identity = octo(1, "octo", "octo@example.com");
    identity.name = Some("N".repeat(60));
    github.code_for("c1", identity);
    let (state, browser) = start(&app, "/login/github", None).await;
    let pending = come_back(&app, &state, "c1", &browser)
        .await
        .cookie()
        .unwrap();
    let form = app.page("/signup/github", Some(&pending)).await;
    let expected = format!("value=\"{}\"", "N".repeat(50));
    assert!(form.body.contains(&expected), "{}", form.body);
}

#[tokio::test]
async fn a_linked_github_user_signs_in_and_a_rename_shows() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let first = github_signup(&app, "c1").await;
    let id = common::account_id(&app, &first).await;
    github.code_for("c2", octo(1, "octo-renamed", "octo@example.com"));
    let (state, browser) = start(&app, "/login/github", None).await;
    let back = come_back(&app, &state, "c2", &browser).await;
    assert_eq!(back.status, StatusCode::SEE_OTHER, "{}", back.body);
    assert_eq!(back.location(), Some("/account"));
    let max_age = back.cookie_attribute("Max-Age");
    assert_eq!(max_age.as_deref(), Some(common::SESSION_MAX_AGE));
    let session = back.cookie().unwrap();
    assert_eq!(common::account_id(&app, &session).await, id);
    let page = app.page("/account", Some(&session)).await;
    assert!(page.body.contains("github:octo-renamed"), "{}", page.body);
}

#[tokio::test]
async fn a_callback_works_once_and_only_in_the_browser_that_started_it() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let (state, browser) = start(&app, "/login/github", None).await;
    let (_, other_browser) = start(&app, "/login/github", None).await;
    for cookie in [other_browser.as_str(), "__Host-fd_github=someone-else"] {
        let elsewhere = come_back(&app, &state, "c1", cookie).await;
        assert_eq!(
            elsewhere.status,
            StatusCode::BAD_REQUEST,
            "{cookie}: {}",
            elsewhere.body
        );
    }
    let no_cookie = app
        .page(
            &format!("/login/github/callback?code=c1&state={state}"),
            None,
        )
        .await;
    assert_eq!(no_cookie.status, StatusCode::BAD_REQUEST);
    // Neither spent the start, so its own browser still finishes it.
    let back = come_back(&app, &state, "c1", &browser).await;
    assert_eq!(back.status, StatusCode::SEE_OTHER, "{}", back.body);
    // The server keeps no start to spend: a copy of the cookie gets the start
    // again, and GitHub refuses its spent code.
    let again = come_back(&app, &state, "c1", &browser).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(github.exchanges.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_callback_with_two_start_cookies_finishes_nothing() {
    // A browser that lets a subdomain set a cookie with no name could send one
    // ahead of the real one: someone's own start, finished with their code, would
    // sign the browser in to their account.
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let (_, browser) = start(&app, "/login/github", None).await;
    let (theirs, planted) = start(&app, "/login/github", None).await;
    let back = come_back(&app, &theirs, "c1", &format!("{planted}; {browser}")).await;
    assert_eq!(back.status, StatusCode::BAD_REQUEST, "{}", back.body);
    assert!(github.exchanges.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_start_with_a_garbled_cookie_starts_afresh() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let (state, browser) =
        start(&app, "/login/github", Some("__Host-fd_github=%%not-base64")).await;
    let back = come_back(&app, &state, "c1", &browser).await;
    assert_eq!(back.status, StatusCode::SEE_OTHER, "{}", back.body);
}

#[tokio::test]
async fn two_tabs_each_finish_their_own_sign_in() {
    let (app, github) = github_app().await;
    let (first, browser) = start(&app, "/login/github", None).await;
    let (second, both) = start(&app, "/login/github", Some(&browser)).await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    github.code_for("c2", octo(2, "cat", "cat@example.com"));
    assert_eq!(
        come_back(&app, &first, "c1", &both).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        come_back(&app, &second, "c2", &both).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn a_fourth_start_drops_the_oldest() {
    let (app, github) = github_app().await;
    let (oldest, mut cookie) = start(&app, "/login/github", None).await;
    let mut states = vec![];
    for _ in 1..=filedrawer_server::github_accounts::STARTS_KEPT {
        let (state, kept) = start(&app, "/login/github", Some(&cookie)).await;
        states.push(state);
        cookie = kept;
    }
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let dropped = come_back(&app, &oldest, "c1", &cookie).await;
    assert_eq!(dropped.status, StatusCode::BAD_REQUEST, "{}", dropped.body);
    for (n, state) in states.iter().enumerate() {
        let code = format!("k{n}");
        github.code_for(&code, octo(1, "octo", "octo@example.com"));
        let back = come_back(&app, state, &code, &cookie).await;
        assert_eq!(back.status, StatusCode::SEE_OTHER, "{n}: {}", back.body);
    }
}

#[tokio::test]
async fn a_next_page_edited_in_the_cookie_is_checked_again() {
    // The cookie is the browser's to edit, so the callback doesn't trust its page.
    use filedrawer_server::github_accounts::{Purpose, add_start};
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    github_signup(&app, "c1").await;
    let (started, kept) = add_start(
        None,
        Purpose::SignIn,
        Some("//evil.example"),
        time::OffsetDateTime::now_utc(),
    );
    github.code_for("again", octo(1, "octo", "octo@example.com"));
    let cookie = format!("__Host-fd_github={kept}");
    let back = come_back(&app, &started.state, "again", &cookie).await;
    assert_eq!(back.location(), Some("/account"), "{}", back.body);
}

#[test]
fn a_start_that_cant_fit_drops_the_oldest_starts() {
    use filedrawer_server::github_accounts::{Purpose, START_COOKIE_BYTES, add_start, find_start};
    let at = time::macros::datetime!(2026-11-23 12:00 UTC);
    // A start whose page fills nearly the whole cookie by itself.
    let (big, cookie) = (1..)
        .map(|n| {
            add_start(
                None,
                Purpose::SignIn,
                Some(&format!("/{}", "a".repeat(n))),
                at,
            )
        })
        .find(|(_, cookie)| cookie.len() > START_COOKIE_BYTES - 40)
        .unwrap();
    assert!(cookie.len() <= START_COOKIE_BYTES);
    let (newest, both) = add_start(Some(&cookie), Purpose::SignIn, None, at);
    assert!(both.len() <= START_COOKIE_BYTES, "{}", both.len());
    assert!(find_start(&both, &big.state, at).is_none());
    assert!(find_start(&both, &newest.state, at).is_some());
}

#[test]
fn a_start_expires_after_ten_minutes() {
    use filedrawer_server::github_accounts::{Purpose, START_MINUTES, add_start, find_start};
    let at = time::macros::datetime!(2026-11-23 12:00 UTC);
    let (started, cookie) = add_start(None, Purpose::SignIn, None, at);
    let just_before = at + time::Duration::minutes(START_MINUTES) - time::Duration::seconds(1);
    assert!(find_start(&cookie, &started.state, just_before).is_some());
    let at_expiry = at + time::Duration::minutes(START_MINUTES);
    assert!(find_start(&cookie, &started.state, at_expiry).is_none());
    // A later start drops it from the cookie.
    let (_, later) = add_start(Some(&cookie), Purpose::SignIn, None, at_expiry);
    assert!(find_start(&later, &started.state, at).is_none());
}

#[tokio::test]
async fn githubs_failures_get_pages_that_say_what_to_do() {
    let (app, github) = github_app().await;
    let cancelled = app
        .page("/login/github/callback?error=access_denied", None)
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{}", cancelled.body);
    // Any other error is the app's setup, not the person's cancel.
    let broken = app
        .page("/login/github/callback?error=redirect_uri_mismatch", None)
        .await;
    assert_eq!(broken.status, StatusCode::BAD_GATEWAY, "{}", broken.body);
    assert!(
        broken.body.contains("redirect_uri_mismatch"),
        "{}",
        broken.body
    );
    let crafted = app.page("/login/github/callback?error=%3Cb%3E", None).await;
    assert!(!crafted.body.contains("<b>"), "{}", crafted.body);
    github.code_fails("c1", Failure::NoEmailAccess);
    let (state, browser) = start(&app, "/login/github", None).await;
    let no_scope = come_back(&app, &state, "c1", &browser).await;
    assert_eq!(no_scope.status, StatusCode::FORBIDDEN, "{}", no_scope.body);
    github.code_fails("c2", Failure::Unavailable("timed out".into()));
    let (state, browser) = start(&app, "/login/github", None).await;
    let down = come_back(&app, &state, "c2", &browser).await;
    assert_eq!(down.status, StatusCode::BAD_GATEWAY, "{}", down.body);
    assert!(down.body.contains("/reset"), "{}", down.body);
    let mut none_usable = octo(3, "quiet", "x@example.com");
    none_usable.emails.clear();
    github.code_for("c3", none_usable);
    let (state, browser) = start(&app, "/login/github", None).await;
    let no_address = come_back(&app, &state, "c3", &browser).await;
    assert_eq!(
        no_address.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        no_address.body
    );
}

#[tokio::test]
async fn an_address_with_an_account_gets_the_taken_email_and_nothing_is_made() {
    let (app, github) = github_app().await;
    browser_account(&app, "grace@example.com").await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    github.code_for("c1", octo(1, "octo", "grace@example.com"));
    let (state, browser) = start(&app, "/login/github", None).await;
    let pending = come_back(&app, &state, "c1", &browser)
        .await
        .cookie()
        .unwrap();
    let done = app
        .form(
            "/signup/github",
            &[("display_name", "Octo"), ("email", "0")],
            Some(&pending),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert!(
        done.headers
            .get_all("set-cookie")
            .iter()
            .all(|cookie| !cookie.to_str().unwrap().starts_with("__Host-fd_session="))
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(after, before);
    let last = app.sent().await.pop().unwrap();
    assert_eq!(last.to, "grace@example.com");
    assert!(last.body.contains("link GitHub"), "{}", last.body);
}

#[tokio::test]
async fn linking_from_the_account_page_verifies_a_matching_address() {
    let (app, github) = github_app().await;
    // An unverified email account.
    let signup = app
        .form(
            "/signup",
            &[
                ("email", "grace@example.com"),
                ("password", PASSWORD),
                ("display_name", "Grace"),
            ],
            None,
        )
        .await;
    assert_eq!(signup.status, StatusCode::OK);
    let session = logged_in(&app, "grace@example.com", PASSWORD).await;
    github.code_for("c1", octo(1, "grace-gh", "grace@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&session)).await;
    let cookies = format!("{session}; {browser}");
    let back = come_back(&app, &state, "c1", &cookies).await;
    assert_eq!(back.location(), Some("/account"), "{}", back.body);
    let page = app.page("/account", Some(&session)).await;
    assert!(page.body.contains("github:grace-gh"), "{}", page.body);
    common::new_token(&app, &session).await;
}

#[tokio::test]
async fn a_link_finished_under_another_accounts_session_links_nothing() {
    let (app, github) = github_app().await;
    let (first, _) = browser_account(&app, "grace@example.com").await;
    let (second, _) = browser_account(&app, "hopper@example.com").await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&first)).await;
    let back = come_back(&app, &state, "c1", &format!("{second}; {browser}")).await;
    assert_eq!(back.status, StatusCode::FORBIDDEN, "{}", back.body);
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM github_links")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(links, 0);
}

#[tokio::test]
async fn an_account_links_one_github_account() {
    let (app, github) = github_app().await;
    let (session, _) = browser_account(&app, "grace@example.com").await;
    for (code, id) in [("c1", 1), ("c2", 2)] {
        github.code_for(code, octo(id, &format!("gh{id}"), "grace@example.com"));
    }
    let (state, browser) = start(&app, "/account/github/link", Some(&session)).await;
    come_back(&app, &state, "c1", &format!("{session}; {browser}")).await;
    let (state, browser) = start(&app, "/account/github/link", Some(&session)).await;
    let second = come_back(&app, &state, "c2", &format!("{session}; {browser}")).await;
    assert_eq!(second.status, StatusCode::CONFLICT, "{}", second.body);
}

#[tokio::test]
async fn a_wrong_invite_code_counts_as_a_guess_against_the_ips_signups() {
    let github = FakeGithub::default();
    let app = one_signup_an_hour(test_app().await.with_github(&github));
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let (state, browser) = start(&app, "/login/github", None).await;
    let pending = come_back(&app, &state, "c1", &browser)
        .await
        .cookie()
        .unwrap();
    let wrong = app
        .form(
            "/signup/github",
            &[
                ("display_name", "Octo"),
                ("email", "0"),
                ("invite", "inv-nope"),
            ],
            Some(&pending),
        )
        .await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.body
    );
    let next = app
        .form(
            "/signup",
            &[
                ("email", "grace@example.com"),
                ("password", PASSWORD),
                ("display_name", "Grace"),
            ],
            None,
        )
        .await;
    assert_eq!(next.status, StatusCode::TOO_MANY_REQUESTS, "{}", next.body);
}

#[tokio::test]
async fn each_finish_counts_against_its_addresss_emails_taken_or_not() {
    let github = FakeGithub::default();
    let app = test_app().await.with_github(&github).with_limits(Limits {
        emails_per_address: vec![Window {
            seconds: 3600,
            count: 1,
        }],
        ..Limits::site()
    });
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    github_signup(&app, "c1").await;
    // A second GitHub account with the same address finds it taken, past its limit.
    github.code_for("c2", octo(2, "other", "octo@example.com"));
    let (state, browser) = start(&app, "/login/github", None).await;
    let pending = come_back(&app, &state, "c2", &browser)
        .await
        .cookie()
        .unwrap();
    let refused = app
        .form(
            "/signup/github",
            &[("display_name", "Other"), ("email", "0")],
            Some(&pending),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.body
    );
    assert!(app.sent().await.is_empty());
}

#[tokio::test]
async fn a_link_needs_the_session_that_started_it_and_an_unlinked_github_account() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    github_signup(&app, "c1").await;
    let (session, _) = browser_account(&app, "grace@example.com").await;
    // The GitHub account belongs to the first account.
    github.code_for("c2", octo(1, "octo", "octo@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&session)).await;
    let taken = come_back(&app, &state, "c2", &format!("{session}; {browser}")).await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.body);
    // A link whose session ended meanwhile links nothing.
    github.code_for("c3", octo(9, "nine", "nine@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&session)).await;
    let no_session = come_back(&app, &state, "c3", &browser).await;
    assert_eq!(
        no_session.status,
        StatusCode::FORBIDDEN,
        "{}",
        no_session.body
    );
}

#[tokio::test]
async fn unlinking_needs_a_password_and_takes_the_handle_off() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let github_only = github_signup(&app, "c1").await;
    // The account page says what choosing a password costs.
    let page = app.page("/account", Some(&github_only)).await;
    assert!(page.has_class("github-needs-password"), "{}", page.body);
    assert!(
        page.body.contains("revokes every API token"),
        "{}",
        page.body
    );
    let refused = app
        .form("/account/github/unlink", &[], Some(&github_only))
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    let (session, _) = browser_account(&app, "grace@example.com").await;
    github.code_for("c2", octo(2, "grace-gh", "grace@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&session)).await;
    come_back(&app, &state, "c2", &format!("{session}; {browser}")).await;
    let page = app.page("/account", Some(&session)).await;
    assert!(
        page.body
            .contains(&format!("settings/connections/applications/{CLIENT_ID}"))
    );
    let done = app
        .form("/account/github/unlink", &[], Some(&session))
        .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER, "{}", done.body);
    let page = app.page("/account", Some(&session)).await;
    assert!(!page.has_class("github-linked"), "{}", page.body);
}

#[tokio::test]
async fn the_card_and_the_author_line_show_the_handle_before_the_name() {
    let app = test_app().await;
    // Confirmed by GitHub just now, so the card shows it.
    let now = common::stamp(time::OffsetDateTime::now_utc());
    sqlx::query(
        "INSERT INTO github_links (account_id, github_id, login, linked_at, refreshed_at)
         VALUES (?, 1, 'ada-gh', ?, ?)",
    )
    .bind(&app.account)
    .bind(&now)
    .bind(&now)
    .execute(&app.pool)
    .await
    .unwrap();
    let id = app.submit(&common::entry("Steering", "")).await;
    let page = app.get(&format!("/e/{id}")).await;
    let expected = format!("account {}, github:ada-gh, Ada", app.account);
    assert_eq!(
        field(&page.body, "author"),
        Some(expected.as_str()),
        "{}",
        page.body
    );
    let card = app.get("/search?q=Steering").await;
    assert!(
        card.body.contains(&format!("author: {expected}")),
        "{}",
        card.body
    );
    // A handle GitHub hides shows nowhere.
    sqlx::query("UPDATE github_links SET hidden = 1")
        .execute(&app.pool)
        .await
        .unwrap();
    let page = app.get(&format!("/e/{id}")).await;
    let plain = format!("account {}, Ada", app.account);
    assert_eq!(field(&page.body, "author"), Some(plain.as_str()));
}

#[tokio::test]
async fn an_account_with_no_password_confirms_deletion_with_its_github_login() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    let session = github_signup(&app, "c1").await;
    let form = app.page("/account/delete", Some(&session)).await;
    assert!(form.body.contains("name=\"login\""), "{}", form.body);
    let wrong = app
        .form("/account/delete", &[("login", "someone")], Some(&session))
        .await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.body
    );
    let done = app
        .form("/account/delete", &[("login", "Octo")], Some(&session))
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM github_links")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(links, 0);
    // The bare GitHub ID stays, with no account, so it exempts no second account.
    let kept: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT github_id, account_id FROM github_exemptions")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(kept, vec![(1, None)]);
}

/// Links the app's own account to a GitHub account made at `created_at`, with an
/// exemption record naming `exempted`.
async fn link_app_account(app: &TestApp, created_at: &str, exempted: &str) {
    sqlx::query(
        "INSERT INTO github_links
           (account_id, github_id, login, github_created_at, linked_at, refreshed_at)
         VALUES (?, 7, 'ada-gh', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(&app.account)
    .bind(created_at)
    .execute(&app.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO github_exemptions (github_id, account_id) VALUES (7, ?)")
        .bind(exempted)
        .execute(&app.pool)
        .await
        .unwrap();
}

/// Whether the app's account may have 21 entries screened today.
async fn gets_50_today(app: &TestApp) -> bool {
    let entries: Vec<String> = (0..21)
        .map(|n| common::entry(&format!("Steering {n}"), &format!("slug: s-{n}\n")))
        .collect();
    let reply = app
        .post(
            "/entries/batch",
            serde_json::json!({ "entries": entries }).to_string(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    field(&reply.body, "screen_from").is_none()
}

#[tokio::test]
async fn the_first_account_an_old_github_account_links_skips_the_new_account_limit() {
    let app = test_app().await;
    link_app_account(&app, "2015-01-01T00:00:00Z", &app.account.clone()).await;
    assert!(gets_50_today(&app).await);
}

#[tokio::test]
async fn a_young_github_account_or_a_second_account_gets_no_exemption() {
    let young = test_app().await;
    let recent = common::stamp(time::OffsetDateTime::now_utc() - time::Duration::days(30));
    link_app_account(&young, &recent, &young.account.clone()).await;
    assert!(!gets_50_today(&young).await);
    let second = test_app().await;
    let other = filedrawer_server::admin::create_account(&second.pool, "Other")
        .await
        .unwrap();
    link_app_account(&second, "2015-01-01T00:00:00Z", &other).await;
    assert!(!gets_50_today(&second).await);
}

#[tokio::test]
async fn the_exemption_stays_with_the_first_account_to_link() {
    let (app, github) = github_app().await;
    let (first, _) = browser_account(&app, "grace@example.com").await;
    let first_id = common::account_id(&app, &first).await;
    github.code_for("c1", octo(1, "octo", "grace@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&first)).await;
    come_back(&app, &state, "c1", &format!("{first}; {browser}")).await;
    app.form("/account/github/unlink", &[], Some(&first)).await;
    let (second, _) = browser_account(&app, "hopper@example.com").await;
    github.code_for("c2", octo(1, "octo", "grace@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&second)).await;
    let back = come_back(&app, &state, "c2", &format!("{second}; {browser}")).await;
    assert_eq!(back.location(), Some("/account"), "{}", back.body);
    let exempted: Option<String> =
        sqlx::query_scalar("SELECT account_id FROM github_exemptions WHERE github_id = 1")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(exempted, Some(first_id));
}

/// The app with one signup an hour from an IP.
fn one_signup_an_hour(app: TestApp) -> TestApp {
    app.with_limits(Limits {
        signups_per_ip: vec![Window {
            seconds: 3600,
            count: 1,
        }],
        ..Limits::site()
    })
}

#[tokio::test]
async fn only_young_github_accounts_count_against_the_ips_signups() {
    let github = FakeGithub::default();
    let app = one_signup_an_hour(test_app().await.with_github(&github));
    // Old GitHub accounts sign up past the IP's limit.
    for n in 1..=3 {
        github.code_for(
            &format!("c{n}"),
            octo(n, &format!("old{n}"), &format!("old{n}@example.com")),
        );
        github_signup(&app, &format!("c{n}")).await;
    }
    let mut young = octo(10, "young", "young@example.com");
    young.created_at = Some(common::stamp(time::OffsetDateTime::now_utc()));
    github.code_for("y1", young.clone());
    github_signup(&app, "y1").await;
    let mut unknown = octo(11, "unknown", "unknown@example.com");
    unknown.created_at = None;
    github.code_for("y2", unknown);
    let (state, browser) = start(&app, "/login/github", None).await;
    let pending = come_back(&app, &state, "y2", &browser)
        .await
        .cookie()
        .unwrap();
    let refused = app
        .form(
            "/signup/github",
            &[("display_name", "Unknown"), ("email", "0")],
            Some(&pending),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.body
    );
}

#[tokio::test]
async fn a_display_name_cant_pose_as_a_handle() {
    let app = test_app().await;
    for name in ["github:torvalds", "Ada (ORCID:0000)"] {
        let reply = app
            .form(
                "/signup",
                &[
                    ("email", "ada@example.com"),
                    ("password", PASSWORD),
                    ("display_name", name),
                ],
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{name}");
        assert!(reply.has_class("error"), "{}", reply.body);
    }
}

#[tokio::test]
async fn starts_from_one_client_are_never_refused() {
    // The server keeps nothing for a start, so there is no table to fill.
    let (app, _) = github_app().await;
    for _ in 0..50 {
        start(&app, "/login/github", None).await;
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM github_starts")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_long_next_page_is_dropped_so_the_cookie_fits() {
    // `"` doubles in the cookie's JSON, so the cap is on the cookie, not on `next`.
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    github_signup(&app, "c1").await;
    let long = format!("/{}", "\"".repeat(3000));
    let mut cookie: Option<String> = None;
    let mut state = String::new();
    for _ in 0..filedrawer_server::github_accounts::STARTS_KEPT {
        let reply = app
            .form("/login/github", &[("next", &long)], cookie.as_deref())
            .await;
        state = state_in(reply.location().unwrap());
        let set = reply.cookie().unwrap();
        let value = set.strip_prefix("__Host-fd_github=").unwrap();
        assert!(
            value.len() <= filedrawer_server::github_accounts::START_COOKIE_BYTES,
            "{}",
            value.len()
        );
        cookie = Some(set);
    }
    github.code_for("again", octo(1, "octo", "octo@example.com"));
    let back = come_back(&app, &state, "again", &cookie.unwrap()).await;
    assert_eq!(back.location(), Some("/account"), "{}", back.body);
}

#[tokio::test]
async fn a_sign_in_lands_on_the_page_it_was_asked_from_on_this_site_only() {
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    github_signup(&app, "c1").await;
    for (next, landing) in [("/admin", "/admin"), ("//evil.example", "/account")] {
        let reply = app.form("/login/github", &[("next", next)], None).await;
        let state = state_in(reply.location().unwrap());
        github.code_for("again", octo(1, "octo", "octo@example.com"));
        let back = come_back(&app, &state, "again", &reply.cookie().unwrap()).await;
        assert_eq!(back.location(), Some(landing), "{next}");
    }
}

#[tokio::test]
async fn a_password_reset_unlinks_github() {
    // Whoever reads the inbox now may not be who linked GitHub: GitHub can list an
    // address its user no longer holds, so a reset takes the handle off the cards.
    let (app, github) = github_app().await;
    github.code_for("c1", octo(1, "octo", "octo@example.com"));
    github_signup(&app, "c1").await;
    let reset = app
        .form("/reset", &[("email", "octo@example.com")], None)
        .await;
    assert_eq!(reset.status, StatusCode::OK, "{}", reset.body);
    let link = app.link_in_last_email_to("octo@example.com").await;
    let token = link.strip_prefix("/reset/confirm?token=").expect(&link);
    let done = app
        .form(
            "/reset/confirm",
            &[("token", token), ("password", PASSWORD)],
            None,
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let session = logged_in(&app, "octo@example.com", PASSWORD).await;
    let page = app.page("/account", Some(&session)).await;
    assert!(!page.has_class("github-linked"), "{}", page.body);
    assert!(!page.body.contains("github:octo"), "{}", page.body);
    // The exemption record stays, as unlinking leaves it.
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM github_exemptions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(kept, 1);
}

#[tokio::test]
async fn linking_an_old_github_account_moves_waiting_entries_up() {
    let (app, github) = github_app().await;
    let (session, token) = browser_account(&app, "grace@example.com").await;
    let mut last = String::new();
    for n in 0..21 {
        let text = common::entry(&format!("Steering {n}"), "");
        let reply = app.post_as("/entries", Some(&token), text).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        last = common::id_in(&reply.body);
    }
    // A new account's 21st entry waits for tomorrow's units.
    let path = format!("/e/{last}");
    let waiting = app.get_with_token(&path, &token).await;
    let tomorrow = common::date_in(1);
    assert_eq!(field(&waiting.body, "screen_from"), Some(tomorrow.as_str()));
    github.code_for("c1", octo(1, "grace-gh", "grace@example.com"));
    let (state, browser) = start(&app, "/account/github/link", Some(&session)).await;
    let back = come_back(&app, &state, "c1", &format!("{session}; {browser}")).await;
    assert_eq!(back.location(), Some("/account"), "{}", back.body);
    // The link lifts the new-account limit, so today has room.
    let moved = app.get_with_token(&path, &token).await;
    assert_eq!(field(&moved.body, "screen_from"), None, "{}", moved.body);
}
