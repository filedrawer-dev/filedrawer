//! Seam: the router and `jobs::publish_due`, for the daily allowance (design.md,
//! Limits and abuse): each accepted submission takes a unit dated to the earliest UTC
//! day with room, 20 a day in an account's first 7 days and 50 after, at most 10 days
//! ahead; an entry isn't screened before its unit's day.

mod common;

use axum::http::StatusCode;
use common::fake_models::FakeModels;
use common::series::{DESCRIPTION, series_doc};
use common::{Reply, TestApp, date_in, entry, field, id_in, owner, test_app, test_app_with};
use filedrawer_server::jobs::publish_due;
use filedrawer_server::screening::Screener;
use time::{Duration, OffsetDateTime};

/// Submits an entry titled `title` with the app's token.
async fn post(app: &TestApp, title: &str) -> Reply {
    let reply = app.post("/entries", entry(title, "")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply
}

/// Submits `count` entries, each of which must be screened today.
async fn fill(app: &TestApp, count: usize) -> Vec<String> {
    let mut ids = Vec::new();
    for n in 0..count {
        let reply = post(app, &format!("Steering {n}")).await;
        assert_eq!(field(&reply.body, "screen_from"), None, "{}", reply.body);
        ids.push(id_in(&reply.body));
    }
    ids
}

/// A batch of `count` members whose slugs start with `prefix`.
fn batch(prefix: &str, count: usize) -> String {
    let entries: Vec<String> = (0..count)
        .map(|n| {
            entry(
                &format!("Steering {prefix} {n}"),
                &format!("slug: {prefix}-{n}\n"),
            )
        })
        .collect();
    serde_json::json!({ "entries": entries }).to_string()
}

async fn post_batch(app: &TestApp, prefix: &str, count: usize) -> Reply {
    app.post("/entries/batch", batch(prefix, count)).await
}

/// Makes the app's account `days` old.
async fn backdate(app: &TestApp, days: i64) {
    let created = format!(
        "{}T00:00:00Z",
        (OffsetDateTime::now_utc() - Duration::days(days)).date()
    );
    sqlx::query("UPDATE accounts SET created_at = ? WHERE id = ?")
        .bind(created)
        .bind(&app.account)
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn status(app: &TestApp, id: &str) -> String {
    common::author_status(app, &app.token, id)
        .await
        .unwrap_or_default()
}

#[tokio::test]
async fn a_new_accounts_21st_entry_of_the_day_waits_for_tomorrow() {
    let app = test_app().await;
    fill(&app, 20).await;
    let reply = post(&app, "Steering 21").await;
    let tomorrow = date_in(1);
    assert_eq!(field(&reply.body, "screen_from"), Some(tomorrow.as_str()));
    let id = id_in(&reply.body);

    // Ending the hold early doesn't skip the queue.
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    assert_eq!(field(&placed.body, "status"), Some("screening"));
    assert_eq!(field(&placed.body, "screen_from"), Some(tomorrow.as_str()));
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(field(&own.body, "screen_from"), Some(tomorrow.as_str()));

    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(status(&app, &id).await, "screening");
    publish_due(&app.state, OffsetDateTime::now_utc() + Duration::days(1))
        .await
        .unwrap();
    assert_eq!(status(&app, &id).await, "public");
}

#[tokio::test]
async fn an_account_past_its_first_7_days_gets_50_a_day() {
    let app = test_app().await;
    backdate(&app, 8).await;
    let reply = post_batch(&app, "a", 50).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(field(&reply.body, "screen_from"), None, "{}", reply.body);
    let reply = post(&app, "Steering 51").await;
    assert_eq!(field(&reply.body, "screen_from"), Some(date_in(1).as_str()));
}

#[tokio::test]
async fn a_new_account_may_have_10_days_allowance_waiting_and_no_more() {
    // 7 days at 20, then 3 at 50: 290 units from today through today + 9.
    let app = test_app().await;
    let first = post_batch(&app, "a", 100).await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    // A batch says when its last member's day comes.
    assert_eq!(field(&first.body, "screen_from"), Some(date_in(4).as_str()));
    // Publishing the batch early doesn't skip the queue either.
    let placed = app
        .post(&format!("/batches/{}/publish", id_in(&first.body)), "")
        .await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    assert_eq!(
        field(&placed.body, "screen_from"),
        Some(date_in(4).as_str())
    );
    let second = post_batch(&app, "b", 100).await;
    assert_eq!(
        field(&second.body, "screen_from"),
        Some(date_in(8).as_str())
    );

    // A batch that doesn't fit is refused whole, and takes nothing.
    let too_many = post_batch(&app, "c", 91).await;
    assert_eq!(
        too_many.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        too_many.body
    );
    let last = post_batch(&app, "d", 90).await;
    assert_eq!(last.status, StatusCode::CREATED, "{}", last.body);
    assert_eq!(field(&last.body, "screen_from"), Some(date_in(9).as_str()));

    let refused = app.post("/entries", entry("Steering one more", "")).await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.body
    );
    assert!(refused.body.contains("10 days"), "{}", refused.body);
}

#[tokio::test]
async fn refused_submissions_take_no_allowance() {
    let app = test_app().await;
    let mut members: Vec<String> = (0..25)
        .map(|n| entry(&format!("Steering {n}"), &format!("slug: s{n}\n")))
        .collect();
    members.push(entry("Steering, no slug", ""));
    let batch = serde_json::json!({ "entries": members }).to_string();
    let refused = app.post("/entries/batch", batch).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    for _ in 0..5 {
        let refused = app.post("/entries", "---\ntitle: no fields\n---\n").await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    fill(&app, 20).await;
    let reply = post(&app, "Steering 21").await;
    assert_eq!(field(&reply.body, "screen_from"), Some(date_in(1).as_str()));
}

#[tokio::test]
async fn a_put_takes_a_unit_but_the_entry_keeps_its_day() {
    let app = test_app().await;
    let ids = fill(&app, 20).await;
    let put = app
        .put_as(
            &format!("/e/{}", ids[0]),
            &app.token,
            entry("Steering, fixed", ""),
        )
        .await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.body);
    assert_eq!(field(&put.body, "screen_from"), None, "{}", put.body);
    let placed = app.post(&format!("/e/{}/publish", ids[0]), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
    // The PUT took one of tomorrow's 20, so 19 more fit tomorrow.
    for n in 0..19 {
        let reply = post(&app, &format!("Steering tomorrow {n}")).await;
        assert_eq!(field(&reply.body, "screen_from"), Some(date_in(1).as_str()));
    }
    let reply = post(&app, "Steering later").await;
    assert_eq!(field(&reply.body, "screen_from"), Some(date_in(2).as_str()));
}

#[tokio::test]
async fn a_new_version_takes_a_unit() {
    let app = test_app().await;
    let public = app.submit(&entry("Steering", "")).await;
    fill(&app, 19).await;
    let version = app
        .post(
            &format!("/e/{public}/versions"),
            entry("Steering, corrected", ""),
        )
        .await;
    assert_eq!(version.status, StatusCode::CREATED, "{}", version.body);
    assert_eq!(
        field(&version.body, "screen_from"),
        Some(date_in(1).as_str())
    );
}

#[tokio::test]
async fn a_series_overbooks_today_up_to_a_days_allowance_of_series() {
    let app = test_app().await;
    fill(&app, 20).await;
    let created = app
        .post("/series", series_doc("Layer sweep", DESCRIPTION))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let series = id_in(&created.body);
    // A new version counts as a series too.
    for n in 1..20 {
        let version = app
            .post(
                &format!("/series/{series}/versions"),
                series_doc(&format!("Layer sweep {n}"), DESCRIPTION),
            )
            .await;
        assert_eq!(version.status, StatusCode::CREATED, "{}", version.body);
    }
    let refused = app
        .post(
            &format!("/series/{series}/versions"),
            series_doc("Layer sweep, one more", DESCRIPTION),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.body
    );
    assert!(refused.body.contains("tomorrow"), "{}", refused.body);
}

#[tokio::test]
async fn a_series_counts_toward_the_days_units() {
    let app = test_app().await;
    let created = app
        .post("/series", series_doc("Layer sweep", DESCRIPTION))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    fill(&app, 19).await;
    let reply = post(&app, "Steering 20").await;
    assert_eq!(field(&reply.body, "screen_from"), Some(date_in(1).as_str()));
}

#[tokio::test]
async fn a_series_turned_away_for_the_servers_reasons_takes_no_unit() {
    let models = FakeModels::default();
    // The day's screening budget spent, then the model failing.
    let cases = [
        (
            test_app_with(Screener::PassAll, 0).await,
            "",
            StatusCode::TOO_MANY_REQUESTS,
        ),
        (
            test_app_with(models.screener(), 1_000_000_000).await,
            "FAKE-ERROR",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    ];
    for (app, marker, turned_away) in cases {
        for _ in 0..5 {
            let refused = app
                .post(
                    "/series",
                    series_doc("Layer sweep", &format!("{DESCRIPTION}\n{marker}\n")),
                )
                .await;
            assert_eq!(refused.status, turned_away, "{}", refused.body);
        }
        fill(&app, 20).await;
        let reply = post(&app, "Steering 21").await;
        assert_eq!(field(&reply.body, "screen_from"), Some(date_in(1).as_str()));
    }
}

/// Guards the app's account the way it happens: an entry reaches review and the
/// owner holds it.
async fn guard(app: &TestApp) {
    let id = post(app, "Steering FAKE-HAZARD FAKE-SONNET-HAZARD x").await;
    let id = id_in(&id.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("review"),
        "{}",
        placed.body
    );
    let cookie = owner(app).await;
    let hold = app
        .form(&format!("/admin/e/{id}/hold"), &[], Some(&cookie))
        .await;
    assert_eq!(hold.status, StatusCode::SEE_OTHER, "{}", hold.body);
}

#[tokio::test]
async fn a_guarded_accounts_queue_drains_into_review_at_the_daily_rate() {
    let models = FakeModels::default();
    let app = test_app_with(models.screener(), 1_000_000_000).await;
    guard(&app).await;
    let today = fill(&app, 19).await;
    let late = post(&app, "Steering late").await;
    let late = id_in(&late.body);
    for id in today.iter().chain([&late]) {
        let placed = app.post(&format!("/e/{id}/publish"), "").await;
        assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    }
    for id in &today {
        assert_eq!(status(&app, id).await, "review");
    }
    assert_eq!(status(&app, &late).await, "screening");
    publish_due(&app.state, OffsetDateTime::now_utc() + Duration::days(1))
        .await
        .unwrap();
    assert_eq!(status(&app, &late).await, "review");
}

#[tokio::test]
async fn a_batch_rejected_partway_gives_back_its_unscreened_members_units() {
    let models = FakeModels::default();
    let app = test_app_with(models.screener(), 1_000_000_000).await;
    // Today's 20 members are screened, and the last of them fails; tomorrow's 5 never
    // are.
    let members: Vec<String> = (0..25)
        .map(|n| {
            let marker = if n == 19 { " FAKE-OFFTOPIC" } else { "" };
            entry(&format!("Steering {n}{marker}"), &format!("slug: s{n}\n"))
        })
        .collect();
    let batch = serde_json::json!({ "entries": members }).to_string();
    let reply = app.post("/entries/batch", batch).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(field(&reply.body, "screen_from"), Some(date_in(1).as_str()));
    let id = id_in(&reply.body);
    let placed = app.post(&format!("/batches/{id}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    assert!(!placed.body.contains("screening"), "{}", placed.body);
    // Today's screened members kept their units, and tomorrow's came back.
    for n in 0..20 {
        let reply = post(&app, &format!("Steering tomorrow {n}")).await;
        assert_eq!(field(&reply.body, "screen_from"), Some(date_in(1).as_str()));
    }
    let reply = post(&app, "Steering later").await;
    assert_eq!(field(&reply.body, "screen_from"), Some(date_in(2).as_str()));
}

/// Staging's accounts that are outside the allowance.
#[derive(Debug, Clone, Copy)]
enum TestAccount {
    Smoke,
    ScreenTest,
    CiScreen,
}

/// A test app whose own account is `role` on staging.
async fn staging_app(role: TestAccount) -> TestApp {
    let mut app = test_app().await;
    let account = Some(app.account.clone());
    match role {
        TestAccount::Smoke => app.state.smoke_account = account,
        TestAccount::ScreenTest => app.state.screen_test_account = account,
        TestAccount::CiScreen => app.state.ci_screen_account = account,
    }
    app.router = filedrawer_server::app(app.state.clone());
    app
}

#[tokio::test]
async fn stagings_test_accounts_have_no_allowance() {
    for role in [
        TestAccount::Smoke,
        TestAccount::ScreenTest,
        TestAccount::CiScreen,
    ] {
        let app = staging_app(role).await;
        fill(&app, 25).await;
        for n in 0..25 {
            let created = app
                .post(
                    "/series",
                    series_doc(&format!("Layer sweep {n}"), DESCRIPTION),
                )
                .await;
            assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        }
    }
}
