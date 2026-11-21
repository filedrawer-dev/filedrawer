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
async fn the_session_cookie_is_http_only_secure_and_lax() {
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
    // Lax, so a link from an email to an admin or account page keeps the session.
    for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(header.contains(attribute), "{header}");
    }
}

#[tokio::test]
async fn a_page_opened_without_a_session_sends_the_login_back_to_it() {
    let app = test_app().await;
    let reply = app.get("/admin/e/3h1kzn8sj4?x=1").await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(
        reply.location(),
        Some("/login?next=%2Fadmin%2Fe%2F3h1kzn8sj4%3Fx%3D1")
    );
    let login = app.get(reply.location().unwrap()).await;
    assert!(
        login
            .body
            .contains(r#"name="next" value="/admin/e/3h1kzn8sj4?x=1""#),
        "{}",
        login.body
    );
}

#[tokio::test]
async fn login_lands_on_the_page_it_was_sent_from() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let reply = app
        .form(
            "/login",
            &[
                ("email", "grace@example.com"),
                ("password", PASSWORD),
                ("next", "/account/delete"),
            ],
            None,
        )
        .await;
    assert_eq!(reply.location(), Some("/account/delete"));
}

#[tokio::test]
async fn login_never_sends_a_browser_off_the_site() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    for next in [
        "//evil.example/",
        "https://evil.example/",
        "/\\evil.example/",
        "/\t/evil.example/",
        "evil",
        "/é",
        "",
    ] {
        let reply = app
            .form(
                "/login",
                &[
                    ("email", "grace@example.com"),
                    ("password", PASSWORD),
                    ("next", next),
                ],
                None,
            )
            .await;
        assert_eq!(reply.location(), Some("/account"), "{next:?}");
    }
}

#[tokio::test]
async fn a_failed_login_keeps_the_page_to_return_to() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let reply = app
        .form(
            "/login",
            &[
                ("email", "grace@example.com"),
                ("password", "wrong password"),
                ("next", "/admin"),
            ],
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(
        reply.body.contains(r#"name="next" value="/admin""#),
        "{}",
        reply.body
    );
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
async fn the_check_your_email_page_says_to_log_in_to_get_the_link_again() {
    // Logging in reaches the account page's resend button, so the page says how to
    // get another link, and says it whether or not the address was taken.
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let mut pages = Vec::new();
    for email in ["ada@example.com", "grace@example.com"] {
        let reply = app
            .form(
                "/signup",
                &[
                    ("email", email),
                    ("password", PASSWORD),
                    ("display_name", "Someone"),
                ],
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply.body.contains(r#"<a href="/login">"#),
            "{}",
            reply.body
        );
        pages.push(reply.body.replace(email, "ADDRESS"));
    }
    assert_eq!(pages[0], pages[1]);
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
    assert_eq!(reply.location(), Some("/login?next=%2Faccount"));
    let forged = app.page("/account", Some("fd_session=forged")).await;
    assert_eq!(forged.location(), Some("/login?next=%2Faccount"));
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
        Some("/login?next=%2Faccount")
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
        Some("/login?next=%2Faccount")
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
    // The signup's email goes before the account asks again, as the sender would.
    assert_eq!(app.sent().await.len(), 1);
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
        Some("/login?next=%2Faccount")
    );
    // The reset proved the address, so Grace can make a token and submit.
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let page = app.page("/account", Some(&cookie)).await.body;
    assert!(!offers_verification(&page), "{page}");
    let grace = new_token(&app, &cookie).await;
    let reply = app.post_as("/entries", Some(&grace), entry("T", "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

/// Signs up as Mallory with `email`, which is Grace's, and logs in before anyone
/// verified it. Returns Mallory's session cookie.
async fn mallory_signs_up_with(app: &common::TestApp, email: &str) -> String {
    app.form(
        "/signup",
        &[
            ("email", email),
            ("password", "mallory's passphrase"),
            ("display_name", "Mallory"),
        ],
        None,
    )
    .await;
    logged_in(app, email, "mallory's passphrase").await
}

/// Presses the button of verification link `link`, as its email gave it.
async fn press_verify(app: &common::TestApp, link: &str) -> common::Reply {
    let token = link.strip_prefix("/verify?token=").expect(link);
    app.form("/verify", &[("token", token)], None).await
}

/// Resets `email`'s password to `PASSWORD` through the link sent to it.
async fn reset_password(app: &common::TestApp, email: &str) -> common::Reply {
    app.form("/reset", &[("email", email)], None).await;
    let token = app
        .link_in_last_email_to(email)
        .await
        .replace("/reset/confirm?token=", "");
    app.form(
        "/reset/confirm",
        &[("token", &token), ("password", PASSWORD)],
        None,
    )
    .await
}

#[tokio::test]
async fn verifying_an_address_ends_the_sessions_made_before_it_was_proved() {
    let app = test_app().await;
    let before = mallory_signs_up_with(&app, "grace@example.com").await;
    let link = app.link_in_last_email_to("grace@example.com").await;
    let verified = press_verify(&app, &link).await;
    assert_eq!(verified.status, StatusCode::OK, "{}", verified.body);
    assert_eq!(
        app.page("/account", Some(&before)).await.location(),
        Some("/login?next=%2Faccount")
    );
}

#[tokio::test]
async fn a_verification_link_pressed_after_the_address_was_proved_ends_no_session() {
    let app = test_app().await;
    let first = mallory_signs_up_with(&app, "grace@example.com").await;
    let older = app.link_in_last_email_to("grace@example.com").await;
    // A second link, from the account page, while the first still works.
    app.form("/account/verify-email", &[], Some(&first)).await;
    let newer = app.link_in_last_email_to("grace@example.com").await;
    assert_ne!(older, newer);
    press_verify(&app, &newer).await;
    let after = logged_in(&app, "grace@example.com", "mallory's passphrase").await;
    press_verify(&app, &older).await;
    let page = app.page("/account", Some(&after)).await;
    assert_eq!(page.status, StatusCode::OK, "{:?}", page.location());
}

#[tokio::test]
async fn a_reset_revokes_every_token_the_account_has() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let token = new_token(&app, &cookie).await;
    let reply = app.get_with_token("/entries/mine", &token).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let done = reset_password(&app, "grace@example.com").await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert!(
        done.body.contains("API tokens are revoked"),
        "{}",
        done.body
    );
    let reply = app.get_with_token("/entries/mine", &token).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
    // A token made after the reset works.
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let again = new_token(&app, &cookie).await;
    let reply = app.get_with_token("/entries/mine", &again).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn nothing_made_before_an_address_was_proved_outlives_its_owners_reset() {
    // Mallory signs up with Grace's address and logs in. Grace presses the button in
    // the verification email, and later resets the password through her inbox. As
    // when she never verified, nothing Mallory made with the account still works.
    let app = test_app().await;
    mallory_signs_up_with(&app, "grace@example.com").await;
    let link = app.link_in_last_email_to("grace@example.com").await;
    press_verify(&app, &link).await;
    let mallory = logged_in(&app, "grace@example.com", "mallory's passphrase").await;
    let mallorys_token = new_token(&app, &mallory).await;
    let done = reset_password(&app, "grace@example.com").await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(
        app.page("/account", Some(&mallory)).await.location(),
        Some("/login?next=%2Faccount")
    );
    let reply = app.get_with_token("/entries/mine", &mallorys_token).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
    let relogin = app
        .form(
            "/login",
            &[
                ("email", "grace@example.com"),
                ("password", "mallory's passphrase"),
            ],
            None,
        )
        .await;
    assert_eq!(relogin.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_reset_email_and_its_form_say_what_a_new_password_ends() {
    let app = test_app().await;
    signed_up_and_verified(&app, "grace@example.com").await;
    app.form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let sent = app.sent().await;
    let email = sent.last().expect("the reset email");
    assert!(email.body.contains("API tokens"), "{}", email.body);
    let link = app.link_in_last_email_to("grace@example.com").await;
    let form = app.page(&link, None).await;
    assert_eq!(form.status, StatusCode::OK, "{}", form.body);
    assert!(form.has_class("reset-ends"), "{}", form.body);
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

/// The display name stands in the `author:` line above the delimiter and on the
/// card's last line, so it is a label on one line that holds nothing invisible.
#[tokio::test]
async fn signup_refuses_a_display_name_with_invisible_characters() {
    let app = test_app().await;
    for name in [
        "Ada\u{e0061}\u{e0062}",
        "Ada\u{200b}",
        "Ada\u{202e}",
        "Ada\u{2028}Lovelace",
        "\u{3164}",
        "\u{fe0f}",
        "Ada\u{200c}",
        "A\u{200d}da",
    ] {
        let reply = app
            .form(
                "/signup",
                &[
                    ("email", "mallory@example.com"),
                    ("password", PASSWORD),
                    ("display_name", name),
                ],
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{name:?}");
        assert!(
            error_on(&reply.body).contains("invisible characters"),
            "{name:?}: {}",
            reply.body
        );
    }
    assert!(app.sent().await.is_empty());
}

/// Persian names hold a zero-width non-joiner inside a word, and emoji sequences a
/// zero-width joiner: neither hides anything, so signup takes them.
#[tokio::test]
async fn signup_takes_a_name_with_joiners_between_letters() {
    let app = test_app().await;
    for (email, name) in [
        ("mina@example.com", "\u{645}\u{6cc}\u{200c}\u{646}\u{627}"),
        ("dev@example.com", "Ada \u{1f469}\u{200d}\u{1f4bb}"),
    ] {
        let reply = app
            .form(
                "/signup",
                &[
                    ("email", email),
                    ("password", PASSWORD),
                    ("display_name", name),
                ],
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{name:?}: {}", reply.body);
    }
    assert_eq!(app.sent().await.len(), 2);
}

/// A name stored before signup refused invisible characters is printed without
/// them, and one that then shows nothing as the server's "(no name)".
#[tokio::test]
async fn a_stored_display_name_is_printed_without_its_invisible_characters() {
    let app = test_app().await;
    let hidden = "\u{e0061}\u{e0062}\u{200b}\u{202e}";
    for (stored, shown) in [
        (format!("Ada{hidden} Lovelace"), "Ada Lovelace"),
        (hidden.to_string(), "(no name)"),
    ] {
        sqlx::query("UPDATE accounts SET display_name = ? WHERE id = ?")
            .bind(&stored)
            .bind(&app.account)
            .execute(&app.pool)
            .await
            .unwrap();
        let reply = app.post("/entries", entry(shown, "")).await;
        let account = &app.account;
        assert!(
            reply
                .body
                .contains(&format!("account: {account} ({shown})\n")),
            "{}",
            reply.body
        );
        let id = common::id_in(&reply.body);
        app.post(&format!("/e/{id}/publish"), "").await;
        let body = app.get(&format!("/e/{id}")).await.body;
        let (server, _) = body.split_once(common::DELIMITER).expect(&body);
        assert!(
            server.contains(&format!("author: {shown}, account {account}\n")),
            "{server:?}"
        );
        let cards = app.get("/cards.txt").await.body;
        assert!(
            cards.contains(&format!("\n{shown}, account {account}\n")),
            "{cards:?}"
        );
        assert!(!server.contains('\u{e0061}'), "{server:?}");
        assert!(!cards.contains('\u{e0061}'), "{cards:?}");
        let made = app
            .post("/series", common::series::series_doc(shown, "About it."))
            .await;
        let series = app
            .get_as_author(&format!("/series/{}", common::id_in(&made.body)))
            .await
            .body;
        assert!(
            series.contains(&format!("member: {account}, by {shown}\n")),
            "{series:?}"
        );
    }
}

/// Signs up with `email` and returns the reply.
async fn sign_up_with(app: &common::TestApp, email: &str) -> common::Reply {
    app.form(
        "/signup",
        &[
            ("email", email),
            ("password", PASSWORD),
            ("display_name", "Ada"),
        ],
        None,
    )
    .await
}

/// An address is `local@domain` alone, in ASCII, so the address stored, the one the
/// limits count and the one mail goes to are the same string.
#[tokio::test]
async fn signup_takes_only_a_bare_address() {
    let app = test_app().await;
    for email in [
        "x<ada@example.com>",
        "<ada@example.com>",
        "\"ada\"@example.com",
        "\"x\"<ada@example.com>",
        "=?utf-8?q?x?=<ada@example.com>",
        "(x)ada@example.com",
        "ada@example.com(x)",
        "x:ada@example.com;",
        "ada@example.com,grace@example.com",
        "ada\\@example.com",
        "ada@[192.0.2.1]",
        "ada@ex\u{e4}mple.com",
        "\u{e4}da@example.com",
        // The Kelvin sign, whose lower case is an ASCII k.
        "\u{212a}ate@example.com",
        "a..b@example.com",
        ".ada@example.com",
        "ada.@example.com",
        "ada@-example.com",
        "ada@example-.com",
        "ada@example..com",
        "ada@example_x.com",
    ] {
        let reply = sign_up_with(&app, email).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{email}: {}",
            reply.body
        );
        assert!(error_on(&reply.body).contains("email address"), "{email}");
    }
    assert!(app.sent().await.is_empty());
}

#[tokio::test]
async fn signup_takes_the_addresses_people_have() {
    let app = test_app().await;
    for email in [
        "ada+tag@example.com",
        "o'brien@sub.example.co.uk",
        "a.b-c_d@xn--exmple-cua.com",
        "Ada.Lovelace@Example.COM",
        "1@2.example",
    ] {
        let reply = sign_up_with(&app, email).await;
        assert_eq!(reply.status, StatusCode::OK, "{email}: {}", reply.body);
    }
    let to: Vec<String> = app.sent().await.into_iter().map(|email| email.to).collect();
    assert_eq!(
        to,
        [
            "ada+tag@example.com",
            "o'brien@sub.example.co.uk",
            "a.b-c_d@xn--exmple-cua.com",
            "ada.lovelace@example.com",
            "1@2.example",
        ]
    );
}
