//! Seam: the router, for invite codes (design.md, Limits and abuse): the owner
//! creates them, an account signs up with one, and the code lifts the per-IP signup
//! and email limits and the new-account allowance.

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Reply, TestApp, date_in, entry, field, id_in, owner, test_app};
use filedrawer_server::limits::{Limits, Rate, Window};

const CADDY: &str = "127.0.0.1:50000";

fn hourly(count: i64) -> Vec<Window> {
    vec![Window {
        seconds: 3600,
        count,
    }]
}

/// Limits wide enough that only the one a test sets small trips.
fn wide() -> Limits {
    let rate = Rate {
        count: 10_000,
        per: Duration::from_secs(60),
    };
    Limits {
        per_ip: rate,
        writes_per_account: rate,
        signups_per_ip: hourly(10_000),
        emails_per_address: hourly(10_000),
        emails_per_ip: hourly(10_000),
        failed_logins_per_address: hourly(10_000),
    }
}

/// Creates a code on the owner's page and returns it.
async fn create_code(app: &TestApp, cookie: &str, label: &str, uses: &str, days: &str) -> String {
    let made = app
        .form(
            "/admin/invites",
            &[("label", label), ("uses", uses), ("days", days)],
            Some(cookie),
        )
        .await;
    assert_eq!(made.status, StatusCode::SEE_OTHER, "{}", made.body);
    let page = app.page("/admin/invites", Some(cookie)).await;
    let start = page.body.find("inv-").expect("the page shows the code");
    page.body[start..start + 20].to_string()
}

/// POSTs the signup form from `forwarded` through Caddy, with `code` if given.
async fn sign_up_as(app: &TestApp, forwarded: &str, email: &str, code: Option<&str>) -> Reply {
    let mut body = format!("email={email}&password=correct-horse-battery-9&display_name=Ada");
    if let Some(code) = code {
        body.push_str(&format!("&invite={code}"));
    }
    let request = Request::post("/signup")
        .header("x-forwarded-for", forwarded)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", common::PUBLIC_URL)
        .body(Body::from(body))
        .unwrap();
    app.send_from(CADDY, request).await
}

#[tokio::test]
async fn the_owner_creates_a_code_and_sees_its_uses_and_expiry() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Apart sprint", "120", "14").await;
    assert_eq!(code.len(), 20, "{code}");
    let page = app.page("/admin/invites", Some(&cookie)).await;
    assert!(page.body.contains("Apart sprint"), "{}", page.body);
    assert!(page.body.contains("0 of 120"), "{}", page.body);
    assert!(page.body.contains(&date_in(14)), "{}", page.body);
    // Nobody else reaches it.
    let (other, _) = common::browser_account(&app, "mallory@example.com").await;
    assert_eq!(
        app.page("/admin/invites", Some(&other)).await.status,
        StatusCode::NOT_FOUND
    );
    let forged = app
        .form(
            "/admin/invites",
            &[("label", "x"), ("uses", "9"), ("days", "1")],
            Some(&other),
        )
        .await;
    assert_eq!(forged.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_link_fills_in_the_code() {
    let app = test_app().await;
    let page = app.page("/signup?invite=inv-abc", None).await;
    assert!(
        page.body.contains(r#"name="invite" value="inv-abc""#),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn a_sprint_behind_one_ip_signs_up_and_verifies_past_the_per_ip_limits() {
    let app = test_app().await.with_limits(Limits {
        signups_per_ip: hourly(1),
        emails_per_ip: hourly(1),
        ..wide()
    });
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Sprint", "10", "7").await;
    // One plain signup fills the IP's hour.
    let plain = sign_up_as(&app, "203.0.113.5", "q%40example.com", None).await;
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.body);
    for n in 0..4 {
        let reply = sign_up_as(
            &app,
            "203.0.113.5",
            &format!("p{n}%40example.com"),
            Some(&code),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
    // Without the code, the same IP is past its limit.
    let again = sign_up_as(&app, "203.0.113.5", "r%40example.com", None).await;
    assert_eq!(
        again.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        again.body
    );
    let page = app.page("/admin/invites", Some(&cookie)).await;
    assert!(page.body.contains("4 of 10"), "{}", page.body);
}

#[tokio::test]
async fn an_invited_signup_still_counts_emails_per_address() {
    let app = test_app().await.with_limits(Limits {
        emails_per_address: hourly(1),
        ..wide()
    });
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Sprint", "10", "7").await;
    let first = sign_up_as(&app, "203.0.113.5", "p%40example.com", Some(&code)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let again = sign_up_as(&app, "203.0.113.6", "p%40example.com", Some(&code)).await;
    assert_eq!(
        again.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        again.body
    );
}

#[tokio::test]
async fn an_unknown_used_up_or_expired_code_refuses_the_signup() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let two = create_code(&app, &cookie, "Two", "2", "7").await;
    assert_eq!(
        sign_up_as(&app, "203.0.113.5", "a%40example.com", Some(&two))
            .await
            .status,
        StatusCode::OK
    );
    // A signup to a taken address takes no use, so a second account still fits.
    assert_eq!(
        sign_up_as(&app, "203.0.113.5", "a%40example.com", Some(&two))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        sign_up_as(&app, "203.0.113.5", "e%40example.com", Some(&two))
            .await
            .status,
        StatusCode::OK
    );
    // A used-up code refuses every signup, a taken address's too, so the answer
    // never says whether an address has an account.
    let taken = sign_up_as(&app, "203.0.113.5", "a%40example.com", Some(&two)).await;
    assert_eq!(
        taken.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        taken.body
    );
    let used_up = sign_up_as(&app, "203.0.113.5", "b%40example.com", Some(&two)).await;
    assert_eq!(
        used_up.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        used_up.body
    );
    assert!(used_up.body.contains("invite code"), "{}", used_up.body);
    let unknown = sign_up_as(
        &app,
        "203.0.113.5",
        "c%40example.com",
        Some("inv-nosuchcode00000"),
    )
    .await;
    assert_eq!(
        unknown.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        unknown.body
    );
    let expired = create_code(&app, &cookie, "Old", "5", "1").await;
    sqlx::query("UPDATE invite_codes SET expires_at = '2020-01-01T00:00:00Z' WHERE code = ?")
        .bind(&expired)
        .execute(&app.pool)
        .await
        .unwrap();
    let late = sign_up_as(&app, "203.0.113.5", "d%40example.com", Some(&expired)).await;
    assert_eq!(
        late.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        late.body
    );
    // None of the refused signups made an account.
    let sent = app.outbox.sent();
    for email in ["b@example.com", "c@example.com", "d@example.com"] {
        assert!(sent.iter().all(|mail| mail.to != email), "{email}");
    }
}

#[tokio::test]
async fn an_invited_account_gets_50_a_day_from_its_first_day() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Sprint", "10", "7").await;
    let signup = sign_up_as(&app, "203.0.113.5", "grace%40example.com", Some(&code)).await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    let link = app.link_in_last_email_to("grace@example.com").await;
    let token = link.strip_prefix("/verify?token=").unwrap();
    app.form("/verify", &[("token", token)], None).await;
    let session = common::logged_in(&app, "grace@example.com", "correct-horse-battery-9").await;
    let token = common::new_token(&app, &session).await;
    for n in 0..21 {
        let reply = app
            .post_as(
                "/entries",
                Some(&token),
                entry(&format!("Steering {n}"), ""),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        assert_eq!(
            field(&reply.body, "screen_from"),
            None,
            "entry {n}: {}",
            reply.body
        );
        let _ = id_in(&reply.body);
    }
}

#[tokio::test]
async fn an_invited_accounts_resends_count_per_address_only() {
    let app = test_app().await.with_limits(Limits {
        emails_per_ip: hourly(1),
        ..wide()
    });
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Sprint", "10", "7").await;
    let signup = sign_up_as(&app, "203.0.113.5", "grace%40example.com", Some(&code)).await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    let session = common::logged_in(&app, "grace@example.com", "correct-horse-battery-9").await;
    // The owner's account filled nothing, and the IP limit is 1: plain resends would
    // stop at the second.
    for _ in 0..3 {
        let resent = app.form("/account/verify-email", &[], Some(&session)).await;
        assert_eq!(resent.status, StatusCode::SEE_OTHER, "{}", resent.body);
    }
}

#[tokio::test]
async fn wrong_codes_count_against_the_ips_signups() {
    let app = test_app().await.with_limits(Limits {
        signups_per_ip: hourly(2),
        ..wide()
    });
    for n in 0..2 {
        let guess = format!("inv-guess{n:011}");
        let reply = sign_up_as(&app, "203.0.113.5", "a%40example.com", Some(&guess)).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
    }
    let more = sign_up_as(
        &app,
        "203.0.113.5",
        "a%40example.com",
        Some("inv-guess99999999999"),
    )
    .await;
    assert_eq!(more.status, StatusCode::TOO_MANY_REQUESTS, "{}", more.body);
}
