//! Seam: the router, for accounts in the browser (design.md, Accounts): signup,
//! verification, login, sessions, password reset, and tokens.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{PASSWORD, PUBLIC_URL, entry, logged_in, new_token, signed_up_and_verified, test_app};

/// Whether account page `page` offers to send the verification email again, as it
/// does only while the address is unverified.
fn offers_verification(page: &str) -> bool {
    assert!(
        page.contains("action=\"/logout\""),
        "not the account page: {page}"
    );
    page.contains("action=\"/account/verify-email\"")
}

/// The text of a form page's error paragraph, or "" when it has none.
fn error_on(page: &str) -> &str {
    page.split_once(r#"<p class="error">"#)
        .and_then(|(_, rest)| rest.split_once("</p>"))
        .map_or("", |(error, _)| error)
}

#[tokio::test]
async fn signup_verify_login_and_submit_with_a_new_token() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let page = app.page("/account", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Grace"), "{}", page.body);
    assert!(page.body.contains("grace@example.com"), "{}", page.body);
    assert!(!offers_verification(&page.body), "{}", page.body);
    let token = new_token(&app, &cookie).await;
    let reply = app.post_as("/entries", Some(&token), entry("T", "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert!(reply.body.contains("Grace, account a-"), "{}", reply.body);
}

#[tokio::test]
async fn the_session_cookie_is_http_only_secure_and_strict() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let reply = app
        .form(
            "/login",
            &[("email", "grace@example.com"), ("password", PASSWORD)],
            None,
        )
        .await;
    let header = reply.headers["set-cookie"].to_str().unwrap();
    for attribute in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
        assert!(header.contains(attribute), "{header}");
    }
}

#[tokio::test]
async fn the_verification_link_opens_a_page_and_only_its_post_verifies() {
    let app = test_app().await;
    app.form(
        "/signup",
        &[
            ("email", "grace@example.com"),
            ("password", PASSWORD),
            ("display_name", "Grace"),
        ],
        None,
    )
    .await;
    let link = app.link_in_last_email_to("grace@example.com").await;
    assert!(
        app.sent().await[0]
            .body
            .contains(&format!("{PUBLIC_URL}/verify?token="))
    );
    let page = app.page(&link, None).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body
            .contains("<form method=\"post\" action=\"/verify\">"),
        "{}",
        page.body
    );
    // Opening the link verified nothing: the account still can't make a token.
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let account = app.page("/account", Some(&cookie)).await.body;
    assert!(offers_verification(&account), "{account}");
    let reply = app
        .form("/account/tokens", &[("name", "agent")], Some(&cookie))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
}

#[tokio::test]
async fn a_token_on_an_account_whose_email_isnt_verified_cant_submit() {
    // Accounts get tokens only once verified, so this guards the gate in the submit
    // path itself: a token made by the admin CLI for an account that signed up.
    let app = test_app().await;
    app.form(
        "/signup",
        &[
            ("email", "grace@example.com"),
            ("password", PASSWORD),
            ("display_name", "Grace"),
        ],
        None,
    )
    .await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let account = common::account_id(&app, &cookie).await;
    let token = filedrawer_server::admin::create_token(&app.pool, &account, "cli")
        .await
        .unwrap();
    let reply = app.post_as("/entries", Some(&token), entry("T", "")).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    assert!(reply.body.contains("verify"), "{}", reply.body);
}

#[tokio::test]
async fn a_verification_link_works_once() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let token = app
        .link_in_last_email_to("grace@example.com")
        .await
        .replace("/verify?token=", "");
    let again = app.form("/verify", &[("token", &token)], None).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert!(again.body.contains("used or expired"), "{}", again.body);
}

#[tokio::test]
async fn signup_checks_its_fields() {
    let app = test_app().await;
    for (email, password, name, complaint) in [
        ("not-an-email", PASSWORD, "Grace", "email"),
        ("grace@example.com", "short", "Grace", "12"),
        ("grace@example.com", PASSWORD, "", "display name"),
        (
            "grace@example.com",
            PASSWORD,
            &"x".repeat(51),
            "display name",
        ),
    ] {
        let reply = app
            .form(
                "/signup",
                &[
                    ("email", email),
                    ("password", password),
                    ("display_name", name),
                ],
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{email}");
        assert!(
            error_on(&reply.body).contains(complaint),
            "{complaint}: {}",
            reply.body
        );
    }
    assert!(app.sent().await.is_empty());
}

#[tokio::test]
async fn signing_up_with_a_taken_email_looks_the_same_and_warns_its_owner() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let first = app.sent().await.len();
    let reply = app
        .form(
            "/signup",
            &[
                ("email", " Grace@Example.com "),
                ("password", "another long password"),
                ("display_name", "Mallory"),
            ],
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains("check your email"), "{}", reply.body);
    let warning = &app.sent().await[first];
    assert_eq!(warning.to, "grace@example.com");
    assert!(
        warning.body.contains("tried to sign up"),
        "{}",
        warning.body
    );
    // The first password still works; the second never took.
    logged_in(&app, "grace@example.com", PASSWORD).await;
    let wrong = app
        .form(
            "/login",
            &[
                ("email", "grace@example.com"),
                ("password", "another long password"),
            ],
            None,
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_email_get_the_same_answer() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let wrong = app
        .form(
            "/login",
            &[
                ("email", "grace@example.com"),
                ("password", "not the password"),
            ],
            None,
        )
        .await;
    let unknown = app
        .form(
            "/login",
            &[
                ("email", "nobody@example.com"),
                ("password", "not the password"),
            ],
            None,
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.body, unknown.body);
    assert!(wrong.cookie().is_none());
}

#[tokio::test]
async fn the_account_page_needs_a_session() {
    let app = test_app().await;
    let reply = app.page("/account", None).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), Some("/login"));
    let forged = app.page("/account", Some("fd_session=forged")).await;
    assert_eq!(forged.location(), Some("/login"));
}

#[tokio::test]
async fn logout_ends_the_session() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let reply = app.form("/logout", &[], Some(&cookie)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(
        app.page("/account", Some(&cookie)).await.location(),
        Some("/login")
    );
}

#[tokio::test]
async fn a_revoked_token_stops_working() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let token = new_token(&app, &cookie).await;
    let page = app.page("/account", Some(&cookie)).await.body;
    let marker = "action=\"/account/tokens/";
    let start = page.find(marker).expect(&page) + marker.len();
    let id: String = page[start..].chars().take_while(|c| *c != '/').collect();
    let reply = app
        .form(&format!("/account/tokens/{id}/revoke"), &[], Some(&cookie))
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    let refused = app.post_as("/entries", Some(&token), entry("T", "")).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_token_can_neither_open_the_account_page_nor_make_tokens() {
    let app = test_app().await;
    let reply = app
        .send(
            Request::post("/account/tokens")
                .header("authorization", format!("Bearer {}", app.token))
                .header("origin", PUBLIC_URL)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("name=sneaky"))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), Some("/login"));
    assert!(!reply.body.contains("fdt_"));
}

#[tokio::test]
async fn a_form_post_from_another_origin_or_none_is_refused() {
    let app = test_app().await;
    for origin in [Some("https://evil.example"), None] {
        let mut request =
            Request::post("/login").header("content-type", "application/x-www-form-urlencoded");
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let reply = app
            .send(
                request
                    .body(Body::from("email=a%40b.c&password=x"))
                    .unwrap(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{origin:?}");
    }
}

#[tokio::test]
async fn password_reset_sets_a_new_password_and_ends_old_sessions() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let old_session = logged_in(&app, "grace@example.com", PASSWORD).await;
    let reply = app
        .form("/reset", &[("email", "grace@example.com")], None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let link = app.link_in_last_email_to("grace@example.com").await;
    let token = link.strip_prefix("/reset/confirm?token=").expect(&link);
    let form = app.page(&link, None).await;
    assert!(
        form.body
            .contains("<form method=\"post\" action=\"/reset/confirm\">"),
        "{}",
        form.body
    );
    let new_password = "a brand new passphrase";
    let done = app
        .form(
            "/reset/confirm",
            &[("token", token), ("password", new_password)],
            None,
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    logged_in(&app, "grace@example.com", new_password).await;
    let old = app
        .form(
            "/login",
            &[("email", "grace@example.com"), ("password", PASSWORD)],
            None,
        )
        .await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        app.page("/account", Some(&old_session)).await.location(),
        Some("/login")
    );
    let again = app
        .form(
            "/reset/confirm",
            &[("token", token), ("password", "yet another passphrase")],
            None,
        )
        .await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_reset_for_an_unknown_email_looks_the_same_and_sends_nothing() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let known = app
        .form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let sent = app.sent().await.len();
    let unknown = app
        .form("/reset", &[("email", "nobody@example.com")], None)
        .await;
    assert_eq!(known.status, unknown.status);
    assert_eq!(known.body, unknown.body);
    assert_eq!(app.sent().await.len(), sent);
}

#[tokio::test]
async fn an_unverified_account_can_ask_for_another_verification_email() {
    let app = test_app().await;
    app.form(
        "/signup",
        &[
            ("email", "grace@example.com"),
            ("password", PASSWORD),
            ("display_name", "Grace"),
        ],
        None,
    )
    .await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let page = app.page("/account", Some(&cookie)).await.body;
    assert!(offers_verification(&page), "{page}");
    let reply = app.form("/account/verify-email", &[], Some(&cookie)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    assert_eq!(app.sent().await.len(), 2);
    let token = app
        .link_in_last_email_to("grace@example.com")
        .await
        .replace("/verify?token=", "");
    assert_eq!(
        app.form("/verify", &[("token", &token)], None).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn pages_escape_what_users_write() {
    let app = test_app().await;
    app.form(
        "/signup",
        &[
            ("email", "grace@example.com"),
            ("password", PASSWORD),
            ("display_name", "<script>x</script>"),
        ],
        None,
    )
    .await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let page = app.page("/account", Some(&cookie)).await.body;
    assert!(!page.contains("<script>x"), "{page}");
    // askama writes numeric entities; either form is escaped.
    assert!(
        page.contains("&#60;script&#62;x") || page.contains("&lt;script&gt;x"),
        "{page}"
    );
}

#[tokio::test]
async fn an_unverified_account_cant_make_tokens() {
    let app = test_app().await;
    app.form(
        "/signup",
        &[
            ("email", "grace@example.com"),
            ("password", PASSWORD),
            ("display_name", "Grace"),
        ],
        None,
    )
    .await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let reply = app
        .form("/account/tokens", &[("name", "agent")], Some(&cookie))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    assert!(!reply.body.contains("fdt_"));
}

#[tokio::test]
async fn whoever_resets_an_address_gets_the_account_with_no_tokens_left() {
    // Mallory signs up with Grace's address before Grace does and can't verify it.
    // Grace resets the password through her own inbox: she owns the account now,
    // and nothing Mallory made with it still works.
    let app = test_app().await;
    app.form(
        "/signup",
        &[
            ("email", "grace@example.com"),
            ("password", "mallory's passphrase"),
            ("display_name", "Mallory"),
        ],
        None,
    )
    .await;
    let mallory = logged_in(&app, "grace@example.com", "mallory's passphrase").await;
    app.form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let token = app
        .link_in_last_email_to("grace@example.com")
        .await
        .replace("/reset/confirm?token=", "");
    let done = app
        .form(
            "/reset/confirm",
            &[("token", &token), ("password", PASSWORD)],
            None,
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(
        app.page("/account", Some(&mallory)).await.location(),
        Some("/login")
    );
    // The reset proved the address, so Grace can make a token and submit.
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let page = app.page("/account", Some(&cookie)).await.body;
    assert!(!offers_verification(&page), "{page}");
    let grace = new_token(&app, &cookie).await;
    let reply = app.post_as("/entries", Some(&grace), entry("T", "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[tokio::test]
async fn a_reset_spends_every_reset_link_the_account_has() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    app.form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let older = app
        .link_in_last_email_to("grace@example.com")
        .await
        .replace("/reset/confirm?token=", "");
    app.form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let newer = app
        .link_in_last_email_to("grace@example.com")
        .await
        .replace("/reset/confirm?token=", "");
    let done = app
        .form(
            "/reset/confirm",
            &[("token", &newer), ("password", "a brand new passphrase")],
            None,
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);
    let stale = app
        .form(
            "/reset/confirm",
            &[("token", &older), ("password", "an attacker's passphrase")],
            None,
        )
        .await;
    assert_eq!(stale.status, StatusCode::BAD_REQUEST, "{}", stale.body);
}

#[tokio::test]
async fn the_signup_login_and_reset_pages_render_their_forms() {
    let app = test_app().await;
    for (path, action) in [
        ("/signup", "/signup"),
        ("/login", "/login"),
        ("/reset", "/reset"),
    ] {
        let reply = app.page(path, None).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert!(
            reply
                .body
                .contains(&format!("<form method=\"post\" action=\"{action}\">")),
            "{path}: {}",
            reply.body
        );
    }
}

#[tokio::test]
async fn every_account_form_needs_this_sites_origin() {
    let app = test_app().await;
    for path in [
        "/signup",
        "/verify",
        "/login",
        "/logout",
        "/reset",
        "/reset/confirm",
        "/account/tokens",
        "/account/tokens/x/revoke",
        "/account/verify-email",
    ] {
        let reply = app
            .send(
                Request::post(path)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("origin", "https://evil.example")
                    .body(Body::from("a=b"))
                    .unwrap(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
    }
}
