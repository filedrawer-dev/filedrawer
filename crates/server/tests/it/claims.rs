//! Seam: the router, with the mail queue's sender run by hand, for claiming an
//! account an agent made (design.md, Accounts; #130): the agent names an address and
//! gets a code, the address gets a link, and whoever opens the link and enters the
//! code gives the account an owner. The agent's token keeps working.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::through_caddy::{hourly, wide};
use common::{PASSWORD, Reply, TestApp, agent_account, entry, field, id_in, test_app};
use filedrawer_server::limits::Limits;

const ADA: &str = "ada@example.com";

/// Asks, with `token`, to claim its account for `email`.
async fn ask(app: &TestApp, token: &str, email: &str) -> Reply {
    let request = Request::post("/account/claim")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("email={}", email.replace('@', "%40"))))
        .unwrap();
    app.send(request).await
}

/// Asks, and returns the code the reply gives and the link the address got.
async fn asked(app: &TestApp, token: &str, email: &str) -> (String, String) {
    let reply = ask(app, token, email).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let code = field(&reply.body, "code").expect(&reply.body).to_string();
    let link = app.link_in_last_email_to(email).await;
    assert!(link.starts_with("/claim?token="), "{link}");
    (code, link)
}

/// Presses the claim page's button with `code`, naming the account Ada.
async fn confirm(app: &TestApp, link: &str, code: &str, cookie: Option<&str>) -> Reply {
    let secret = link.strip_prefix("/claim?token=").unwrap();
    let fields = [
        ("token", secret),
        ("code", code),
        ("display_name", "Ada"),
        ("password", PASSWORD),
    ];
    app.form("/claim", &fields, cookie).await
}

#[tokio::test]
async fn the_address_claims_the_account_with_the_agents_code() {
    let app = test_app().await;
    let (id, token) = agent_account(&app).await;
    let posted = app
        .post_as("/entries", Some(&token), entry("Steering", ""))
        .await;
    let (code, link) = asked(&app, &token, ADA).await;
    // The agent shows a code shaped for reading aloud.
    assert_eq!(code.len(), 9, "{code}");
    // The link only shows a page.
    let page = app.page(&link, None).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.has_class("claim"), "{}", page.body);
    assert!(page.body.contains(&id), "{}", page.body);
    let unclaimed = format!("{id} (unclaimed)");
    let mine = app.get_with_token("/entries/mine", &token).await;
    assert_eq!(field(&mine.body, "account"), Some(unclaimed.as_str()));

    let done = confirm(&app, &link, &code, None).await;
    assert!(done.status.is_redirection(), "{}", done.body);
    let cookie = done.cookie().expect("the claim logs the browser in");
    assert_eq!(common::account_id(&app, &cookie).await, id);
    // The agent's token still posts, now under the name.
    let named = format!("{id} (Ada)");
    let mine = app.get_with_token("/entries/mine", &token).await;
    assert_eq!(field(&mine.body, "account"), Some(named.as_str()));
    assert_eq!(field(&mine.body, "entries"), Some("1"), "{}", mine.body);
    let _ = id_in(&posted.body);
    // The address and the password log in.
    common::logged_in(&app, ADA, PASSWORD).await;
    // The link is spent, and the account can't be claimed twice.
    let again = confirm(&app, &link, &code, None).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST, "{}", again.body);
    let second = ask(&app, &token, "grace@example.com").await;
    assert_eq!(second.status, StatusCode::CONFLICT, "{}", second.body);
}

#[tokio::test]
async fn the_link_without_the_code_claims_nothing() {
    let app = test_app().await;
    let (id, token) = agent_account(&app).await;
    let (code, link) = asked(&app, &token, ADA).await;
    let wrong = if code == "aaaa-aaaa" {
        "bbbb-bbbb"
    } else {
        "aaaa-aaaa"
    };
    for _ in 0..4 {
        let reply = confirm(&app, &link, wrong, None).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
    }
    // The fifth wrong code ends the claim, and the page says so.
    let fifth = confirm(&app, &link, wrong, None).await;
    assert_eq!(fifth.status, StatusCode::BAD_REQUEST, "{}", fifth.body);
    // So the right one comes too late.
    let late = confirm(&app, &link, &code, None).await;
    assert_eq!(late.status, StatusCode::BAD_REQUEST, "{}", late.body);
    let unclaimed = format!("{id} (unclaimed)");
    let mine = app.get_with_token("/entries/mine", &token).await;
    assert_eq!(field(&mine.body, "account"), Some(unclaimed.as_str()));
}

#[tokio::test]
async fn the_code_is_read_as_a_person_types_it() {
    let app = test_app().await;
    let (_, token) = agent_account(&app).await;
    let (code, link) = asked(&app, &token, ADA).await;
    // Capitals, spaces for the dash, and the letters a 0 and a 1 look like.
    let typed = format!(" {} ", code.replace('-', " ").to_uppercase())
        .replace('0', "O")
        .replace('1', "l");
    let done = confirm(&app, &link, &typed, None).await;
    assert!(done.status.is_redirection(), "{}", done.body);
}

#[tokio::test]
async fn an_account_suspended_since_it_asked_isnt_claimed() {
    let app = test_app().await;
    let (id, token) = agent_account(&app).await;
    let (code, link) = asked(&app, &token, ADA).await;
    let owner = common::owner(&app).await;
    let suspend = format!("/admin/accounts/{id}/suspend");
    let suspended = app.form(&suspend, &[], Some(&owner)).await;
    assert!(suspended.status.is_redirection(), "{}", suspended.body);
    let page = app.page(&link, None).await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST, "{}", page.body);
    let reply = confirm(&app, &link, &code, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
}

#[tokio::test]
async fn a_claim_fills_each_day_only_to_its_room() {
    let app = test_app().await;
    let (_, token) = agent_account(&app).await;
    // 5 a day for 10 days.
    let mut ids = Vec::new();
    for n in 0..50 {
        let reply = app
            .post_as(
                "/entries",
                Some(&token),
                entry(&format!("Steering {n}"), ""),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        ids.push(id_in(&reply.body));
    }
    let (code, link) = asked(&app, &token, ADA).await;
    confirm(&app, &link, &code, None).await;
    // A claimed account's first days hold 20 each: 20 today, 20 tomorrow, 10 after.
    let (tomorrow, after) = (common::date_in(1), common::date_in(2));
    for (nth, day) in [
        (19, None),
        (20, Some(tomorrow.as_str())),
        (39, Some(tomorrow.as_str())),
        (40, Some(after.as_str())),
        (49, Some(after.as_str())),
    ] {
        let own = app
            .get_with_token(&format!("/e/{}", ids[nth]), &token)
            .await;
        assert_eq!(field(&own.body, "screen_from"), day, "entry {nth}");
    }
}

#[tokio::test]
async fn a_new_request_replaces_the_one_before() {
    let app = test_app().await;
    let (_, token) = agent_account(&app).await;
    let (old_code, old_link) = asked(&app, &token, ADA).await;
    let (code, link) = asked(&app, &token, "grace@example.com").await;
    let old = confirm(&app, &old_link, &old_code, None).await;
    assert_eq!(old.status, StatusCode::BAD_REQUEST, "{}", old.body);
    let done = confirm(&app, &link, &code, None).await;
    assert!(done.status.is_redirection(), "{}", done.body);
    common::logged_in(&app, "grace@example.com", PASSWORD).await;
}

#[tokio::test]
async fn only_an_unclaimed_accounts_token_asks() {
    let app = test_app().await;
    // The app's own account signed up.
    let reply = ask(&app, &app.token, ADA).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
    let (_, token) = agent_account(&app).await;
    let bad = ask(&app, &token, "not an address").await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", bad.body);
    assert!(app.sent().await.is_empty());
}

#[tokio::test]
async fn an_address_with_an_account_gets_the_same_reply_and_email() {
    let app = test_app().await;
    common::signed_up_and_verified(&app, ADA).await;
    let before = app.sent().await.len();
    let (_, token) = agent_account(&app).await;
    let (code, link) = asked(&app, &token, ADA).await;
    assert_eq!(app.sent().await.len(), before + 1);
    // The link's page says so at once, to whoever holds the inbox, with no form to
    // fill in first.
    let page = app.page(&link, None).await;
    assert_eq!(page.status, StatusCode::CONFLICT, "{}", page.body);
    assert!(!page.has_class("claim"), "{}", page.body);
    // Nothing is claimed with an address another account has, code or no code.
    let reply = confirm(&app, &link, &code, None).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
    // Codes typed meanwhile don't use the claim up: the sixth wrong one still
    // gets this page, where a fifth would end a claim whose address is free.
    for _ in 0..6 {
        let guess = confirm(&app, &link, "0000-0000", None).await;
        assert_eq!(guess.status, StatusCode::CONFLICT, "{}", guess.body);
    }
    let again = ask(&app, &token, "grace@example.com").await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
}

#[tokio::test]
async fn claims_are_limited_per_account_and_across_the_site() {
    let app = test_app().await.with_limits(Limits {
        claims_per_account: hourly(2),
        ..wide()
    });
    let (_, token) = agent_account(&app).await;
    for n in 0..2 {
        let reply = ask(&app, &token, &format!("a{n}@example.com")).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
    let refused = ask(&app, &token, "a2@example.com").await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);

    let app = test_app().await.with_limits(Limits {
        claims_site: hourly(1),
        ..wide()
    });
    let (_, first) = agent_account(&app).await;
    let (_, second) = agent_account(&app).await;
    assert_eq!(ask(&app, &first, ADA).await.status, StatusCode::OK);
    let refused = ask(&app, &second, "grace@example.com").await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn an_expired_claim_is_tidied_away() {
    let app = test_app().await;
    let (_, token) = agent_account(&app).await;
    let (code, link) = asked(&app, &token, ADA).await;
    let later = time::OffsetDateTime::now_utc() + time::Duration::days(2);
    filedrawer_server::claims::tidy(&app.pool, later)
        .await
        .unwrap();
    let reply = confirm(&app, &link, &code, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
}

#[tokio::test]
async fn a_claim_moves_the_waiting_entries_up() {
    let app = test_app().await;
    let (_, token) = agent_account(&app).await;
    let mut last = String::new();
    for n in 0..6 {
        let reply = app
            .post_as(
                "/entries",
                Some(&token),
                entry(&format!("Steering {n}"), ""),
            )
            .await;
        last = id_in(&reply.body);
    }
    // The sixth waits for tomorrow's units.
    let path = format!("/e/{last}");
    let waiting = app.get_with_token(&path, &token).await;
    let tomorrow = common::date_in(1);
    assert_eq!(field(&waiting.body, "screen_from"), Some(tomorrow.as_str()));
    let (code, link) = asked(&app, &token, ADA).await;
    confirm(&app, &link, &code, None).await;
    // A claimed account has 20 a day, so today has room.
    let moved = app.get_with_token(&path, &token).await;
    assert_eq!(field(&moved.body, "screen_from"), None, "{}", moved.body);
}
