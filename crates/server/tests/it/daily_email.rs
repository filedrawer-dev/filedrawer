//! Seam: `daily_email::send_daily_email`, with the router to set up what it reports,
//! a fake model client, and a fake healthchecks.io: the owner's daily email
//! (design.md, Admin).

use crate::common;
use crate::invites::{create_code, invited_account, revoke};

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::fake_models::FakeModels;
use common::{DELIMITER, Outbox, TestApp, entry, id_in, owner, test_app_with};
use filedrawer_server::config::Environment;
use filedrawer_server::daily_email::{DailyRun, Pinger, Pinging, send_daily_email};
use filedrawer_server::mail::{Email, MailError, Mailer, Sending};
use time::{Duration, OffsetDateTime};

const DOLLAR: i64 = 1_000_000_000;
const HAZARD: &str = "FAKE-HAZARD FAKE-SONNET-HAZARD";
const OWNER: &str = "owner@example.com";

/// A healthchecks.io that keeps each ping: `true` for success, `false` for `/fail`.
#[derive(Clone, Default)]
struct Pings(Arc<Mutex<Vec<bool>>>);

impl Pings {
    fn sent(&self) -> Vec<bool> {
        self.0.lock().unwrap().clone()
    }
}

impl Pinger for Pings {
    fn ping(&self, ok: bool) -> Pinging<'_> {
        self.0.lock().unwrap().push(ok);
        Box::pin(async { Ok(()) })
    }
}

/// A test app in `environment` with the fake models and a fake healthchecks.io; the
/// owner is an admin unless `with_owner` is false.
async fn app_in(environment: Environment, with_owner: bool) -> (TestApp, Pings) {
    let mut app = test_app_with(FakeModels::default().screener(), DOLLAR).await;
    if with_owner {
        owner(&app).await;
    }
    let pings = Pings::default();
    app.state.environment = environment;
    app.state.pinger = Some(Arc::new(pings.clone()));
    (app, pings)
}

/// `at` as the server writes times: UTC, to the second.
fn stamp(at: OffsetDateTime) -> String {
    at.format(time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
    ))
    .unwrap()
}

/// Writes today's nightly copy, so production finds its copies fresh.
fn fresh_copy(app: &TestApp) {
    std::fs::create_dir(&app.state.backups_dir).unwrap();
    let today = OffsetDateTime::now_utc().date();
    std::fs::write(
        app.state.backups_dir.join(format!("filedrawer-{today}.db")),
        "",
    )
    .unwrap();
}

/// A minute from now, so whatever the test did lies in the run's period.
fn soon() -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::minutes(1)
}

/// The daily emails sent so far, leaving out the owner's signup email.
fn reports(outbox: &Outbox) -> Vec<Email> {
    outbox
        .sent()
        .into_iter()
        .filter(|email| email.subject.contains("daily report"))
        .collect()
}

/// The one daily email sent so far.
fn the_report(app: &TestApp) -> Email {
    let mut reports = reports(&app.outbox);
    assert_eq!(reports.len(), 1, "{reports:#?}");
    reports.remove(0)
}

/// Submits `text` as the app's account and ends its hold; returns its ID.
async fn submit(app: &TestApp, text: &str) -> String {
    let held = app.post("/entries", text.to_string()).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    id
}

async fn feedback(app: &TestApp, experiment: &str, wanted: &str, workaround: &str) {
    let body = serde_json::json!({
        "experiment": experiment, "wanted": wanted, "workaround": workaround,
    });
    let request = Request::post("/feedback")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", app.token))
        .body(Body::from(body.to_string()))
        .unwrap();
    let reply = app.send(request).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[tokio::test]
async fn a_day_with_nothing_to_report_sends_nothing_and_pings_success() {
    let (app, pings) = app_in(Environment::Production, true).await;
    fresh_copy(&app);

    let run = send_daily_email(&app.state, soon()).await.unwrap();

    assert_eq!(run, DailyRun::Quiet);
    assert!(reports(&app.outbox).is_empty());
    assert_eq!(pings.sent(), vec![true]);
}

#[tokio::test]
async fn it_runs_once_a_utc_day_and_again_after_midnight() {
    let (app, pings) = app_in(Environment::Dev, true).await;
    let midnight = OffsetDateTime::now_utc().replace_time(time::Time::MIDNIGHT);
    let first = midnight + Duration::days(1) + Duration::minutes(3);

    assert_eq!(
        send_daily_email(&app.state, first).await.unwrap(),
        DailyRun::Quiet
    );
    let later = first + Duration::hours(23);
    assert_eq!(
        send_daily_email(&app.state, later).await.unwrap(),
        DailyRun::NotDue
    );
    assert_eq!(pings.sent(), vec![true], "a run not due pings nothing");
    let next_day = first + Duration::days(1);
    assert_eq!(
        send_daily_email(&app.state, next_day).await.unwrap(),
        DailyRun::Quiet
    );
    assert_eq!(pings.sent(), vec![true, true]);
}

#[tokio::test]
async fn a_stored_run_later_than_now_is_taken_for_a_wrong_clock_and_runs() {
    let (app, _) = app_in(Environment::Dev, true).await;
    let future = OffsetDateTime::now_utc() + Duration::days(3650);
    send_daily_email(&app.state, future).await.unwrap();

    let run = send_daily_email(&app.state, soon()).await.unwrap();

    assert_eq!(run, DailyRun::Quiet);
}

#[tokio::test]
async fn production_lists_what_screening_published_each_by_an_admin_link() {
    let (app, _) = app_in(Environment::Production, true).await;
    let id = submit(&app, &entry("Steering published", "")).await;

    let run = send_daily_email(&app.state, soon()).await.unwrap();

    assert_eq!(run, DailyRun::Sent { to: 1 });
    let email = the_report(&app);
    assert_eq!(email.to, OWNER);
    assert_eq!(email.subject, "File Drawer daily report");
    assert!(email.body.contains("Published (1):"), "{}", email.body);
    assert!(
        email
            .body
            .contains(&format!("- {id} http://localhost/admin/e/{id}")),
        "{}",
        email.body
    );
    assert!(
        !email.body.contains("Steering published"),
        "author text stays out of the server's sections:\n{}",
        email.body
    );
}

#[tokio::test]
async fn the_next_days_email_leaves_out_what_the_last_one_listed() {
    let (app, _) = app_in(Environment::Production, true).await;
    fresh_copy(&app);
    submit(&app, &entry("Steering published", "")).await;
    send_daily_email(&app.state, soon()).await.unwrap();

    let run = send_daily_email(&app.state, soon() + Duration::days(1))
        .await
        .unwrap();

    assert_eq!(run, DailyRun::Quiet);
}

#[tokio::test]
async fn entries_waiting_for_the_owner_nag_every_day_with_the_oldests_age() {
    let (app, _) = app_in(Environment::Production, true).await;
    let id = submit(&app, &entry(&format!("Steering {HAZARD}"), "")).await;

    send_daily_email(&app.state, soon()).await.unwrap();
    send_daily_email(&app.state, soon() + Duration::days(1))
        .await
        .unwrap();

    let reports = reports(&app.outbox);
    assert_eq!(reports.len(), 2, "{reports:#?}");
    let body = &reports[1].body;
    assert!(
        body.contains(
            "Waiting for the owner (1), the oldest submitted 1 day ago: http://localhost/admin"
        ),
        "{body}"
    );
    assert!(
        body.contains(&format!("- {id} review http://localhost/admin/e/{id}")),
        "{body}"
    );
}

#[tokio::test]
async fn frozen_entries_are_listed() {
    let (app, _) = app_in(Environment::Production, false).await;
    let cookie = owner(&app).await;
    let code = create_code(&app, &cookie, "Leaked", "10", "7").await;
    let (_, token) = invited_account(&app, &code, "grace@example.com").await;
    let reply = app
        .post_as("/entries", Some(&token), entry("Steering frozen", ""))
        .await;
    let id = id_in(&reply.body);
    revoke(&app, &cookie, &code).await;

    send_daily_email(&app.state, soon()).await.unwrap();

    let body = the_report(&app).body;
    assert!(body.contains("Frozen (1):"), "{body}");
    assert!(
        body.contains(&format!("- {id} http://localhost/admin/e/{id}")),
        "{body}"
    );
}

#[tokio::test]
async fn staging_mails_a_failing_screen_but_not_published_or_waiting_entries() {
    let (app, _) = app_in(Environment::Staging, true).await;
    submit(&app, &entry("Steering published", "")).await;
    submit(&app, &entry(&format!("Steering {HAZARD}"), "")).await;
    assert_eq!(
        send_daily_email(&app.state, soon()).await.unwrap(),
        DailyRun::Quiet
    );

    submit(&app, &entry("Steering FAKE-ERROR", "")).await;
    let run = send_daily_email(&app.state, soon() + Duration::days(1))
        .await
        .unwrap();

    assert_eq!(run, DailyRun::Sent { to: 1 });
    let email = the_report(&app);
    assert_eq!(email.subject, "File Drawer (staging) daily report");
    assert!(
        email
            .body
            .contains("Screening: 1 failed call; the last call failed, at "),
        "{}",
        email.body
    );
    assert!(
        email.body.contains("Waiting for the owner (1)"),
        "{}",
        email.body
    );
}

#[tokio::test]
async fn a_failure_the_screen_recovered_from_is_listed_but_sends_nothing_alone() {
    let (app, _) = app_in(Environment::Dev, true).await;
    submit(&app, &entry("Steering FAKE-ERROR", "")).await;
    submit(&app, &entry("Steering fine", "")).await;

    // A day back, so the next run's period holds what the test does after it.
    let run = send_daily_email(&app.state, soon() - Duration::days(1))
        .await
        .unwrap();
    assert_eq!(run, DailyRun::Quiet);

    // Feedback sends the next one, which no longer counts the reported failure.
    feedback(
        &app,
        "steering sweep",
        "a column per layer",
        "one entry per layer",
    )
    .await;
    submit(&app, &entry("Steering FAKE-ERROR again", "")).await;
    submit(&app, &entry("Steering fine again", "")).await;
    send_daily_email(&app.state, soon()).await.unwrap();
    let body = the_report(&app).body;
    assert!(
        body.contains("Screening: 1 failed call; the last at "),
        "{body}"
    );
}

#[tokio::test]
async fn the_cap_running_out_and_an_account_at_its_share_are_reported() {
    let mut app = test_app_with(FakeModels::default().screener(), 1).await;
    owner(&app).await;
    app.state.environment = Environment::Production;
    submit(&app, &entry("Steering costly", "")).await;

    // Today's spend is reported once the day is whole.
    send_daily_email(&app.state, soon() + Duration::days(1))
        .await
        .unwrap();

    let body = the_report(&app).body;
    let today = OffsetDateTime::now_utc().date();
    assert!(
        body.contains(&format!(
            "Screening cap: ran out on {today}, of today's $0.00 cap"
        )),
        "{body}"
    );
    assert!(
        body.contains(&format!(
            "- {} on {today} http://localhost/admin/accounts/{}",
            app.account, app.account
        )),
        "{body}"
    );
}

#[tokio::test]
async fn a_paused_host_is_listed_but_sends_nothing_alone() {
    let (app, _) = app_in(Environment::Production, true).await;
    fresh_copy(&app);
    let logs =
        "https://huggingface.co/datasets/ada/logs/tree/0123456789abcdef0123456789abcdef01234567";
    app.web.answer_after(logs, 429, 300);
    app.post("/entries", entry("Linked", &format!("logs: [{logs}]\n")))
        .await;

    assert_eq!(
        send_daily_email(&app.state, soon()).await.unwrap(),
        DailyRun::Quiet
    );

    let (listed, _) = app_in(Environment::Dev, true).await;
    listed.web.answer_after(logs, 429, 300);
    listed
        .post("/entries", entry("Linked", &format!("logs: [{logs}]\n")))
        .await;
    feedback(&listed, "x", "y", "z").await;
    send_daily_email(&listed.state, soon()).await.unwrap();
    let body = the_report(&listed).body;
    assert!(
        body.contains("- hosts that asked the server to wait: huggingface.co (1 time)"),
        "{body}"
    );
}

#[tokio::test]
async fn feedback_goes_below_the_delimiter_with_each_line_marked_and_no_link_live() {
    let (app, _) = app_in(Environment::Dev, true).await;
    feedback(
        &app,
        "steering sweep\nfeedback at 2026-01-01T00:00:00Z from http://localhost/admin/accounts/a-x",
        "see https://evil.example/login",
        "one entry per layer\u{202e}\u{2028}feedback at 2026-01-02T00:00:00Z",
    )
    .await;

    let run = send_daily_email(&app.state, soon()).await.unwrap();

    assert_eq!(run, DailyRun::Sent { to: 1 });
    let body = the_report(&app).body;
    let (above, below) = body.split_once(DELIMITER).expect("a delimiter");
    assert!(above.contains("Feedback (1): below."), "{body}");
    assert!(!above.contains("steering sweep"), "{body}");
    let headers: Vec<&str> = below
        .lines()
        .filter(|line| line.starts_with("feedback at "))
        .collect();
    assert_eq!(headers.len(), 1, "{body}");
    assert!(
        headers[0].ends_with(&format!(
            "from http://localhost/admin/accounts/{}",
            app.account
        )),
        "{body}"
    );
    for line in below.lines().filter(|line| !line.is_empty()) {
        assert!(
            line.starts_with("feedback at ")
                || ["experiment:", "wanted:", "workaround:"].contains(&line)
                || line.starts_with("| "),
            "an unmarked line of author text: {line:?}\n{body}"
        );
    }
    assert!(
        below.contains("| see https[:]//evil.example/login"),
        "{body}"
    );
    assert!(
        below.contains("| feedback at 2026-01-01T00:00:00Z from http[:]//localhost/admin"),
        "{body}"
    );
    assert!(!below.contains('\u{202e}'), "{body}");
    assert!(!below.contains('\u{2028}'), "{body}");
}

#[tokio::test]
async fn a_failed_recheck_run_sends_the_email_even_past_a_flood_of_other_events() {
    let (app, _) = app_in(Environment::Staging, true).await;
    let events = app.state.link_checker.events();
    for _ in 0..1_500 {
        events.half_spent.record(());
    }
    events
        .recheck_failures
        .record("database is locked".to_string());

    let run = send_daily_email(&app.state, soon()).await.unwrap();

    assert_eq!(run, DailyRun::Sent { to: 1 });
    let body = the_report(&app).body;
    assert!(
        body.contains("- rechecks failed 1 time; the last: database is locked"),
        "{body}"
    );
    assert!(
        body.contains(
            "- GitHub API calls not made because the server had spent its half of the hour's: 1500"
        ),
        "{body}"
    );
}

#[tokio::test]
async fn events_past_the_logs_limit_are_counted_once() {
    let (app, _) = app_in(Environment::Staging, true).await;
    let rechecks = &app.state.link_checker.events().recheck_failures;
    for n in 0..1_200 {
        rechecks.record(format!("failure {n}"));
    }
    send_daily_email(&app.state, soon()).await.unwrap();
    rechecks.record("the next day's".to_string());

    send_daily_email(&app.state, soon() + Duration::days(1))
        .await
        .unwrap();

    let reports = reports(&app.outbox);
    assert_eq!(reports.len(), 2, "{reports:#?}");
    assert!(
        reports[0]
            .body
            .contains("- rechecks failed 1200 times; the last: failure 1199"),
        "{}",
        reports[0].body
    );
    assert!(
        reports[1]
            .body
            .contains("- rechecks failed 1 time; the last: the next day's"),
        "{}",
        reports[1].body
    );
}

#[tokio::test]
async fn a_spent_half_of_githubs_calls_counts_the_calls_not_made() {
    let (app, _) = app_in(Environment::Dev, true).await;
    let sha = "0123456789abcdef0123456789abcdef01234567";
    app.web.api(
        &format!("/repos/ada/steering/compare/HEAD...{sha}?per_page=1"),
        200,
        serde_json::json!({"status": "identical"}),
        2000,
    );
    for repo in ["steering", "probing"] {
        let code = format!("code: https://github.com/ada/{repo}/tree/{sha}\n");
        app.post("/entries", entry("Linked", &code)).await;
    }
    feedback(&app, "x", "y", "z").await;

    send_daily_email(&app.state, soon()).await.unwrap();

    let body = the_report(&app).body;
    assert!(
        body.contains(
            "- GitHub API calls not made because the server had spent its half of the hour's: "
        ),
        "{body}"
    );
}

#[tokio::test]
async fn a_backups_folder_the_server_cant_read_is_reported() {
    let (app, _) = app_in(Environment::Staging, true).await;
    std::fs::write(&app.state.backups_dir, "not a folder").unwrap();

    let run = send_daily_email(&app.state, soon()).await.unwrap();

    assert_eq!(run, DailyRun::Sent { to: 1 });
    assert!(
        the_report(&app)
            .body
            .contains("Nightly copy: can't read backups/: "),
    );
}

#[tokio::test]
async fn past_twenty_feedback_items_the_rest_are_counted() {
    let (app, _) = app_in(Environment::Dev, true).await;
    for n in 0..23 {
        feedback(&app, &format!("experiment {n}"), "w", "x").await;
    }

    send_daily_email(&app.state, soon()).await.unwrap();

    let body = the_report(&app).body;
    assert!(body.contains("Feedback (23): below."), "{body}");
    assert_eq!(body.matches("feedback at ").count(), 20, "{body}");
    assert!(
        body.contains("| experiment 22"),
        "the newest go in full: {body}"
    );
    assert!(!body.contains("| experiment 2\n"), "{body}");
    assert!(
        body.contains("3 more feedback items came; the oldest are left out."),
        "{body}"
    );
}

#[tokio::test]
async fn the_nightly_copy_line_names_the_newest_copy() {
    let (app, _) = app_in(Environment::Dev, true).await;
    feedback(&app, "x", "y", "z").await;
    std::fs::create_dir(&app.state.backups_dir).unwrap();
    let today = OffsetDateTime::now_utc().date();
    let yesterday = today - Duration::days(1);
    for name in [
        format!("filedrawer-{yesterday}.db"),
        format!("filedrawer-{today}.db"),
        format!("filedrawer-{}.db.tmp", today + Duration::days(1)),
        "notes.txt".to_string(),
    ] {
        std::fs::write(app.state.backups_dir.join(name), "").unwrap();
    }

    send_daily_email(&app.state, soon()).await.unwrap();

    let body = the_report(&app).body;
    assert!(
        body.contains(&format!("Nightly copy: filedrawer-{today}.db\n")),
        "{body}"
    );
}

#[tokio::test]
async fn a_stale_or_missing_copy_sends_the_email() {
    let (app, _) = app_in(Environment::Staging, true).await;
    std::fs::create_dir(&app.state.backups_dir).unwrap();
    let old = OffsetDateTime::now_utc().date() - Duration::days(2);
    std::fs::write(
        app.state.backups_dir.join(format!("filedrawer-{old}.db")),
        "",
    )
    .unwrap();

    assert_eq!(
        send_daily_email(&app.state, soon()).await.unwrap(),
        DailyRun::Sent { to: 1 }
    );
    let body = the_report(&app).body;
    assert!(
        body.contains(&format!(
            "Nightly copy: filedrawer-{old}.db, older than yesterday"
        )),
        "{body}"
    );

    let (empty, _) = app_in(Environment::Staging, true).await;
    std::fs::create_dir(&empty.state.backups_dir).unwrap();
    assert_eq!(
        send_daily_email(&empty.state, soon()).await.unwrap(),
        DailyRun::Sent { to: 1 }
    );
    assert!(the_report(&empty).body.contains("Nightly copy: none in "));
}

#[tokio::test]
async fn no_backups_folder_sends_the_email_only_in_production() {
    let (staging, _) = app_in(Environment::Staging, true).await;
    assert_eq!(
        send_daily_email(&staging.state, soon()).await.unwrap(),
        DailyRun::Quiet
    );

    let (production, _) = app_in(Environment::Production, true).await;
    assert_eq!(
        send_daily_email(&production.state, soon()).await.unwrap(),
        DailyRun::Sent { to: 1 }
    );
    assert!(
        the_report(&production)
            .body
            .contains("Nightly copy: not set up: no backups/ beside the database"),
    );
}

#[tokio::test]
async fn counts_kept_in_memory_say_when_the_server_started() {
    let (mut app, _) = app_in(Environment::Dev, true).await;
    let started = OffsetDateTime::now_utc() - Duration::hours(2);
    app.state.started_at = started;
    feedback(&app, "x", "y", "z").await;

    send_daily_email(&app.state, soon()).await.unwrap();

    let body = the_report(&app).body;
    assert!(
        body.contains(&format!(
            "The screen's and the link checks' counts run from when the server started, at {}.",
            stamp(started)
        )),
        "{body}"
    );
}

#[tokio::test]
async fn with_no_admin_production_pings_failure_and_staging_success() {
    for (environment, ping) in [
        (Environment::Production, false),
        (Environment::Staging, true),
    ] {
        let (app, pings) = app_in(environment, false).await;
        feedback(&app, "x", "y", "z").await;

        let run = send_daily_email(&app.state, soon()).await.unwrap();

        assert_eq!(run, DailyRun::NoOneToTell, "{environment:?}");
        assert_eq!(pings.sent(), vec![ping], "{environment:?}");
        assert_eq!(
            send_daily_email(&app.state, soon() + Duration::hours(1))
                .await
                .unwrap(),
            DailyRun::NotDue,
            "{environment:?}"
        );
    }
}

/// A mailer that refuses every email to `refused` and keeps the rest.
struct RefusingFor {
    refused: String,
    outbox: Outbox,
}

impl Mailer for RefusingFor {
    fn send(&self, email: Email) -> Sending<'_> {
        if email.to == self.refused {
            return Box::pin(async move { Err(MailError::Address(email.to)) });
        }
        self.outbox.send(email)
    }
}

#[tokio::test]
async fn a_send_that_fails_for_one_admin_is_retried_for_that_admin_only() {
    let (mut app, pings) = app_in(Environment::Dev, true).await;
    let (cookie, _) = common::browser_account(&app, "second@example.com").await;
    filedrawer_server::admin::grant(&app.pool, &common::account_id(&app, &cookie).await)
        .await
        .unwrap();
    feedback(&app, "x", "y", "z").await;
    submit(&app, &entry("Steering FAKE-ERROR", "")).await;
    app.state.mailer = Arc::new(RefusingFor {
        refused: "second@example.com".to_string(),
        outbox: app.outbox.clone(),
    });

    let failed = send_daily_email(&app.state, soon()).await;
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(pings.sent(), vec![false]);
    assert_eq!(the_report(&app).to, OWNER);

    app.state.mailer = Arc::new(app.outbox.clone());
    let retried = send_daily_email(&app.state, soon() + Duration::seconds(1))
        .await
        .unwrap();

    assert_eq!(retried, DailyRun::Sent { to: 1 });
    assert_eq!(pings.sent(), vec![false, true]);
    let reports = reports(&app.outbox);
    let to: Vec<&str> = reports.iter().map(|email| email.to.as_str()).collect();
    assert_eq!(to, vec![OWNER, "second@example.com"]);
    assert!(
        reports[1].body.contains("Screening: 1 failed call;"),
        "a failed run keeps the events it read: {}",
        reports[1].body
    );
}
