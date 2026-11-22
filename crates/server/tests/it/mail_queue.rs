//! Seam: the router and `mail_queue::send_due`, for the outgoing mail queue (#103;
//! design.md, Hosting and operations): a failed send is retried, a row past a day is
//! dropped, a sent row is gone, and no row holds a link or an address.

use crate::common;

use axum::http::StatusCode;
use common::{PASSWORD, TestApp, field, logged_in, stamp, test_app};
use filedrawer_server::mail::{Email, MailError};
use filedrawer_server::mail_queue::{self, Queued, Run};
use time::{Duration, OffsetDateTime};

/// A row of `outgoing_mail`, as a test reads it.
#[derive(Debug, sqlx::FromRow)]
struct Row {
    kind: String,
    subject: Option<String>,
    body: Option<String>,
    tries: i64,
    last_error: Option<String>,
    dropped_at: Option<String>,
}

async fn rows(app: &TestApp) -> Vec<Row> {
    sqlx::query_as(
        "SELECT kind, subject, body, tries, last_error, dropped_at FROM outgoing_mail ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

/// The email links that would still work.
async fn live_links(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM email_tokens WHERE used_at IS NULL")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn sign_up(app: &TestApp, email: &str) {
    let reply = app
        .form(
            "/signup",
            &[
                ("email", email),
                ("password", PASSWORD),
                ("display_name", "Grace"),
            ],
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

async fn run(app: &TestApp, at: OffsetDateTime) -> Run {
    mail_queue::send_due(&app.state, at).await.unwrap()
}

/// A mailer with no SMTP server to reach.
fn unreachable(_: &Email) -> Option<MailError> {
    Some(MailError::Unreachable("connection refused".into()))
}

#[tokio::test]
async fn a_failed_send_is_retried_after_a_wait_that_doubles_and_then_its_row_is_gone() {
    let app = test_app().await;
    app.outbox.fail(Some(unreachable));
    sign_up(&app, "grace@example.com").await;
    // After the signup, whose row is due from the second it was stamped.
    let now = OffsetDateTime::now_utc();

    let failed = run(&app, now).await;

    assert!(failed.stopped && failed.sent == 0, "{failed:?}");
    let [row] = &rows(&app).await[..] else {
        panic!("one row")
    };
    assert_eq!(row.last_error.as_deref(), Some("connect"));
    // The link minted for the email that didn't go no longer works.
    assert_eq!(live_links(&app).await, 0);

    // A minute's wait, then two.
    let again = now + Duration::seconds(61);
    assert!(run(&app, now + Duration::seconds(30)).await.stopped);
    assert!(run(&app, again).await.stopped);
    app.outbox.fail(None);
    let waiting = run(&app, again + Duration::seconds(90)).await;
    assert!(waiting.stopped && waiting.sent == 0, "{waiting:?}");
    assert!(app.outbox.sent().is_empty());

    let sent = run(&app, again + Duration::seconds(121)).await;

    assert_eq!(sent.sent, 1, "{sent:?}");
    assert!(rows(&app).await.is_empty());
    assert_eq!(live_links(&app).await, 1);
    let link = app.link_in_last_email_to("grace@example.com").await;
    let token = link.replace("/verify?token=", "");
    let verified = app.form("/verify", &[("token", &token)], None).await;
    assert!(verified.status.is_redirection() || verified.status.is_success());
}

#[tokio::test]
async fn a_waiting_row_holds_no_link_or_address() {
    let app = test_app().await;
    app.outbox.fail(Some(unreachable));
    sign_up(&app, "grace@example.com").await;
    run(&app, OffsetDateTime::now_utc()).await;

    let [row] = &rows(&app).await[..] else {
        panic!("one row")
    };
    assert_eq!(row.kind, "verify");
    assert_eq!((&row.subject, &row.body), (&None, &None));
    let everything: String = sqlx::query_scalar(
        "SELECT account_id || kind || created_at || next_try_at || COALESCE(last_error, '')
         FROM outgoing_mail",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(!everything.contains("grace"), "{everything}");
}

#[tokio::test]
async fn a_row_queued_over_a_day_ago_is_dropped_unsent() {
    let app = test_app().await;
    sign_up(&app, "grace@example.com").await;
    let now = OffsetDateTime::now_utc();
    sqlx::query("UPDATE outgoing_mail SET created_at = ?")
        .bind(stamp(now - Duration::hours(25)))
        .execute(&app.pool)
        .await
        .unwrap();

    let dropped = run(&app, now).await;

    assert_eq!(dropped, Run::default());
    assert!(app.outbox.sent().is_empty());
    let [row] = &rows(&app).await[..] else {
        panic!("one row")
    };
    assert!(row.dropped_at.is_some(), "{row:?}");
}

#[tokio::test]
async fn a_refused_recipient_is_dropped_and_the_run_goes_on() {
    let app = test_app().await;
    app.outbox.fail(Some(|email| {
        email
            .to
            .starts_with("ada")
            .then_some(MailError::Reply { code: 550 })
    }));
    sign_up(&app, "ada@example.com").await;
    sign_up(&app, "grace@example.com").await;

    let done = run(&app, OffsetDateTime::now_utc()).await;

    assert_eq!(
        done,
        Run {
            sent: 1,
            dropped: 1,
            stopped: false
        }
    );
    let sent = app.outbox.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, "grace@example.com");
    let [row] = &rows(&app).await[..] else {
        panic!("one row")
    };
    assert_eq!(row.last_error.as_deref(), Some("refused 550"));
    assert!(row.dropped_at.is_some(), "{row:?}");
}

#[tokio::test]
async fn a_failure_of_the_servers_stops_the_run_and_leaves_every_row_due() {
    let app = test_app().await;
    app.outbox
        .fail(Some(|_| Some(MailError::Reply { code: 535 })));
    sign_up(&app, "ada@example.com").await;
    sign_up(&app, "grace@example.com").await;
    let now = OffsetDateTime::now_utc();

    let done = run(&app, now).await;

    assert!(
        done.stopped && done.sent == 0 && done.dropped == 0,
        "{done:?}"
    );
    let tries: Vec<(i64, Option<String>)> = rows(&app)
        .await
        .into_iter()
        .map(|row| (row.tries, row.last_error))
        .collect();
    assert_eq!(tries, [(0, Some("smtp 535".to_string())), (0, None)]);
    // Once the server's wait passes, both go in one run.
    app.outbox.fail(None);
    assert_eq!(run(&app, now + Duration::seconds(61)).await.sent, 2);
}

#[tokio::test]
async fn an_email_the_server_turns_away_for_now_waits_alone_while_the_run_goes_on() {
    let app = test_app().await;
    app.outbox.fail(Some(|email| {
        email
            .to
            .starts_with("ada")
            .then_some(MailError::Reply { code: 452 })
    }));
    sign_up(&app, "ada@example.com").await;
    sign_up(&app, "grace@example.com").await;
    let now = OffsetDateTime::now_utc();

    let done = run(&app, now).await;

    assert_eq!(
        done,
        Run {
            sent: 1,
            dropped: 0,
            stopped: false
        }
    );
    let [row] = &rows(&app).await[..] else {
        panic!("one row")
    };
    assert_eq!(
        (row.tries, row.last_error.as_deref()),
        (1, Some("smtp 452"))
    );
    // Other mail isn't held up, and the row keeps its own wait.
    sign_up(&app, "lin@example.com").await;
    assert_eq!(run(&app, now + Duration::seconds(30)).await.sent, 1);
    app.outbox.fail(None);
    assert_eq!(run(&app, now + Duration::seconds(61)).await.sent, 1);
}

#[tokio::test]
async fn asking_again_while_mail_is_down_sends_one_verify_and_one_reset_email() {
    let app = test_app().await;
    app.outbox.fail(Some(unreachable));
    sign_up(&app, "grace@example.com").await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    app.form("/account/verify-email", &[], Some(&cookie)).await;
    for _ in 0..2 {
        app.form("/reset", &[("email", "grace@example.com")], None)
            .await;
    }
    assert_eq!(kinds(&app).await, ["reset", "verify"]);

    app.outbox.fail(None);
    let sent = app.sent().await;

    assert_eq!(sent.len(), 2, "{sent:#?}");
}

#[tokio::test]
async fn signups_with_a_taken_address_while_mail_is_down_send_one_warning() {
    let app = test_app().await;
    sign_up(&app, "grace@example.com").await;
    app.sent().await;
    app.outbox.fail(Some(unreachable));
    for _ in 0..2 {
        sign_up(&app, "grace@example.com").await;
    }
    assert_eq!(kinds(&app).await, ["taken"]);

    app.outbox.fail(None);
    let sent = app.sent().await;

    assert_eq!(sent.len(), 2, "{sent:#?}");
}

/// The kinds of the waiting rows, sorted.
async fn kinds(app: &TestApp) -> Vec<String> {
    let mut kinds: Vec<String> = rows(app).await.into_iter().map(|row| row.kind).collect();
    kinds.sort();
    kinds
}

#[tokio::test]
async fn a_row_with_no_one_left_to_tell_is_deleted_unsent() {
    let app = test_app().await;
    sign_up(&app, "grace@example.com").await;
    let link = app.link_in_last_email_to("grace@example.com").await;
    let token = link.replace("/verify?token=", "");
    app.form("/verify", &[("token", &token)], None).await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;
    let grace = common::account_id(&app, &cookie).await;
    let before = app.outbox.sent().len();

    // Verified already, and no admin.
    mail_queue::queue(&app.state, &grace, Queued::Verify)
        .await
        .unwrap();
    let notice = Queued::Notice {
        subject: "S".into(),
        body: "B".into(),
    };
    mail_queue::queue(&app.state, &grace, notice).await.unwrap();

    assert_eq!(app.sent().await.len(), before);
    assert!(rows(&app).await.is_empty());
}

#[tokio::test]
async fn the_tidier_deletes_rows_dropped_a_week_ago() {
    let app = test_app().await;
    sign_up(&app, "ada@example.com").await;
    sign_up(&app, "grace@example.com").await;
    let now = OffsetDateTime::now_utc();
    for (email, dropped) in [("ada@example.com", 8), ("grace@example.com", 6)] {
        sqlx::query(
            "UPDATE outgoing_mail SET dropped_at = ?
             WHERE account_id = (SELECT id FROM accounts WHERE email = ?)",
        )
        .bind(stamp(now - Duration::days(dropped)))
        .bind(email)
        .execute(&app.pool)
        .await
        .unwrap();
    }

    mail_queue::tidy(&app.pool, now).await.unwrap();

    assert_eq!(rows(&app).await.len(), 1);
}

#[tokio::test]
async fn deleting_an_account_deletes_its_waiting_email() {
    let app = test_app().await;
    app.outbox.fail(Some(unreachable));
    sign_up(&app, "grace@example.com").await;
    let cookie = logged_in(&app, "grace@example.com", PASSWORD).await;

    let deleted = app
        .form("/account/delete", &[("password", PASSWORD)], Some(&cookie))
        .await;

    assert!(deleted.status.is_success(), "{}", deleted.body);
    assert!(rows(&app).await.is_empty());
}

#[tokio::test]
async fn status_shows_the_waiting_email_and_the_last_failures_class() {
    let app = test_app().await;
    app.outbox.fail(Some(unreachable));
    sign_up(&app, "grace@example.com").await;
    let now = OffsetDateTime::now_utc();
    run(&app, now).await;

    let status = app.get("/staging/status").await;

    let waiting = field(&status.body, "emails waiting").unwrap();
    assert!(waiting.starts_with("1, the oldest queued "), "{waiting}");
    assert_eq!(
        field(&status.body, "email last failure"),
        Some(format!("{}: connect", stamp(now)).as_str())
    );
    assert!(!status.body.contains("grace"), "{}", status.body);
}

/// An account stored before signup took only a bare address may hold another
/// spelling of one, which no email goes to: its row is deleted, not counted as
/// dropped mail.
#[tokio::test]
async fn an_account_whose_stored_address_is_no_bare_address_gets_no_email() {
    let app = test_app().await;
    sign_up(&app, "grace@example.com").await;
    let sent = app.sent().await.len();
    let stored = "x<ada@example.com>";
    sqlx::query("UPDATE accounts SET email = ? WHERE email = 'grace@example.com'")
        .bind(stored)
        .execute(&app.pool)
        .await
        .unwrap();
    let reply = app.form("/reset", &[("email", stored)], None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(app.sent().await.len(), sent);
    assert!(rows(&app).await.is_empty());
    // Its password still logs it in.
    logged_in(&app, stored, PASSWORD).await;
}

#[test]
fn the_mailer_sends_to_the_address_signup_stored_and_to_no_other_spelling() {
    use filedrawer_server::mail::recipient;
    for address in [
        "ada+tag@example.com",
        "o'brien@sub.example.co.uk",
        "a.b-c_d@xn--exmple-cua.com",
        "!#$%&'*+-/=?^_`{|}~@example.com",
    ] {
        let mailbox = recipient(address).expect(address);
        assert_eq!(mailbox.name, None, "{address}");
        assert_eq!(mailbox.email.to_string(), address);
    }
    for other in [
        "x<ada@example.com>",
        "<ada@example.com>",
        "\"ada\"@example.com",
        "\"x\"<ada@example.com>",
        "ada@example.com,grace@example.com",
    ] {
        assert!(
            matches!(recipient(other), Err(MailError::Address(_))),
            "{other}"
        );
    }
}
