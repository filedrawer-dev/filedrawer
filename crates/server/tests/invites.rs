//! Seam: the router, for invite codes (design.md, Limits and abuse): the owner
//! creates them, an account signs up with one, and the code lifts the per-IP signup
//! and email limits and the new-account allowance.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::through_caddy::{SIGNUP_PASSWORD, hourly, sign_up_from, wide};
use common::{Reply, TestApp, date_in, entry, field, id_in, owner, test_app};
use filedrawer_server::limits::Limits;

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
    let plain = sign_up_from(&app, "203.0.113.5", "q@example.com", None).await;
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.body);
    for n in 0..4 {
        let reply = sign_up_from(
            &app,
            "203.0.113.5",
            &format!("p{n}@example.com"),
            Some(&code),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
    // Without the code, the same IP is past its limit.
    let again = sign_up_from(&app, "203.0.113.5", "r@example.com", None).await;
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
    let first = sign_up_from(&app, "203.0.113.5", "p@example.com", Some(&code)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let again = sign_up_from(&app, "203.0.113.6", "p@example.com", Some(&code)).await;
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
        sign_up_from(&app, "203.0.113.5", "a@example.com", Some(&two))
            .await
            .status,
        StatusCode::OK
    );
    // A signup to a taken address takes no use, so a second account still fits.
    assert_eq!(
        sign_up_from(&app, "203.0.113.5", "a@example.com", Some(&two))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        sign_up_from(&app, "203.0.113.5", "e@example.com", Some(&two))
            .await
            .status,
        StatusCode::OK
    );
    // A used-up code refuses every signup, a taken address's too, so the answer
    // never says whether an address has an account.
    let taken = sign_up_from(&app, "203.0.113.5", "a@example.com", Some(&two)).await;
    assert_eq!(
        taken.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        taken.body
    );
    let used_up = sign_up_from(&app, "203.0.113.5", "b@example.com", Some(&two)).await;
    assert_eq!(
        used_up.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        used_up.body
    );
    assert!(used_up.body.contains("invite code"), "{}", used_up.body);
    let unknown = sign_up_from(
        &app,
        "203.0.113.5",
        "c@example.com",
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
    let late = sign_up_from(&app, "203.0.113.5", "d@example.com", Some(&expired)).await;
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
    let signup = sign_up_from(&app, "203.0.113.5", "grace@example.com", Some(&code)).await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    let link = app.link_in_last_email_to("grace@example.com").await;
    let token = link.strip_prefix("/verify?token=").unwrap();
    app.form("/verify", &[("token", token)], None).await;
    let session = common::logged_in(&app, "grace@example.com", SIGNUP_PASSWORD).await;
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
    let signup = sign_up_from(&app, "203.0.113.5", "grace@example.com", Some(&code)).await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    let session = common::logged_in(&app, "grace@example.com", SIGNUP_PASSWORD).await;
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
        let reply = sign_up_from(&app, "203.0.113.5", "a@example.com", Some(&guess)).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
    }
    let more = sign_up_from(
        &app,
        "203.0.113.5",
        "a@example.com",
        Some("inv-guess99999999999"),
    )
    .await;
    assert_eq!(more.status, StatusCode::TOO_MANY_REQUESTS, "{}", more.body);
}

// Revoking a code.

/// An invited, verified account: its session and a token.
async fn invited_account(app: &TestApp, code: &str, email: &str) -> (String, String) {
    let signup = sign_up_from(app, "203.0.113.5", email, Some(code)).await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    let link = app.link_in_last_email_to(email).await;
    let token = link.strip_prefix("/verify?token=").unwrap();
    app.form("/verify", &[("token", token)], None).await;
    let session = common::logged_in(app, email, SIGNUP_PASSWORD).await;
    let token = common::new_token(app, &session).await;
    (session, token)
}

async fn revoke(app: &TestApp, cookie: &str, code: &str) -> Reply {
    app.form(&format!("/admin/invites/{code}/revoke"), &[], Some(cookie))
        .await
}

async fn status_of(app: &TestApp, token: &str, id: &str) -> String {
    common::author_status(app, token, id)
        .await
        .expect("the author's view gives a status")
}

#[tokio::test]
async fn revoking_freezes_the_codes_unscreened_entries_and_nothing_else() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Leaked", "10", "7").await;
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    let post = async |title: &str| {
        let reply = app
            .post_as("/entries", Some(&token), entry(title, ""))
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        id_in(&reply.body)
    };
    let public = post("Steering public").await;
    app.post_as(&format!("/e/{public}/publish"), Some(&token), "")
        .await;
    assert_eq!(
        app.get(&format!("/e/{public}")).await.status,
        StatusCode::OK
    );
    let pending = post("Steering pending").await;
    // Another account's entry isn't touched.
    let theirs = app.post("/entries", entry("Steering theirs", "")).await;
    let theirs = id_in(&theirs.body);

    let revoked = revoke(&app, &cookie, &code).await;
    assert_eq!(revoked.status, StatusCode::SEE_OTHER, "{}", revoked.body);
    assert_eq!(status_of(&app, &token, &pending).await, "frozen");
    assert_eq!(
        app.get(&format!("/e/{public}")).await.status,
        StatusCode::OK
    );
    assert_eq!(status_of(&app, &app.token, &theirs).await, "pending");
    // The worker leaves it alone, and the code takes no more signups.
    filedrawer_server::jobs::publish_due(
        &app.state,
        time::OffsetDateTime::now_utc() + time::Duration::hours(2),
    )
    .await
    .unwrap();
    assert_eq!(status_of(&app, &token, &pending).await, "frozen");
    let late = sign_up_from(&app, "203.0.113.5", "late@example.com", Some(&code)).await;
    assert_eq!(
        late.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        late.body
    );
    let page = app.page("/admin/invites", Some(&cookie)).await;
    assert!(page.body.contains(", revoked 2"), "{}", page.body);
    let queue = app.page("/admin", Some(&cookie)).await;
    assert!(queue.body.contains("Steering pending"), "{}", queue.body);
}

#[tokio::test]
async fn a_revoked_codes_accounts_fall_back_to_the_new_account_limit() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Leaked", "10", "7").await;
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    revoke(&app, &cookie, &code).await;
    for n in 0..20 {
        let reply = app
            .post_as(
                "/entries",
                Some(&token),
                entry(&format!("Steering {n}"), ""),
            )
            .await;
        assert_eq!(
            field(&reply.body, "screen_from"),
            None,
            "entry {n}: {}",
            reply.body
        );
    }
    let reply = app
        .post_as("/entries", Some(&token), entry("Steering 21", ""))
        .await;
    assert_eq!(
        field(&reply.body, "screen_from"),
        Some(date_in(1).as_str()),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn the_owner_releases_a_frozen_entry_to_screening_or_rejects_it_without_a_guard() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Leaked", "10", "7").await;
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    let mut ids = Vec::new();
    for title in ["Steering one", "Steering two"] {
        let reply = app
            .post_as("/entries", Some(&token), entry(title, ""))
            .await;
        ids.push(id_in(&reply.body));
    }
    revoke(&app, &cookie, &code).await;
    let page = app
        .page(&format!("/admin/e/{}", ids[0]), Some(&cookie))
        .await;
    assert!(page.body.contains("/release"), "{}", page.body);

    let released = app
        .form(&format!("/admin/e/{}/release", ids[0]), &[], Some(&cookie))
        .await;
    assert_eq!(released.status, StatusCode::SEE_OTHER, "{}", released.body);
    filedrawer_server::jobs::publish_due(&app.state, time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(status_of(&app, &token, &ids[0]).await, "public");

    let rejected = app
        .form(&format!("/admin/e/{}/reject", ids[1]), &[], Some(&cookie))
        .await;
    assert_eq!(rejected.status, StatusCode::SEE_OTHER, "{}", rejected.body);
    filedrawer_server::jobs::publish_due(&app.state, time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(status_of(&app, &token, &ids[1]).await, "rejected");
    // No guard: the account's next entry is screened as usual.
    let queue = app.page("/admin", Some(&cookie)).await;
    assert!(!queue.body.contains("clear-guard"), "{}", queue.body);
    // Only a frozen entry is released.
    let again = app
        .form(&format!("/admin/e/{}/release", ids[0]), &[], Some(&cookie))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.body);
}

#[tokio::test]
async fn only_the_owner_revokes_and_releases() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Sprint", "10", "7").await;
    let (other, _) = common::browser_account(&app, "mallory@example.com").await;
    assert_eq!(
        revoke(&app, &other, &code).await.status,
        StatusCode::NOT_FOUND
    );
    let page = app.page("/admin/invites", Some(&cookie)).await;
    assert!(!page.body.contains(", revoked 2"), "{}", page.body);
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    let reply = app
        .post_as("/entries", Some(&token), entry("Steering one", ""))
        .await;
    let id = id_in(&reply.body);
    revoke(&app, &cookie, &code).await;
    let released = app
        .form(&format!("/admin/e/{id}/release"), &[], Some(&other))
        .await;
    assert_eq!(released.status, StatusCode::NOT_FOUND);
    assert_eq!(status_of(&app, &token, &id).await, "frozen");
}

#[tokio::test]
async fn revoking_an_unknown_or_revoked_code_is_not_found() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    assert_eq!(
        revoke(&app, &cookie, "inv-nosuchcode00000").await.status,
        StatusCode::NOT_FOUND
    );
    let code = create_code(&app, &cookie, "Sprint", "10", "7").await;
    assert_eq!(
        revoke(&app, &cookie, &code).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        revoke(&app, &cookie, &code).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn an_entry_waiting_in_screening_freezes_and_keeps_its_day_when_released() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Leaked", "10", "7").await;
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    // The 51st entry of an invited account's day waits in screening for tomorrow.
    for n in 0..50 {
        app.post_as(
            "/entries",
            Some(&token),
            entry(&format!("Steering {n}"), ""),
        )
        .await;
    }
    let late = app
        .post_as("/entries", Some(&token), entry("Steering late", ""))
        .await;
    let late = id_in(&late.body);
    app.post_as(&format!("/e/{late}/publish"), Some(&token), "")
        .await;
    assert_eq!(status_of(&app, &token, &late).await, "screening");
    revoke(&app, &cookie, &code).await;
    assert_eq!(status_of(&app, &token, &late).await, "frozen");
    app.form(&format!("/admin/e/{late}/release"), &[], Some(&cookie))
        .await;
    let own = app
        .send(
            Request::get(format!("/e/{late}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        field(&own.body, "status"),
        Some("screening"),
        "{}",
        own.body
    );
    assert_eq!(
        field(&own.body, "screen_from"),
        Some(date_in(1).as_str()),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn a_released_pending_entry_is_no_longer_held() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Leaked", "10", "7").await;
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    let held = app
        .post_as("/entries", Some(&token), entry("Steering held", ""))
        .await;
    let held = id_in(&held.body);
    revoke(&app, &cookie, &code).await;
    app.form(&format!("/admin/e/{held}/release"), &[], Some(&cookie))
        .await;
    let own = app
        .send(
            Request::get(format!("/e/{held}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(field(&own.body, "publish_at"), None, "{}", own.body);
}

#[tokio::test]
async fn rejecting_a_frozen_batch_member_rejects_its_frozen_siblings_and_gives_back_units() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Leaked", "10", "7").await;
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    let batch = serde_json::json!({
        "entries": [entry("Steering one", "slug: one\n"), entry("Steering two", "slug: two\n")]
    });
    let made = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(Body::from(batch.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let ids = common::batch_member_ids(&serde_json::from_str(&made.body).unwrap());
    assert_eq!(ids.len(), 2);
    // 18 more fill the 20 a day the account has once its code is revoked.
    for n in 0..18 {
        app.post_as(
            "/entries",
            Some(&token),
            entry(&format!("Steering {n}"), ""),
        )
        .await;
    }
    revoke(&app, &cookie, &code).await;
    let rejected = app
        .form(&format!("/admin/e/{}/reject", ids[0]), &[], Some(&cookie))
        .await;
    assert_eq!(rejected.status, StatusCode::SEE_OTHER, "{}", rejected.body);
    for id in &ids {
        assert_eq!(status_of(&app, &token, id).await, "rejected", "{id}");
    }
    // Both were rejected unscreened, so both units came back: two more fit today.
    for n in 0..2 {
        let reply = app
            .post_as("/entries", Some(&token), entry(&format!("Again {n}"), ""))
            .await;
        assert_eq!(field(&reply.body, "screen_from"), None, "{}", reply.body);
    }
    // No guard: the next entry waits in the screening queue for its day, where a
    // guarded account's would go to review.
    let next = app
        .post_as("/entries", Some(&token), entry("Steering next", ""))
        .await;
    let next = id_in(&next.body);
    app.post_as(&format!("/e/{next}/publish"), Some(&token), "")
        .await;
    assert_eq!(status_of(&app, &token, &next).await, "screening");
}
