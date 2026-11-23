//! Seam: the router, with the fake mailer's outbox, for changing an account's email
//! address (design.md, Accounts; #125): the request from the account page, the two
//! emails, the confirm page and the swap. Where no page or email shows a row, a test
//! reads `email_changes` itself.

use crate::common;

use axum::http::StatusCode;
use common::fake_github::{FakeGithub, octo};
use common::{
    PASSWORD, Reply, TestApp, browser_account, entry, logged_in, signed_up_and_verified, test_app,
};
use filedrawer_server::mail::{Email, MailError};
use time::{Duration, OffsetDateTime};

const OLD: &str = "grace@example.com";
const NEW: &str = "grace@new.example";
const CONFIRM: &str = "/account/email/confirm";

/// Asks, from the account page's form, to change the session's address to `to`.
async fn ask(app: &TestApp, cookie: &str, to: &str, password: &str) -> Reply {
    app.form(
        "/account/email",
        &[("email", to), ("password", password)],
        Some(cookie),
    )
    .await
}

/// The token of the confirm link in the last email to `to`.
async fn emailed_token(app: &TestApp, to: &str) -> String {
    let link = app.link_in_last_email_to(to).await;
    link.strip_prefix(&format!("{CONFIRM}?token="))
        .unwrap_or_else(|| panic!("not a confirm link: {link}"))
        .to_string()
}

/// Presses the confirm page's button for `token`.
async fn confirm(app: &TestApp, cookie: &str, token: &str) -> Reply {
    app.form(CONFIRM, &[("token", token)], Some(cookie)).await
}

/// Whether the account page says a change is waiting.
async fn waiting(app: &TestApp, cookie: &str) -> bool {
    let page = app.page("/account", Some(cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    page.has_class("email-change-pending")
}

async fn emails_to(app: &TestApp, to: &str) -> Vec<Email> {
    let sent = app.sent().await;
    sent.into_iter().filter(|email| email.to == to).collect()
}

async fn pending_rows(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM email_changes")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// Grace, verified at `OLD` and logged in: her session cookie and a token.
async fn grace(app: &TestApp) -> (String, String) {
    browser_account(app, OLD).await
}

#[tokio::test]
async fn a_change_confirmed_from_the_new_inbox_swaps_the_address_and_keeps_the_rest() {
    let app = test_app().await;
    let (cookie, token) = grace(&app).await;
    let asked = ask(&app, &cookie, " Grace@New.Example ", PASSWORD).await;
    assert_eq!(asked.status, StatusCode::SEE_OTHER, "{}", asked.body);
    assert_eq!(asked.location(), Some("/account"));
    let page = app.page("/account", Some(&cookie)).await;
    assert!(page.has_class("email-change-pending"), "{}", page.body);
    assert!(page.body.contains(NEW), "{}", page.body);
    assert!(page.body.contains(OLD), "{}", page.body);

    let secret = emailed_token(&app, NEW).await;
    let opened = app
        .page(&format!("{CONFIRM}?token={secret}"), Some(&cookie))
        .await;
    assert_eq!(opened.status, StatusCode::OK, "{}", opened.body);
    assert!(
        opened.body.contains(r#"action="/account/email/confirm""#),
        "{}",
        opened.body
    );
    // Opening the page changes nothing: mail scanners follow links.
    assert!(waiting(&app, &cookie).await);

    let done = confirm(&app, &cookie, &secret).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let page = app.page("/account", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "the session stays");
    assert!(page.body.contains(NEW), "{}", page.body);
    assert!(!page.body.contains(OLD), "{}", page.body);
    assert!(!page.has_class("email-change-pending"), "{}", page.body);
    assert_eq!(pending_rows(&app).await, 0);

    logged_in(&app, NEW, PASSWORD).await;
    let old_login = app
        .form("/login", &[("email", OLD), ("password", PASSWORD)], None)
        .await;
    assert_eq!(old_login.status, StatusCode::UNAUTHORIZED);
    let submitted = app.post_as("/entries", Some(&token), entry("T", "")).await;
    assert_eq!(submitted.status, StatusCode::CREATED, "the token stays");

    let again = confirm(&app, &cookie, &secret).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST, "{}", again.body);
}

#[tokio::test]
async fn the_old_address_gets_a_notice_that_names_no_address_and_the_new_one_the_link() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let to_new = emails_to(&app, NEW).await;
    assert_eq!(to_new.len(), 1, "{to_new:?}");
    let expires: String = sqlx::query_scalar("SELECT expires_at FROM email_changes")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(to_new[0].body.contains(&expires), "{}", to_new[0].body);
    let to_old: Vec<Email> = emails_to(&app, OLD)
        .await
        .into_iter()
        .filter(|email| !email.body.contains("/verify?token="))
        .collect();
    assert_eq!(to_old.len(), 1, "{to_old:?}");
    let notice = &to_old[0].body;
    assert!(!notice.contains(NEW), "{notice}");
    assert!(!notice.contains(CONFIRM), "{notice}");
    assert!(notice.contains("/reset"), "{notice}");
    assert!(notice.contains("contact@filedrawer.dev"), "{notice}");
}

#[tokio::test]
async fn a_wrong_password_changes_nothing_and_ten_an_hour_lock_only_the_form() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    for _ in 0..10 {
        let refused = ask(&app, &cookie, NEW, "not the password").await;
        assert_eq!(refused.status, StatusCode::UNAUTHORIZED, "{}", refused.body);
    }
    assert_eq!(pending_rows(&app).await, 0);
    assert!(emails_to(&app, NEW).await.is_empty());
    let locked = ask(&app, &cookie, NEW, PASSWORD).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    // The form's guesses are counted apart from the address's failed logins.
    logged_in(&app, OLD, PASSWORD).await;
}

#[tokio::test]
async fn a_right_password_gives_its_place_back() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    for round in 0..12 {
        let to = format!("grace{round}@new.example");
        let asked = ask(&app, &cookie, &to, PASSWORD).await;
        assert_eq!(
            asked.status,
            StatusCode::SEE_OTHER,
            "{round}: {}",
            asked.body
        );
    }
}

#[tokio::test]
async fn the_form_refuses_what_is_no_address_and_the_address_the_account_has() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    for bad in [
        "grace",
        "Grace <grace@new.example>",
        OLD,
        " GRACE@example.com",
    ] {
        let refused = ask(&app, &cookie, bad, PASSWORD).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad}: {}",
            refused.body
        );
    }
    assert_eq!(pending_rows(&app).await, 0);
}

#[tokio::test]
async fn the_change_routes_take_a_session_and_never_a_token() {
    let app = test_app().await;
    let (cookie, token) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    let bearer = format!("Bearer {token}");
    for (path, body) in [
        (
            "/account/email",
            format!("email=x@new.example&password={PASSWORD}"),
        ),
        ("/account/email/cancel", String::new()),
        (CONFIRM, format!("token={secret}")),
    ] {
        let request = axum::http::Request::post(path)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("origin", common::PUBLIC_URL)
            .header("authorization", &bearer)
            .body(axum::body::Body::from(body))
            .unwrap();
        let reply = app.send(request).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{path}");
        assert_eq!(reply.location(), Some("/login"), "{path}");
    }
    assert!(waiting(&app, &cookie).await);
}

#[tokio::test]
async fn the_link_alone_confirms_nothing_and_logging_in_comes_back_to_it() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    let link = format!("{CONFIRM}?token={secret}");
    let opened = app.page(&link, None).await;
    assert_eq!(opened.status, StatusCode::SEE_OTHER);
    let next: String = url::form_urlencoded::byte_serialize(link.as_bytes()).collect();
    assert_eq!(
        opened.location(),
        Some(format!("/login?next={next}").as_str())
    );
    let pressed = app.form(CONFIRM, &[("token", &secret)], None).await;
    assert_eq!(pressed.location(), Some("/login"));
    assert!(waiting(&app, &cookie).await);
}

#[tokio::test]
async fn another_accounts_session_cant_confirm_the_link() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    signed_up_and_verified(&app, "mallory@example.com").await;
    let other = logged_in(&app, "mallory@example.com", PASSWORD).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    let opened = app
        .page(&format!("{CONFIRM}?token={secret}"), Some(&other))
        .await;
    assert_eq!(opened.status, StatusCode::BAD_REQUEST, "{}", opened.body);
    let pressed = confirm(&app, &other, &secret).await;
    assert_eq!(pressed.status, StatusCode::BAD_REQUEST, "{}", pressed.body);
    // Her link still works for her.
    assert_eq!(confirm(&app, &cookie, &secret).await.status, StatusCode::OK);
}

#[tokio::test]
async fn an_address_with_an_account_answers_the_same_and_its_holder_gets_the_note() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    signed_up_and_verified(&app, NEW).await;
    let before = emails_to(&app, NEW).await.len();
    let asked = ask(&app, &cookie, NEW, PASSWORD).await;
    assert_eq!(asked.status, StatusCode::SEE_OTHER, "{}", asked.body);
    assert_eq!(asked.location(), Some("/account"));
    assert!(waiting(&app, &cookie).await);
    let to_new = emails_to(&app, NEW).await;
    assert_eq!(to_new.len(), before + 1, "{to_new:?}");
    let note = &to_new.last().unwrap().body;
    assert!(!note.contains(CONFIRM), "{note}");
    assert!(
        note.contains("changing an account's address to it"),
        "{note}"
    );
}

#[tokio::test]
async fn an_address_taken_before_the_button_is_pressed_stays_with_its_account() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    signed_up_and_verified(&app, NEW).await;
    let pressed = confirm(&app, &cookie, &secret).await;
    assert_eq!(pressed.status, StatusCode::CONFLICT, "{}", pressed.body);
    let page = app.page("/account", Some(&cookie)).await;
    assert!(page.body.contains(OLD), "{}", page.body);
    assert!(!page.has_class("email-change-pending"), "{}", page.body);
    logged_in(&app, OLD, PASSWORD).await;
}

#[tokio::test]
async fn the_swap_spends_the_reset_link_sent_to_the_old_address() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    app.form("/reset", &[("email", OLD)], None).await;
    let reset = app.link_in_last_email_to(OLD).await;
    assert!(reset.starts_with("/reset/confirm?token="), "{reset}");
    assert_eq!(app.page(&reset, None).await.status, StatusCode::OK);
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    assert_eq!(confirm(&app, &cookie, &secret).await.status, StatusCode::OK);
    assert_eq!(app.page(&reset, None).await.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_swap_drops_a_reset_email_still_waiting_for_the_old_address() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    // Queued for the account while its address is the old one, and not sent yet.
    app.form("/reset", &[("email", OLD)], None).await;
    assert_eq!(confirm(&app, &cookie, &secret).await.status, StatusCode::OK);
    let resets: Vec<Email> = app
        .sent()
        .await
        .into_iter()
        .filter(|email| email.body.contains("/reset/confirm?token="))
        .collect();
    assert!(resets.is_empty(), "{resets:?}");
}

#[tokio::test]
async fn an_unverified_account_changes_its_address_and_is_verified_by_the_swap() {
    let app = test_app().await;
    let signup = [
        ("email", OLD),
        ("password", PASSWORD),
        ("display_name", "Grace"),
    ];
    app.form("/signup", &signup, None).await;
    let verify = app.link_in_last_email_to(OLD).await;
    let cookie = logged_in(&app, OLD, PASSWORD).await;
    let before = emails_to(&app, OLD).await.len();
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    // Nobody proved to hold the old address, so it gets no notice.
    assert_eq!(emails_to(&app, OLD).await.len(), before);
    assert_eq!(confirm(&app, &cookie, &secret).await.status, StatusCode::OK);
    let page = app.page("/account", Some(&cookie)).await;
    assert!(
        !page.body.contains(r#"action="/account/verify-email""#),
        "{}",
        page.body
    );
    assert_eq!(
        app.page(&verify, None).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_new_request_replaces_the_one_waiting() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, "first@new.example", PASSWORD).await;
    let first = emailed_token(&app, "first@new.example").await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let second = emailed_token(&app, NEW).await;
    assert_eq!(
        confirm(&app, &cookie, &first).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(confirm(&app, &cookie, &second).await.status, StatusCode::OK);
    let page = app.page("/account", Some(&cookie)).await;
    assert!(page.body.contains(NEW), "{}", page.body);
}

#[tokio::test]
async fn cancelling_ends_the_change_and_its_link() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    let page = app.page("/account", Some(&cookie)).await;
    assert!(
        page.body.contains(r#"action="/account/email/cancel""#),
        "{}",
        page.body
    );
    let cancelled = app.form("/account/email/cancel", &[], Some(&cookie)).await;
    assert_eq!(cancelled.status, StatusCode::SEE_OTHER);
    assert_eq!(cancelled.location(), Some("/account"));
    assert!(!waiting(&app, &cookie).await);
    assert_eq!(pending_rows(&app).await, 0);
    assert_eq!(
        confirm(&app, &cookie, &secret).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_password_reset_cancels_the_change() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    app.form("/reset", &[("email", OLD)], None).await;
    let reset = app.link_in_last_email_to(OLD).await;
    let token = reset.strip_prefix("/reset/confirm?token=").unwrap();
    let new_password = "a brand new password";
    let changed = app
        .form(
            "/reset/confirm",
            &[("token", token), ("password", new_password)],
            None,
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(pending_rows(&app).await, 0);
    let cookie = logged_in(&app, OLD, new_password).await;
    assert_eq!(
        confirm(&app, &cookie, &secret).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn deleting_the_account_deletes_the_address_it_asked_for() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let deleted = app
        .form("/account/delete", &[("password", PASSWORD)], Some(&cookie))
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(pending_rows(&app).await, 0);
}

#[tokio::test]
async fn requests_count_against_the_new_addresss_emails() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    // The site's limit: 5 emails an hour to an address.
    for round in 0..5 {
        let asked = ask(&app, &cookie, NEW, PASSWORD).await;
        assert_eq!(
            asked.status,
            StatusCode::SEE_OTHER,
            "{round}: {}",
            asked.body
        );
    }
    let refused = ask(&app, &cookie, NEW, PASSWORD).await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        refused.body.contains("emails to this email address"),
        "{}",
        refused.body
    );
}

#[tokio::test]
async fn requests_count_against_the_ips_emails() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    // The site's limit: 20 emails an hour asked for from an IP; signing up took one.
    let mut refused = None;
    for round in 0..20 {
        let asked = ask(&app, &cookie, &format!("g{round}@new.example"), PASSWORD).await;
        if asked.status != StatusCode::SEE_OTHER {
            refused = Some((round, asked));
            break;
        }
    }
    let (round, refused) = refused.expect("the 20th email from the IP is refused");
    assert_eq!(round, 19);
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        refused.body.contains("from this IP address"),
        "{}",
        refused.body
    );
}

#[tokio::test]
async fn a_change_past_its_day_no_longer_works_and_the_tidy_deletes_it() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    let now = OffsetDateTime::now_utc();
    filedrawer_server::email_change::tidy(&app.pool, now + Duration::hours(23))
        .await
        .unwrap();
    assert_eq!(pending_rows(&app).await, 1, "a day isn't over at 23 hours");
    // The request's time moved back a day and a minute, as if it had passed.
    let past = common::stamp(now - Duration::minutes(1));
    sqlx::query("UPDATE email_changes SET expires_at = ?")
        .bind(&past)
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(!waiting(&app, &cookie).await);
    assert_eq!(
        confirm(&app, &cookie, &secret).await.status,
        StatusCode::BAD_REQUEST
    );
    filedrawer_server::email_change::tidy(&app.pool, now)
        .await
        .unwrap();
    assert_eq!(pending_rows(&app).await, 0);
}

#[tokio::test]
async fn a_link_whose_email_failed_to_send_doesnt_work_and_the_retry_sends_a_new_one() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    app.sent().await;
    app.outbox.fail(Some(|email| {
        (email.to == NEW).then_some(MailError::Reply { code: 450 })
    }));
    ask(&app, &cookie, NEW, PASSWORD).await;
    assert!(emails_to(&app, NEW).await.is_empty());
    let hashes: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM email_changes WHERE link_hash IS NOT NULL")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(hashes, 0, "the unsent email's link is retired");
    app.outbox.fail(None);
    let later = OffsetDateTime::now_utc() + Duration::minutes(5);
    filedrawer_server::mail_queue::send_due(&app.state, later)
        .await
        .unwrap();
    let secret = emailed_token(&app, NEW).await;
    assert_eq!(confirm(&app, &cookie, &secret).await.status, StatusCode::OK);
}

/// Turns away, for now, every email to the old address, so it waits in the queue.
fn hold_mail_to_the_old_address(app: &TestApp) {
    app.outbox.fail(Some(|email| {
        (email.to == OLD).then_some(MailError::Reply { code: 450 })
    }));
}

/// Lets mail through again and sends what waited, and returns every email sent.
async fn sent_once_mail_flows(app: &TestApp) -> Vec<Email> {
    app.outbox.fail(None);
    let later = OffsetDateTime::now_utc() + Duration::minutes(5);
    filedrawer_server::mail_queue::send_due(&app.state, later)
        .await
        .unwrap();
    app.outbox.sent()
}

const NOTICE_SUBJECT: &str = "A change of your File Drawer email address was asked for";

#[tokio::test]
async fn the_swap_drops_the_notice_and_the_taken_note_still_waiting_for_the_old_address() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    app.sent().await;
    hold_mail_to_the_old_address(&app);
    ask(&app, &cookie, NEW, PASSWORD).await;
    let secret = emailed_token(&app, NEW).await;
    // Someone signs up with the old address, so its account is owed a note.
    let signup = [
        ("email", OLD),
        ("password", PASSWORD),
        ("display_name", "Mallory"),
    ];
    app.form("/signup", &signup, None).await;
    assert_eq!(confirm(&app, &cookie, &secret).await.status, StatusCode::OK);
    let before = app.outbox.sent().len();
    let sent = sent_once_mail_flows(&app).await;
    // Both were for the old inbox, and the account's address is the new one now.
    assert_eq!(sent.len(), before, "{:?}", &sent[before..]);
}

#[tokio::test]
async fn a_notice_still_waiting_when_the_change_is_cancelled_never_goes() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    app.sent().await;
    hold_mail_to_the_old_address(&app);
    ask(&app, &cookie, NEW, PASSWORD).await;
    app.sent().await;
    app.form("/account/email/cancel", &[], Some(&cookie)).await;
    let sent = sent_once_mail_flows(&app).await;
    let notices: Vec<&Email> = sent
        .iter()
        .filter(|email| email.subject == NOTICE_SUBJECT)
        .collect();
    assert!(notices.is_empty(), "{notices:?}");
}

#[tokio::test]
async fn an_account_with_no_password_cant_change_its_address_with_the_form() {
    let app = test_app().await;
    let (cookie, _) = grace(&app).await;
    // As `revoke-all --passwords` leaves an account, were its session to survive.
    sqlx::query("UPDATE accounts SET password_hash = NULL")
        .execute(&app.pool)
        .await
        .unwrap();
    let page = app.page("/account", Some(&cookie)).await;
    assert!(
        !page.body.contains(r#"action="/account/email""#),
        "{}",
        page.body
    );
    let refused = ask(&app, &cookie, NEW, "").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(pending_rows(&app).await, 0);
}

// An account with no password confirms through GitHub.

/// The app with GitHub sign-in on, and its fake GitHub.
async fn github_app() -> (TestApp, FakeGithub) {
    let github = FakeGithub::default();
    let app = test_app().await.with_github(&github);
    (app, github)
}

/// The `state` in the GitHub authorize URL a start redirected to.
fn state_in(reply: &Reply) -> String {
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    let url = url::Url::parse(reply.location().unwrap()).unwrap();
    assert_eq!(url.host_str(), Some("github.test"), "{url}");
    let state = url.query_pairs().find(|(key, _)| key == "state");
    state.expect("the authorize URL has a state").1.into_owned()
}

/// Comes back from GitHub with `code`, carrying `cookies`.
async fn come_back(app: &TestApp, state: &str, code: &str, cookies: &str) -> Reply {
    let path = format!("/login/github/callback?code={code}&state={state}");
    app.page(&path, Some(cookies)).await
}

/// Grace signed up with GitHub, as GitHub user 1 with `OLD`, and has no password:
/// her session cookie.
async fn grace_from_github(app: &TestApp, github: &FakeGithub) -> String {
    github.code_for("signup", octo(1, "grace-gh", OLD));
    let started = app.form("/login/github", &[], None).await;
    let browser = started.cookie().unwrap();
    let back = come_back(app, &state_in(&started), "signup", &browser).await;
    let pending = back.cookie().unwrap();
    let fields = [("display_name", "Grace"), ("email", "0")];
    let done = app.form("/signup/github", &fields, Some(&pending)).await;
    assert_eq!(done.location(), Some("/account"), "{}", done.body);
    done.cookie().expect("the signup logs in")
}

/// Asks to change the session's address to `to` from the form of an account with no
/// password, and returns GitHub's state and the cookies to come back with.
async fn ask_through_github(app: &TestApp, session: &str, to: &str) -> (String, String) {
    let started = app
        .form("/account/email", &[("email", to)], Some(session))
        .await;
    let state = state_in(&started);
    let browser = started.cookie().expect("the start sets its cookie");
    (state, format!("{session}; {browser}"))
}

#[tokio::test]
async fn an_account_with_no_password_changes_its_address_once_github_confirms_it() {
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    let page = app.page("/account", Some(&session)).await;
    assert!(
        page.body.contains(r#"action="/account/email""#),
        "{}",
        page.body
    );
    assert!(!page.body.contains(r#"name="password""#), "{}", page.body);

    let (state, cookies) = ask_through_github(&app, &session, NEW).await;
    // Nothing waits and no email goes until GitHub's answer comes back.
    assert!(!waiting(&app, &session).await);
    assert!(emails_to(&app, NEW).await.is_empty());
    github.code_for("again", octo(1, "grace-gh", OLD));
    let back = come_back(&app, &state, "again", &cookies).await;
    assert_eq!(back.status, StatusCode::SEE_OTHER, "{}", back.body);
    assert_eq!(back.location(), Some("/account"));
    assert!(waiting(&app, &session).await);

    let secret = emailed_token(&app, NEW).await;
    assert_eq!(
        confirm(&app, &session, &secret).await.status,
        StatusCode::OK
    );
    let page = app.page("/account", Some(&session)).await;
    assert!(page.body.contains(NEW), "{}", page.body);
    assert!(page.body.contains("github:grace-gh"), "GitHub stays linked");
}

#[tokio::test]
async fn another_github_account_confirms_nothing() {
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    let (state, cookies) = ask_through_github(&app, &session, NEW).await;
    github.code_for("other", octo(2, "mallory-gh", "mallory@example.com"));
    let back = come_back(&app, &state, "other", &cookies).await;
    assert_eq!(back.status, StatusCode::FORBIDDEN, "{}", back.body);
    assert_eq!(pending_rows(&app).await, 0);
    assert!(emails_to(&app, NEW).await.is_empty());
}

#[tokio::test]
async fn a_round_trip_finished_under_another_accounts_session_confirms_nothing() {
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    let (other, _) = browser_account(&app, "hopper@example.com").await;
    let (state, cookies) = ask_through_github(&app, &session, NEW).await;
    let swapped = cookies.replace(&session, &other);
    github.code_for("again", octo(1, "grace-gh", OLD));
    let back = come_back(&app, &state, "again", &swapped).await;
    assert_eq!(back.status, StatusCode::FORBIDDEN, "{}", back.body);
    assert_eq!(pending_rows(&app).await, 0);
}

#[tokio::test]
async fn an_address_edited_in_the_cookie_is_checked_again() {
    // The cookie is the browser's to edit, so the callback checks the address the
    // form did.
    use filedrawer_server::github_accounts::{Purpose, SessionTag, add_start};
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    let secret = session.split_once('=').unwrap().1;
    for edited in ["not an address", "Mallory@Example.com"] {
        let purpose = Purpose::ChangeEmail {
            session: SessionTag::of(secret),
            new_email: edited.into(),
        };
        let (started, kept) = add_start(None, purpose, None, time::OffsetDateTime::now_utc());
        github.code_for("again", octo(1, "grace-gh", OLD));
        let cookies = format!("{session}; __Host-fd_github={kept}");
        let back = come_back(&app, &started.state, "again", &cookies).await;
        assert_eq!(
            back.status,
            StatusCode::BAD_REQUEST,
            "{edited}: {}",
            back.body
        );
    }
    assert_eq!(pending_rows(&app).await, 0);
}

#[tokio::test]
async fn a_round_trip_written_into_the_cookie_for_an_account_with_a_password_confirms_nothing() {
    // The form sends only an account with no password through GitHub; a start
    // written into the cookie by hand mustn't stand in for the password.
    use filedrawer_server::github_accounts::{Purpose, SessionTag, add_start};
    let (app, github) = github_app().await;
    let (session, _) = browser_account(&app, "hopper@example.com").await;
    let linking = app.form("/account/github/link", &[], Some(&session)).await;
    let browser = linking.cookie().unwrap();
    github.code_for("link", octo(2, "hopper-gh", "hopper@example.com"));
    let cookies = format!("{session}; {browser}");
    let linked = come_back(&app, &state_in(&linking), "link", &cookies).await;
    assert_eq!(linked.location(), Some("/account"), "{}", linked.body);

    let secret = session.split_once('=').unwrap().1;
    let purpose = Purpose::ChangeEmail {
        session: SessionTag::of(secret),
        new_email: NEW.into(),
    };
    let (started, kept) = add_start(None, purpose, None, time::OffsetDateTime::now_utc());
    github.code_for("again", octo(2, "hopper-gh", "hopper@example.com"));
    let cookies = format!("{session}; __Host-fd_github={kept}");
    let back = come_back(&app, &started.state, "again", &cookies).await;
    assert_eq!(back.status, StatusCode::FORBIDDEN, "{}", back.body);
    assert_eq!(pending_rows(&app).await, 0);
    assert!(emails_to(&app, NEW).await.is_empty());
}

#[tokio::test]
async fn round_trips_left_unfinished_count_nothing_and_leave_a_waiting_change_alone() {
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    let (state, cookies) = ask_through_github(&app, &session, NEW).await;
    github.code_for("again", octo(1, "grace-gh", OLD));
    come_back(&app, &state, "again", &cookies).await;
    let secret = emailed_token(&app, NEW).await;
    // More starts than the address's 5 emails an hour, none of them finished.
    for _ in 0..6 {
        ask_through_github(&app, &session, NEW).await;
    }
    assert_eq!(
        confirm(&app, &session, &secret).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_finished_round_trip_counts_against_the_new_addresss_emails() {
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    let mut last = None;
    for round in 0..6 {
        let (state, cookies) = ask_through_github(&app, &session, NEW).await;
        let code = format!("again{round}");
        github.code_for(&code, octo(1, "grace-gh", OLD));
        last = Some(come_back(&app, &state, &code, &cookies).await);
    }
    let sixth = last.unwrap();
    assert_eq!(
        sixth.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        sixth.body
    );
}

#[tokio::test]
async fn a_github_account_linked_to_another_account_confirms_nothing() {
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    // Hopper's account links GitHub user 2.
    let (hopper, _) = browser_account(&app, "hopper@example.com").await;
    let linking = app.form("/account/github/link", &[], Some(&hopper)).await;
    let browser = linking.cookie().unwrap();
    github.code_for("link", octo(2, "hopper-gh", "hopper@example.com"));
    let linked = come_back(
        &app,
        &state_in(&linking),
        "link",
        &format!("{hopper}; {browser}"),
    )
    .await;
    assert_eq!(linked.location(), Some("/account"), "{}", linked.body);

    let (state, cookies) = ask_through_github(&app, &session, NEW).await;
    github.code_for("other", octo(2, "hopper-gh", "hopper@example.com"));
    let back = come_back(&app, &state, "other", &cookies).await;
    assert_eq!(back.status, StatusCode::FORBIDDEN, "{}", back.body);
    assert_eq!(pending_rows(&app).await, 0);
}

#[tokio::test]
async fn a_round_trip_that_comes_back_to_an_address_the_account_has_by_now_records_nothing() {
    let (app, github) = github_app().await;
    let session = grace_from_github(&app, &github).await;
    let (first, cookies) = ask_through_github(&app, &session, NEW).await;
    // The same browser starts a second round trip, as from another tab; its start
    // cookie then holds both.
    let started = app
        .form("/account/email", &[("email", NEW)], Some(&cookies))
        .await;
    let second = state_in(&started);
    let cookies = format!("{session}; {}", started.cookie().unwrap());
    github.code_for("one", octo(1, "grace-gh", OLD));
    come_back(&app, &first, "one", &cookies).await;
    let secret = emailed_token(&app, NEW).await;
    assert_eq!(
        confirm(&app, &session, &secret).await.status,
        StatusCode::OK
    );
    let before = app.sent().await.len();
    github.code_for("two", octo(1, "grace-gh", OLD));
    let back = come_back(&app, &second, "two", &cookies).await;
    assert_eq!(back.location(), Some("/account"), "{}", back.body);
    assert_eq!(pending_rows(&app).await, 0);
    assert_eq!(app.sent().await.len(), before);
}

#[tokio::test]
async fn with_github_on_an_account_with_no_password_and_no_github_gets_no_form() {
    let (app, _github) = github_app().await;
    let (cookie, _) = grace(&app).await;
    sqlx::query("UPDATE accounts SET password_hash = NULL")
        .execute(&app.pool)
        .await
        .unwrap();
    let page = app.page("/account", Some(&cookie)).await;
    assert!(
        !page.body.contains(r#"action="/account/email""#),
        "{}",
        page.body
    );
    let refused = ask(&app, &cookie, NEW, "").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
}
