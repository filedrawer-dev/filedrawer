//! Seam: the router, for connecting an agent from the browser with a code
//! (design.md, Accounts; #54; RFC 8628): the agent asks for a code with no token,
//! its human types the code on `/device`, logged in, and approves, and the agent's
//! next poll takes the token.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::through_caddy::{hourly, wide};
use common::{PASSWORD, Reply, TestApp, field, test_app};
use filedrawer_server::limits::Limits;

const ADA: &str = "ada@example.com";
const AGENT: &str = "filedrawer-skill (claude-code; claude-opus-5-5)";

/// A form POST as an agent sends it: no Origin, no cookie, no token.
async fn agent_post(app: &TestApp, path: &str, body: String, json: bool) -> Reply {
    let mut request = Request::post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("user-agent", AGENT);
    if json {
        request = request.header("accept", "application/json");
    }
    app.send_from("203.0.113.5:4000", request.body(Body::from(body)).unwrap())
        .await
}

/// What `POST /device/code` gave the agent.
struct Asked {
    user_code: String,
    device_code: String,
}

async fn ask(app: &TestApp, name: &str) -> Asked {
    let reply = agent_post(app, "/device/code", format!("name={name}"), false).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.headers["cache-control"], "no-store");
    let user_code = field(&reply.body, "user_code").expect(&reply.body);
    let device_code = field(&reply.body, "device_code").expect(&reply.body);
    Asked {
        user_code: user_code.to_string(),
        device_code: device_code.to_string(),
    }
}

async fn poll(app: &TestApp, device_code: &str) -> Reply {
    agent_post(
        app,
        "/device/token",
        format!("device_code={device_code}"),
        false,
    )
    .await
}

/// The RFC 8628 error code a refused poll starts its one line with.
fn poll_error(reply: &Reply) -> &str {
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    reply.body.split(':').next().unwrap()
}

/// Types `code` on `/device` and presses `decision` on the page that follows,
/// with the token named `name`.
async fn decide(app: &TestApp, cookie: &str, code: &str, decision: &str, name: &str) -> Reply {
    let typed = app
        .form("/device", &[("user_code", code)], Some(cookie))
        .await;
    assert_eq!(typed.status, StatusCode::OK, "{}", typed.body);
    assert!(typed.has_class("device-confirm"), "{}", typed.body);
    let fields = [("user_code", code), ("decision", decision), ("name", name)];
    app.form("/device/approve", &fields, Some(cookie)).await
}

/// A logged-in, verified browser account: its session cookie and its ID.
async fn person(app: &TestApp) -> (String, String) {
    common::signed_up_and_verified(app, ADA).await;
    let cookie = common::logged_in(app, ADA, PASSWORD).await;
    let id = common::account_id(app, &cookie).await;
    (cookie, id)
}

#[tokio::test]
async fn an_agent_takes_the_token_its_human_approves() {
    let app = test_app().await;
    let asked = ask(&app, "laptop").await;
    // RFC 8628's shape: 8 letters of its base 20, with a dash for reading.
    let code = &asked.user_code;
    assert_eq!(code.len(), 9, "{code}");
    assert_eq!(code.chars().nth(4), Some('-'), "{code}");
    assert!(
        code.chars()
            .filter(|c| *c != '-')
            .all(|c| "BCDFGHJKLMNPQRSTVWXZ".contains(c)),
        "{code}"
    );
    assert!(
        asked.device_code.starts_with("fdd_"),
        "{}",
        asked.device_code
    );
    let pending = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&pending), "authorization_pending");

    let (cookie, id) = person(&app).await;
    let form = app.page("/device", Some(&cookie)).await;
    assert_eq!(form.status, StatusCode::OK, "{}", form.body);
    assert!(form.has_class("device"), "{}", form.body);
    let typed = app
        .form("/device", &[("user_code", code)], Some(&cookie))
        .await;
    // The page shows who asks, as the agent says, and the account it will post as.
    assert!(typed.body.contains(AGENT), "{}", typed.body);
    assert!(typed.body.contains(&id), "{}", typed.body);
    assert!(typed.body.contains("value=\"laptop\""), "{}", typed.body);
    let approved = decide(&app, &cookie, code, "approve", "Ada laptop").await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);

    let taken = poll(&app, &asked.device_code).await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.body);
    assert_eq!(taken.headers["cache-control"], "no-store");
    assert_eq!(taken.headers["x-filedrawer-account"], id.as_str());
    let token = taken
        .body
        .strip_prefix("Authorization: Bearer ")
        .and_then(|rest| rest.strip_suffix('\n'))
        .expect(&taken.body);
    assert!(
        token.starts_with("fdt_") && !token.contains('\n'),
        "{token}"
    );
    let mine = app.get_with_token("/entries/mine", token).await;
    let named = format!("{id} (Grace)");
    assert_eq!(field(&mine.body, "account"), Some(named.as_str()));
    // The token is shown once.
    let again = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&again), "expired_token");
    // The account page lists it under the name approved, marked.
    let account = app.page("/account", Some(&cookie)).await;
    assert!(account.body.contains("Ada laptop"), "{}", account.body);
    assert!(account.has_class("token-from-code"), "{}", account.body);
}

#[tokio::test]
async fn a_token_made_on_the_account_page_isnt_marked() {
    let app = test_app().await;
    let (cookie, _) = person(&app).await;
    common::new_token(&app, &cookie).await;
    let account = app.page("/account", Some(&cookie)).await;
    assert!(!account.has_class("token-from-code"), "{}", account.body);
}

#[tokio::test]
async fn a_poll_sooner_than_the_interval_slows_the_agent_down() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let first = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&first), "authorization_pending");
    let second = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&second), "slow_down");
    // Each slow_down adds 5 s, for every later poll (RFC 8628, section 3.5).
    let json = agent_post(
        &app,
        "/device/token",
        format!("device_code={}", asked.device_code),
        true,
    )
    .await;
    let reply: serde_json::Value = serde_json::from_str(&json.body).unwrap();
    assert_eq!(reply["error"], "slow_down");
    assert_eq!(reply["interval"], 15);
}

#[tokio::test]
async fn the_reply_speaks_rfc_8628s_json_on_request() {
    let app = test_app().await;
    let reply = agent_post(&app, "/device/code", String::new(), true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let asked: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(asked["verification_uri"], "http://localhost/device");
    assert_eq!(asked["expires_in"], 900);
    assert_eq!(asked["interval"], 5);
    let device_code = asked["device_code"].as_str().unwrap();
    let polled = agent_post(
        &app,
        "/device/token",
        format!("device_code={device_code}"),
        true,
    )
    .await;
    assert_eq!(polled.status, StatusCode::BAD_REQUEST, "{}", polled.body);
    let pending: serde_json::Value = serde_json::from_str(&polled.body).unwrap();
    assert_eq!(pending["error"], "authorization_pending");
    assert_eq!(pending["interval"], 5);
}

#[tokio::test]
async fn a_denied_code_makes_no_token() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let (cookie, _) = person(&app).await;
    let denied = decide(&app, &cookie, &asked.user_code, "deny", "agent").await;
    assert_eq!(denied.status, StatusCode::OK, "{}", denied.body);
    let reply = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&reply), "access_denied");
    let after = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&after), "expired_token");
    // Nor can it be approved after.
    let typed = app
        .form("/device", &[("user_code", &asked.user_code)], Some(&cookie))
        .await;
    assert_eq!(typed.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_code_is_read_as_a_person_types_it() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let (cookie, _) = person(&app).await;
    let typed = format!(" {} ", asked.user_code.replace('-', " ").to_lowercase());
    let approved = decide(&app, &cookie, &typed, "approve", "agent").await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    let taken = poll(&app, &asked.device_code).await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.body);
}

#[tokio::test]
async fn the_pages_need_a_session_with_a_verified_address() {
    let app = test_app().await;
    let anonymous = app.page("/device", None).await;
    assert!(anonymous.status.is_redirection(), "{}", anonymous.body);
    assert_eq!(anonymous.location(), Some("/login?next=%2Fdevice"));
    let asked = ask(&app, "").await;
    let posted = app
        .form(
            "/device/approve",
            &[("user_code", &asked.user_code), ("decision", "approve")],
            None,
        )
        .await;
    assert!(posted.status.is_redirection(), "{}", posted.body);
    // Signed up but not verified.
    let signup = app
        .form(
            "/signup",
            &[
                ("email", ADA),
                ("password", PASSWORD),
                ("display_name", "Ada"),
            ],
            None,
        )
        .await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    let cookie = common::logged_in(&app, ADA, PASSWORD).await;
    let page = app.page("/device", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN, "{}", page.body);
    let typed = app
        .form("/device", &[("user_code", &asked.user_code)], Some(&cookie))
        .await;
    assert_eq!(typed.status, StatusCode::FORBIDDEN, "{}", typed.body);
    let polled = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&polled), "authorization_pending");
}

#[tokio::test]
async fn another_sites_form_approves_nothing() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let (cookie, _) = person(&app).await;
    let request = Request::post("/device/approve")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "https://elsewhere.example")
        .header("cookie", &cookie)
        .body(Body::from(format!(
            "user_code={}&decision=approve&name=agent",
            asked.user_code
        )))
        .unwrap();
    let reply = app.send(request).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    let polled = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&polled), "authorization_pending");
}

#[tokio::test]
async fn wrong_codes_are_limited_per_account() {
    let app = test_app().await.with_limits(Limits {
        wrong_device_codes_per_account: hourly(2),
        ..wide()
    });
    let asked = ask(&app, "").await;
    let (cookie, _) = person(&app).await;
    let wrong = if asked.user_code == "BBBB-BBBB" {
        "CCCC-CCCC"
    } else {
        "BBBB-BBBB"
    };
    for _ in 0..2 {
        let reply = app
            .form("/device", &[("user_code", wrong)], Some(&cookie))
            .await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
        assert!(reply.has_class("error"), "{}", reply.body);
    }
    // Past the limit even the right code waits, so guessing gains nothing.
    let right = app
        .form("/device", &[("user_code", &asked.user_code)], Some(&cookie))
        .await;
    assert_eq!(
        right.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        right.body
    );
}

#[tokio::test]
async fn a_right_code_isnt_counted() {
    let app = test_app().await.with_limits(Limits {
        wrong_device_codes_per_account: hourly(1),
        ..wide()
    });
    let (cookie, _) = person(&app).await;
    for _ in 0..3 {
        let asked = ask(&app, "").await;
        let approved = decide(&app, &cookie, &asked.user_code, "approve", "agent").await;
        assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    }
}

#[tokio::test]
async fn device_codes_are_limited_per_ip_and_across_the_site() {
    for limits in [
        Limits {
            device_codes_per_ip: hourly(1),
            ..wide()
        },
        Limits {
            device_codes_site: hourly(1),
            ..wide()
        },
    ] {
        let app = test_app().await.with_limits(limits);
        ask(&app, "").await;
        let refused = agent_post(&app, "/device/code", String::new(), false).await;
        assert_eq!(
            refused.status,
            StatusCode::TOO_MANY_REQUESTS,
            "{}",
            refused.body
        );
    }
}

#[tokio::test]
async fn an_expired_code_works_nowhere() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let (cookie, _) = person(&app).await;
    sqlx::query("UPDATE device_requests SET expires_at = '2000-01-01T00:00:00Z'")
        .execute(&app.pool)
        .await
        .unwrap();
    let typed = app
        .form("/device", &[("user_code", &asked.user_code)], Some(&cookie))
        .await;
    assert_eq!(
        typed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        typed.body
    );
    let polled = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&polled), "expired_token");
}

#[tokio::test]
async fn an_unknown_device_code_reads_as_expired() {
    let app = test_app().await;
    let polled = poll(&app, "fdd_nothing").await;
    assert_eq!(poll_error(&polled), "expired_token");
}

#[tokio::test]
async fn a_password_reset_ends_an_approval_not_taken_yet() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let (cookie, _) = person(&app).await;
    decide(&app, &cookie, &asked.user_code, "approve", "agent").await;
    let reset = app.form("/reset", &[("email", ADA)], None).await;
    assert_eq!(reset.status, StatusCode::OK, "{}", reset.body);
    let link = app.link_in_last_email_to(ADA).await;
    let secret = link.strip_prefix("/reset/confirm?token=").expect(&link);
    let fields = [("token", secret), ("password", "another long password")];
    let done = app.form("/reset/confirm", &fields, None).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let polled = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&polled), "expired_token");
}

#[tokio::test]
async fn deleting_the_account_ends_an_approval_not_taken_yet() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let (cookie, _) = person(&app).await;
    decide(&app, &cookie, &asked.user_code, "approve", "agent").await;
    let deleted = app
        .form("/account/delete", &[("password", PASSWORD)], Some(&cookie))
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    let polled = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&polled), "expired_token");
}

#[tokio::test]
async fn an_expired_request_is_tidied_away() {
    let app = test_app().await;
    let asked = ask(&app, "").await;
    let later = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
    filedrawer_server::device::tidy(&app.pool, later)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_requests")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    let polled = poll(&app, &asked.device_code).await;
    assert_eq!(poll_error(&polled), "expired_token");
}
