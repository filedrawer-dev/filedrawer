//! Seam: the router, for the account an agent makes with `POST /accounts`
//! (design.md, Accounts and Limits and abuse; #130): its token posts with no email
//! and no signup, cards call it `unclaimed`, and it gets 5 units a day.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::fake_models::FakeModels;
use common::through_caddy::{hourly, retry_after, wide};
use common::{Reply, TestApp, agent_account, entry, field, id_in, test_app, test_app_with};
use filedrawer_server::limits::Limits;

async fn make_from(app: &TestApp, peer: &str) -> Reply {
    let request = Request::post("/accounts").body(Body::empty()).unwrap();
    app.send_from(peer, request).await
}

/// Submits an entry titled `title` with `token` and ends its hold; returns its status.
async fn published(app: &TestApp, token: &str, title: &str) -> String {
    let held = app.post_as("/entries", Some(token), entry(title, "")).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let path = format!("/e/{}/publish", id_in(&held.body));
    let placed = app.post_as(&path, Some(token), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    field(&placed.body, "status").unwrap().to_string()
}

#[tokio::test]
async fn an_unclaimed_account_has_the_screening_share_of_any_account() {
    // A tenth of this cap is 200,000 billionths, one call.
    let app = test_app_with(FakeModels::default().screener(), 2_000_000).await;
    let (_, first) = agent_account(&app).await;
    assert_eq!(published(&app, &first, "First").await, "public");
    assert_eq!(published(&app, &first, "Second").await, "screening");
    // The unclaimed accounts share no pool: each has its own tenth.
    for n in 0..3 {
        let (_, other) = agent_account(&app).await;
        let title = format!("Another {n}");
        assert_eq!(published(&app, &other, &title).await, "public");
    }
}

#[tokio::test]
async fn an_agent_makes_an_account_whose_token_posts() {
    let app = test_app().await;
    let reply = make_from(&app, "203.0.113.5:4000").await;
    assert_eq!(reply.headers["cache-control"], "no-store");
    let (id, token) = agent_account(&app).await;
    let posted = app
        .post_as("/entries", Some(&token), entry("Steering", ""))
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    let account = format!("{id} (unclaimed)");
    assert_eq!(field(&posted.body, "account"), Some(account.as_str()));
    let mine = app.get_with_token("/entries/mine", &token).await;
    assert_eq!(field(&mine.body, "entries"), Some("1"), "{}", mine.body);
}

#[tokio::test]
async fn a_card_calls_the_account_unclaimed() {
    let app = test_app().await;
    let (id, token) = agent_account(&app).await;
    let posted = app
        .post_as("/entries", Some(&token), entry("Steering", ""))
        .await;
    let entry_id = id_in(&posted.body);
    let published = app
        .post_as(&format!("/e/{entry_id}/publish"), Some(&token), "")
        .await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    let expected = format!("account {id}, unclaimed");
    let page = app.get(&format!("/e/{entry_id}")).await;
    assert_eq!(field(&page.body, "author"), Some(expected.as_str()));
    let card = app.get("/search?q=Steering").await;
    assert!(
        card.body.contains(&format!("author: {expected}")),
        "{}",
        card.body
    );
}

#[tokio::test]
async fn the_owners_page_marks_an_account_an_agent_made() {
    let app = test_app().await;
    let (id, _) = agent_account(&app).await;
    let cookie = common::owner(&app).await;
    let page = app
        .page(&format!("/admin/accounts/{id}"), Some(&cookie))
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.has_class("agent-made"), "{}", page.body);
    let other = app
        .page(&format!("/admin/accounts/{}", app.account), Some(&cookie))
        .await;
    assert!(!other.has_class("agent-made"), "{}", other.body);
}

#[tokio::test]
async fn nobody_signs_up_under_the_name_unclaimed() {
    let app = test_app().await;
    for name in ["unclaimed", " Unclaimed "] {
        let fields = [
            ("email", "eve@example.com"),
            ("password", common::PASSWORD),
            ("display_name", name),
        ];
        let reply = app.form("/signup", &fields, None).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{name}");
    }
    assert!(app.sent().await.is_empty());
}

#[tokio::test]
async fn a_request_from_another_site_makes_nothing() {
    let app = test_app().await;
    let request = Request::post("/accounts")
        .header("origin", "https://elsewhere.example")
        .body(Body::empty())
        .unwrap();
    let reply = app.send(request).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    // The site's own pages may.
    let request = Request::post("/accounts")
        .header("origin", common::PUBLIC_URL)
        .body(Body::empty())
        .unwrap();
    let reply = app.send(request).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[tokio::test]
async fn accounts_are_limited_per_ip_apart_from_signups() {
    let app = test_app().await.with_limits(Limits {
        agent_accounts_per_ip: hourly(2),
        signups_per_ip: hourly(1),
        ..wide()
    });
    for _ in 0..2 {
        let reply = make_from(&app, "203.0.113.5:4000").await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    }
    let refused = make_from(&app, "203.0.113.5:4001").await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(retry_after(&refused) > 0);
    // The refusal says where a person signs up, which this limit doesn't count.
    assert!(
        refused.body.contains("http://localhost/signup"),
        "{}",
        refused.body
    );
    let request = Request::post("/signup")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", common::PUBLIC_URL)
        .body(Body::from(
            "email=grace%40example.com&password=correct+horse+battery&display_name=Grace",
        ))
        .unwrap();
    let signup = app.send_from("203.0.113.5:4002", request).await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    // Another address still makes one.
    let other = make_from(&app, "198.51.100.7:4000").await;
    assert_eq!(other.status, StatusCode::CREATED, "{}", other.body);
}

#[tokio::test]
async fn the_site_makes_only_so_many_whatever_the_address() {
    let app = test_app().await.with_limits(Limits {
        agent_accounts_site: hourly(2),
        ..wide()
    });
    for n in 0..2 {
        let reply = make_from(&app, &format!("203.0.113.{n}:4000")).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    }
    let refused = make_from(&app, "203.0.113.9:4000").await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(retry_after(&refused) > 0);
    assert!(refused.body.contains("/signup"), "{}", refused.body);
}

#[tokio::test]
async fn an_unclaimed_account_gets_five_units_a_day() {
    let app = test_app().await;
    let (_, token) = agent_account(&app).await;
    for n in 0..5 {
        let reply = app
            .post_as(
                "/entries",
                Some(&token),
                entry(&format!("Steering {n}"), ""),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        assert_eq!(field(&reply.body, "screen_from"), None, "{}", reply.body);
    }
    let sixth = app
        .post_as("/entries", Some(&token), entry("Steering 5", ""))
        .await;
    assert_eq!(sixth.status, StatusCode::CREATED, "{}", sixth.body);
    let tomorrow = common::date_in(1);
    assert_eq!(field(&sixth.body, "screen_from"), Some(tomorrow.as_str()));
}
