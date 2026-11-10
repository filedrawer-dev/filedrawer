//! Seam: the router, with the owner's browser session, for suspending an account
//! (design.md, Admin; #43): its entries become removed, its token can read but not
//! write, and lifting the suspension lets it write again but restores nothing.

use crate::common;

use axum::http::StatusCode;
use common::series::{create, make_public};
use common::{Reply, TestApp, browser_account, entry, field, owner, test_app};
use filedrawer_server::jobs::publish_due;
use filedrawer_server::limits::{Limits, Rate};
use std::time::Duration;
use time::OffsetDateTime;

async fn suspend(app: &TestApp, cookie: &str, account: &str) -> Reply {
    app.form(
        &format!("/admin/accounts/{account}/suspend"),
        &[],
        Some(cookie),
    )
    .await
}

async fn lift(app: &TestApp, cookie: &str, account: &str) -> Reply {
    app.form(
        &format!("/admin/accounts/{account}/lift"),
        &[],
        Some(cookie),
    )
    .await
}

/// Checks `reply` went back to the account's admin page.
fn done(reply: &Reply, account: &str) {
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    assert_eq!(
        reply.location(),
        Some(format!("/admin/accounts/{account}").as_str()),
        "{}",
        reply.body
    );
}

/// Checks `reply` is the suspended account's 403, which names where to write.
fn refused(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    assert!(reply.body.contains("suspended"), "{}", reply.body);
    assert!(
        reply.body.contains("contact@filedrawer.dev"),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn a_suspended_account_has_every_entry_removed_and_its_token_can_only_read() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let public = app.submit(&entry("Steering vectors", "")).await;
    let pending = app.submit_held(&entry("Probes", "")).await;

    let page = app
        .page(&format!("/admin/accounts/{}", app.account), Some(&cookie))
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains(&format!("href=\"/admin/e/{public}\"")),
        "{}",
        page.body
    );
    assert!(
        page.body.contains(&format!("href=\"/admin/e/{pending}\"")),
        "{}",
        page.body
    );
    done(&suspend(&app, &cookie, &app.account).await, &app.account);

    assert_eq!(
        app.get(&format!("/e/{public}")).await.status,
        StatusCode::NOT_FOUND
    );
    publish_due(
        &app.state,
        OffsetDateTime::now_utc() + time::Duration::days(1),
    )
    .await
    .unwrap();
    for id in [&public, &pending] {
        assert_eq!(
            common::author_status(&app, &app.token, id).await.as_deref(),
            Some("removed"),
            "{id}"
        );
    }
    refused(&app.post("/entries", entry("Again", "")).await);
    refused(
        &app.post(&format!("/e/{public}/versions"), entry("Again", ""))
            .await,
    );
    assert_eq!(
        app.get_as_author("/entries/mine").await.status,
        StatusCode::OK
    );

    let page = app
        .page(&format!("/admin/accounts/{}", app.account), Some(&cookie))
        .await;
    assert!(
        page.body
            .contains(&format!("action=\"/admin/accounts/{}/lift\"", app.account)),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn a_suspended_accounts_refused_writes_dont_count_against_its_rate() {
    let app = test_app().await.with_limits(Limits {
        writes_per_account: Rate {
            count: 1,
            per: Duration::from_secs(3600),
        },
        ..Limits::site()
    });
    let cookie = owner(&app).await;
    done(&suspend(&app, &cookie, &app.account).await, &app.account);
    for _ in 0..3 {
        refused(&app.post("/entries", entry("Again", "")).await);
    }
}

#[tokio::test]
async fn lifting_a_suspension_lets_the_account_write_but_restores_nothing() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    done(&suspend(&app, &cookie, &app.account).await, &app.account);

    // While it is suspended, none of its entries comes back.
    let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
    assert!(page.has_class("restore-refused"), "{}", page.body);
    let early = app
        .form(&format!("/admin/e/{id}/restore"), &[], Some(&cookie))
        .await;
    assert_eq!(early.status, StatusCode::CONFLICT, "{}", early.body);

    done(&lift(&app, &cookie, &app.account).await, &app.account);
    assert_eq!(
        common::author_status(&app, &app.token, &id)
            .await
            .as_deref(),
        Some("removed")
    );
    let again = app.post("/entries", entry("Again", "")).await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);

    let restored = app
        .form(&format!("/admin/e/{id}/restore"), &[], Some(&cookie))
        .await;
    assert_eq!(restored.status, StatusCode::SEE_OTHER, "{}", restored.body);
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_suspended_account_cant_make_its_series_public() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (author, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let account = common::account_id(&app, &author).await;
    done(&suspend(&app, &cookie, &account).await, &account);

    let reply = make_public(&app, &author, &series).await;
    refused(&reply);
    let page = app.get(&format!("/series/{series}")).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND, "{}", page.body);
}

#[tokio::test]
async fn an_entrys_admin_page_links_its_accounts_page() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
    assert!(
        page.body
            .contains(&format!("href=\"/admin/accounts/{}\"", app.account)),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn only_the_admins_session_suspends_and_wrong_states_are_refused() {
    let app = test_app().await;
    let (someone, _) = browser_account(&app, "grace@example.com").await;
    let page = format!("/admin/accounts/{}", app.account);
    assert_eq!(
        app.page(&page, Some(&someone)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        suspend(&app, &someone, &app.account).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.form(&format!("{page}/suspend"), &[], None)
            .await
            .location(),
        Some("/login")
    );
    let still = app.post("/entries", entry("Still", "")).await;
    assert_eq!(still.status, StatusCode::CREATED, "{}", still.body);

    let cookie = owner(&app).await;
    assert_eq!(
        lift(&app, &cookie, &app.account).await.status,
        StatusCode::CONFLICT
    );
    done(&suspend(&app, &cookie, &app.account).await, &app.account);
    assert_eq!(
        suspend(&app, &cookie, &app.account).await.status,
        StatusCode::CONFLICT
    );
    let unknown = "a-0000000000000000";
    assert_eq!(
        app.page(&format!("/admin/accounts/{unknown}"), Some(&cookie))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        suspend(&app, &cookie, unknown).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(field(&still.body, "id").is_some());
}

/// A write that passed its token check before the suspension committed still adds no
/// entry: the database refuses the insert. Checked on the table, since the router
/// can't hold a request between its token check and its insert.
#[tokio::test]
async fn the_database_refuses_an_entry_from_a_suspended_account() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    done(&suspend(&app, &cookie, &app.account).await, &app.account);
    let inserted = sqlx::query(
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version)
         VALUES ('e-race', ?, 'pending', 'text', '2026-11-09', '2026-11-09', 1)",
    )
    .bind(&app.account)
    .execute(&app.pool)
    .await;
    let error = inserted.expect_err("the insert is refused").to_string();
    assert!(error.contains("the account is suspended"), "{error}");
}

#[tokio::test]
async fn an_entry_removed_before_the_suspension_keeps_its_own_status_to_restore() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let id = app.submit_held(&entry("Steering vectors", "")).await;
    let removed = app
        .form(&format!("/admin/e/{id}/remove"), &[], Some(&cookie))
        .await;
    assert_eq!(removed.status, StatusCode::SEE_OTHER, "{}", removed.body);
    done(&suspend(&app, &cookie, &app.account).await, &app.account);
    done(&lift(&app, &cookie, &app.account).await, &app.account);

    let restored = app
        .form(&format!("/admin/e/{id}/restore"), &[], Some(&cookie))
        .await;
    assert_eq!(restored.status, StatusCode::SEE_OTHER, "{}", restored.body);
    assert_eq!(
        common::author_status(&app, &app.token, &id)
            .await
            .as_deref(),
        Some("pending")
    );
}

#[tokio::test]
async fn a_suspended_account_still_logs_in_and_makes_tokens_and_a_deleted_one_has_no_page() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (author, _) = browser_account(&app, "grace@example.com").await;
    let account = common::account_id(&app, &author).await;
    done(&suspend(&app, &cookie, &account).await, &account);

    let again = common::logged_in(&app, "grace@example.com", common::PASSWORD).await;
    let token = common::new_token(&app, &again).await;
    assert_eq!(
        app.get_with_token("/entries/mine", &token).await.status,
        StatusCode::OK
    );

    let deleted = app
        .form(
            "/account/delete",
            &[("password", common::PASSWORD)],
            Some(&again),
        )
        .await;
    assert!(
        deleted.status.is_redirection() || deleted.status.is_success(),
        "{}",
        deleted.body
    );
    let page = app
        .page(&format!("/admin/accounts/{account}"), Some(&cookie))
        .await;
    assert_eq!(page.status, StatusCode::NOT_FOUND, "{}", page.body);
}
