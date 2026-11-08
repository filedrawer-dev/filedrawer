//! Seam: the router and `jobs::release_due`, for releasing embargoes (design.md,
//! Embargo; Entry status; Entry lifecycle; Series): an embargoed entry goes public
//! at 23:59 at UTC−12 on its date, only from `embargoed`, with every version of its
//! chain, its links due for a recheck, and an entry waiting on a hidden series goes
//! public at the 12-month cap.

mod common;

use axum::http::StatusCode;
use common::fake_models::FakeModels;
use common::series::{citing, create, submit_with};
use common::{
    TestApp, author_status, browser_account, date_in, entry, field, id_in, test_app, test_app_with,
};
use filedrawer_server::jobs::release_due;
use time::macros::format_description;
use time::{Date, Duration, OffsetDateTime, PrimitiveDateTime, Time};

/// When an embargo dated `date` (YYYY-MM-DD) ends: 23:59 at UTC−12 on that date,
/// which is 11:59 UTC the next day.
fn release_time(date: &str) -> OffsetDateTime {
    let date = Date::parse(date, format_description!("[year]-[month]-[day]")).unwrap();
    PrimitiveDateTime::new(date + Duration::days(1), Time::from_hms(11, 59, 0).unwrap())
        .assume_utc()
}

async fn release(app: &TestApp, now: OffsetDateTime) -> usize {
    release_due(&app.state, now).await.unwrap()
}

/// Whether anyone can read entry `id`.
async fn is_public(app: &TestApp, id: &str) -> bool {
    let reply = app.get(&format!("/e/{id}")).await;
    match reply.status {
        StatusCode::OK => true,
        StatusCode::NOT_FOUND => false,
        other => panic!("GET /e/{id} answered {other}: {}", reply.body),
    }
}

async fn status(app: &TestApp, id: &str) -> String {
    author_status(app, &app.token, id)
        .await
        .expect("the author's view gives a status")
}

/// Submits `text` as a new version of `id` and ends its hold; returns its ID.
async fn new_version(app: &TestApp, id: &str, text: &str) -> String {
    let held = app
        .post(&format!("/e/{id}/versions"), text.to_string())
        .await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let version = id_in(&held.body);
    let placed = app.post(&format!("/e/{version}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    version
}

/// Writes `date` as entry `id`'s stored embargo. A new version takes its chain's date,
/// so a chain whose versions name different dates is one stored before that rule;
/// nothing else can make one now.
async fn store_date(app: &TestApp, id: &str, date: &str) {
    sqlx::query("UPDATE entries SET embargo_until = ? WHERE id = ?")
        .bind(date)
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_embargoed_entry_is_released_at_2359_at_utc_minus_12_on_its_date() {
    let app = test_app().await;
    let date = date_in(3);
    let text = entry(
        "Quiet steering result",
        &format!("embargo_until: {date}\ntags: [late-tag]\n"),
    )
    .replace("tags: [steering, refusal]\n", "");
    let id = app.submit(&text).await;
    assert_eq!(status(&app, &id).await, "embargoed");

    assert_eq!(
        release(&app, release_time(&date) - Duration::minutes(1)).await,
        0
    );
    assert!(!is_public(&app, &id).await);
    assert!(!app.get("/search?q=quiet").await.body.contains(&id));
    assert!(!app.get("/tags").await.body.contains("late-tag"));

    assert_eq!(release(&app, release_time(&date)).await, 1);
    let page = app.get(&format!("/e/{id}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(field(&page.body, "published").is_some_and(|at| !at.is_empty()));
    assert!(app.get("/search?q=quiet").await.body.contains(&id));
    assert!(app.get("/tags").await.body.contains("late-tag"));
    // Released once: a later run finds nothing more to do.
    assert_eq!(release(&app, release_time(&date)).await, 0);
}

#[tokio::test]
async fn only_an_embargoed_entry_is_released_and_one_in_review_waits() {
    let app = test_app_with(FakeModels::default().screener(), 1_000_000_000).await;
    let date = date_in(3);
    let front = format!("embargo_until: {date}\n");
    let passed = app.submit(&entry("Steering", &front)).await;
    let flagged = app
        .submit(&entry("Steering FAKE-HAZARD FAKE-SONNET-HAZARD", &front))
        .await;
    assert_eq!(status(&app, &passed).await, "embargoed");
    assert_eq!(status(&app, &flagged).await, "review");

    assert_eq!(release(&app, release_time(&date)).await, 1);
    assert!(is_public(&app, &passed).await);
    assert!(!is_public(&app, &flagged).await);
    assert_eq!(status(&app, &flagged).await, "review");
}

#[tokio::test]
async fn a_chain_releases_together_on_its_latest_versions_later_date() {
    let app = test_app().await;
    let (first, later) = (date_in(10), date_in(20));
    let v1 = app
        .submit(&entry("Steering", &format!("embargo_until: {first}\n")))
        .await;
    let v2 = new_version(&app, &v1, &entry("Steering, corrected", "")).await;
    store_date(&app, &v2, &later).await;
    assert_eq!(status(&app, &v2).await, "embargoed");

    assert_eq!(release(&app, release_time(&first)).await, 0);
    assert!(!is_public(&app, &v1).await);
    assert!(!is_public(&app, &v2).await);

    assert_eq!(release(&app, release_time(&later)).await, 2);
    let old = app.get(&format!("/e/{v1}")).await;
    assert_eq!(old.status, StatusCode::OK, "{}", old.body);
    assert_eq!(field(&old.body, "superseded_by"), Some(v2.as_str()));
    assert!(is_public(&app, &v2).await);
}

#[tokio::test]
async fn a_rejected_version_leaves_the_chain_on_its_own_date() {
    let app = test_app_with(FakeModels::default().screener(), 1_000_000_000).await;
    let (first, later) = (date_in(10), date_in(20));
    let v1 = app
        .submit(&entry("Steering", &format!("embargo_until: {first}\n")))
        .await;
    let v2 = new_version(&app, &v1, &entry("Steering FAKE-SPAM", "")).await;
    assert_eq!(status(&app, &v2).await, "rejected");
    store_date(&app, &v2, &later).await;

    assert_eq!(release(&app, release_time(&first)).await, 1);
    assert!(is_public(&app, &v1).await);
}

#[tokio::test]
async fn the_author_sees_the_chains_release_date_on_every_version() {
    let app = test_app().await;
    let (first, later) = (date_in(10), date_in(20));
    let v1 = app
        .submit(&entry("Steering", &format!("embargo_until: {first}\n")))
        .await;
    let v2 = new_version(&app, &v1, &entry("Steering, corrected", "")).await;
    store_date(&app, &v2, &later).await;
    for id in [&v1, &v2] {
        let own = app.get_as_author(&format!("/e/{id}")).await;
        assert_eq!(field(&own.body, "release"), Some(later.as_str()), "{id}");
        let own = app
            .send(
                axum::http::Request::get(format!("/e/{id}"))
                    .header("authorization", format!("Bearer {}", app.token))
                    .header("accept", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        let json: serde_json::Value = serde_json::from_str(&own.body).unwrap();
        assert_eq!(json["release"], later.as_str(), "{id}");
    }
}

#[tokio::test]
async fn a_chain_releases_together_on_its_latest_versions_earlier_date() {
    let app = test_app().await;
    let (first, sooner) = (date_in(30), date_in(10));
    let v1 = app
        .submit(&entry("Steering", &format!("embargo_until: {first}\n")))
        .await;
    let v2 = new_version(&app, &v1, &entry("Steering, corrected", "")).await;
    store_date(&app, &v2, &sooner).await;

    assert_eq!(
        release(&app, release_time(&sooner) - Duration::minutes(1)).await,
        0
    );
    assert_eq!(release(&app, release_time(&sooner)).await, 2);
    assert!(is_public(&app, &v1).await);
    assert!(is_public(&app, &v2).await);
}

const LOGS: &str =
    "https://huggingface.co/datasets/ada/logs/tree/0123456789abcdef0123456789abcdef01234567";
const CODE: &str = "https://github.com/ada/steering/tree/0123456789abcdef0123456789abcdef01234567";

/// The card's line of flags in a plain-text reply.
fn flags(body: &str) -> &str {
    body.lines()
        .map(str::trim)
        .find(|line| line.starts_with("analyzer v"))
        .expect("the reply holds a card")
}

async fn recheck(app: &TestApp, now: OffsetDateTime) -> usize {
    filedrawer_server::link_checks::recheck_due(&app.state, now)
        .await
        .unwrap()
}

#[tokio::test]
async fn release_has_the_links_checked_again() {
    let app = test_app().await;
    let date = date_in(3);
    let id = app
        .submit(&entry(
            "Steering",
            &format!("embargo_until: {date}\ncode: {CODE}\n"),
        ))
        .await;
    // The link answered at submission, so no recheck is due before release.
    app.web.answer(CODE, 404);
    assert_eq!(recheck(&app, release_time(&date)).await, 0);

    release(&app, release_time(&date)).await;
    assert_eq!(recheck(&app, release_time(&date)).await, 1);
    let page = app.get(&format!("/e/{id}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        flags(&page.body).contains("; code link dead"),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn an_embargoed_entry_in_a_hidden_series_waits_for_it_at_release() {
    let app = test_app().await;
    let (_, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let date = date_in(3);
    let id = submit_with(
        &app,
        &token,
        &citing(
            "Steering at 14",
            &format!("{series}@1#models"),
            &format!("embargo_until: {date}\n"),
        ),
    )
    .await;
    assert_eq!(
        author_status(&app, &token, &id).await.as_deref(),
        Some("embargoed")
    );

    assert_eq!(release(&app, release_time(&date)).await, 1);
    assert!(!is_public(&app, &id).await);
    assert_eq!(
        author_status(&app, &token, &id).await.as_deref(),
        Some("waiting_on_series")
    );
}

/// The day 12 months after today (UTC), or that month's last day when it is shorter.
fn a_year_from_today() -> String {
    let today = OffsetDateTime::now_utc().date();
    let year = today.year() + 1;
    let day = today
        .day()
        .min(time::util::days_in_month(today.month(), year));
    Date::from_calendar_date(year, today.month(), day)
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn an_entry_waiting_on_a_hidden_series_is_released_at_the_12_month_cap() {
    let app = test_app().await;
    let (_, token) = browser_account(&app, "grace@example.com").await;
    let (series, _) = create(&app, &token).await;
    let id = submit_with(
        &app,
        &token,
        &citing(
            "Steering at 14",
            &format!("{series}@1#models"),
            &format!("logs: [{LOGS}]\n"),
        ),
    )
    .await;
    assert_eq!(
        author_status(&app, &token, &id).await.as_deref(),
        Some("waiting_on_series")
    );
    app.web.answer(LOGS, 404);
    let cap = release_time(&a_year_from_today());

    assert_eq!(release(&app, cap - Duration::minutes(1)).await, 0);
    assert!(!is_public(&app, &id).await);

    assert_eq!(release(&app, cap).await, 1);
    let page = app.get(&format!("/e/{id}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert_eq!(field(&page.body, "series_description"), Some("not public"));
    assert!(!page.body.contains("Layer sweep"), "{}", page.body);
    assert_eq!(
        app.get(&format!("/series/{series}")).await.status,
        StatusCode::NOT_FOUND
    );
    // Its links are checked again, as at an embargo's release.
    assert_eq!(recheck(&app, cap).await, 1);
    let page = app.get(&format!("/e/{id}")).await;
    assert!(
        flags(&page.body).contains("; logs link dead"),
        "{}",
        page.body
    );
}

#[tokio::test]
async fn a_stored_date_that_isnt_one_holds_the_entry() {
    let app = test_app().await;
    let id = app
        .submit(&entry(
            "Steering",
            &format!("embargo_until: {}\n", date_in(3)),
        ))
        .await;
    store_date(&app, &id, "soon").await;
    assert_eq!(
        release(&app, OffsetDateTime::now_utc() + Duration::days(30)).await,
        0
    );
    assert_eq!(status(&app, &id).await, "embargoed");
}
