//! Seam: the router, for the owner's report of front-matter fields the format
//! doesn't name (design.md, Admin; Feedback and usability cycles): each field with
//! how many accounts and entries used it, the most accounts first, for the owner
//! alone.

use crate::common;

use axum::http::StatusCode;
use common::{Reply, TestApp, browser_account, entry, owner, test_app};
use filedrawer_server::admin;

const REPORT: &str = "/admin/unknown-fields";

/// A token for a second account.
async fn another_account(app: &TestApp, email: &str) -> String {
    let (cookie, _) = browser_account(app, email).await;
    admin::create_token(&app.pool, &common::account_id(app, &cookie).await, "laptop")
        .await
        .unwrap()
}

async fn submit_as(app: &TestApp, token: &str, title: &str, front: &str) {
    let reply = app
        .post_as("/entries", Some(token), entry(title, front))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

/// The text of the first element of `class` in `html`.
fn text_of(html: &str, class: &str) -> String {
    let after = html
        .split_once(&format!(" class=\"{class}\">"))
        .unwrap_or_else(|| panic!("no element of class {class} in {html}"))
        .1;
    after.split('<').next().unwrap().to_string()
}

/// The report's rows in order: each field's name as the page shows it, its
/// accounts and its entries.
fn rows(page: &Reply) -> Vec<(String, String, String)> {
    page.body
        .split("<tr class=\"unknown-field\">")
        .skip(1)
        .map(|row| {
            (
                text_of(row, "name"),
                text_of(row, "accounts"),
                text_of(row, "entries"),
            )
        })
        .collect()
}

fn row(name: &str, accounts: u32, entries: u32) -> (String, String, String) {
    (name.to_string(), accounts.to_string(), entries.to_string())
}

#[tokio::test]
async fn the_report_lists_each_field_with_its_accounts_and_entries_most_accounts_first() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let grace = another_account(&app, "grace@example.com").await;
    submit_as(&app, &app.token, "A", "seed_note: 3 seeds\n").await;
    submit_as(
        &app,
        &app.token,
        "B",
        "seed_note: 5 seeds\ncompute: 4 GPU hours\n",
    )
    .await;
    submit_as(&app, &app.token, "C", "seed_note: 1 seed\n").await;
    submit_as(&app, &grace, "D", "compute: 1 GPU hour\n").await;
    // Used once each: after the others, by name.
    submit_as(&app, &grace, "E", "zeta: 1\nalpha: 2\n").await;
    submit_as(&app, &grace, "F", "").await;

    let page = app.page(REPORT, Some(&cookie)).await;

    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert_eq!(
        rows(&page),
        [
            row("compute", 2, 2),
            row("seed_note", 1, 3),
            row("alpha", 1, 1),
            row("zeta", 1, 1)
        ],
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("GPU hour"),
        "a field's value stays off the page: {}",
        page.body
    );
}

#[tokio::test]
async fn with_no_unknown_field_the_report_has_no_rows() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    submit_as(&app, &app.token, "A", "").await;

    let page = app.page(REPORT, Some(&cookie)).await;

    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert_eq!(rows(&page), []);
}

#[tokio::test]
async fn a_deleted_entrys_fields_leave_the_report() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let held = app.submit_held(&entry("A", "seed_note: 3 seeds\n")).await;
    assert_eq!(
        rows(&app.page(REPORT, Some(&cookie)).await),
        [row("seed_note", 1, 1)]
    );

    let deleted = app
        .delete_as(&format!("/e/{held}"), &app.token, false)
        .await;

    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(rows(&app.page(REPORT, Some(&cookie)).await), []);
}

#[tokio::test]
async fn a_fields_name_is_escaped_and_a_long_one_cut() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let long = "x".repeat(300);
    submit_as(
        &app,
        &app.token,
        "A",
        &format!("\"<b>bold</b>\": 1\n{long}: 2\n"),
    )
    .await;

    let page = app.page(REPORT, Some(&cookie)).await;

    assert!(!page.body.contains("<b>bold"), "{}", page.body);
    let names: Vec<String> = rows(&page).into_iter().map(|row| row.0).collect();
    assert!(names.contains(&format!("{}…", "x".repeat(80))), "{names:?}");
}

/// The front matter of the unknown fields `field_<n>` for each `n` of `numbers`.
fn fields(numbers: std::ops::Range<usize>) -> String {
    numbers.map(|n| format!("field_{n:03}: 1\n")).collect()
}

#[tokio::test]
async fn the_report_lists_200_fields_and_says_when_there_are_more() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    // An entry stores at most 50 of them.
    for first in [0, 50, 100, 150] {
        submit_as(&app, &app.token, "Many", &fields(first..first + 50)).await;
    }

    let all = app.page(REPORT, Some(&cookie)).await;

    assert_eq!(rows(&all).len(), 200);
    assert!(!all.has_class("cut"), "{}", all.body);

    submit_as(&app, &app.token, "One more", &fields(200..201)).await;
    let cut = app.page(REPORT, Some(&cookie)).await;

    assert_eq!(rows(&cut).len(), 200);
    assert!(cut.has_class("cut"), "{}", cut.body);
}

#[tokio::test]
async fn only_the_owner_reaches_the_report_and_the_queue_links_to_it() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (other, _) = browser_account(&app, "mallory@example.com").await;

    assert_eq!(
        app.page(REPORT, Some(&other)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.page(REPORT, None).await.status,
        app.page("/admin", None).await.status
    );
    assert_ne!(app.page(REPORT, None).await.status, StatusCode::OK);
    let token_only = app.get_as_author(REPORT).await;
    assert_eq!(
        token_only.status,
        app.get_as_author("/admin").await.status,
        "{}",
        token_only.body
    );
    assert_ne!(token_only.status, StatusCode::OK, "{}", token_only.body);
    let queue = app.page("/admin", Some(&cookie)).await;
    assert!(
        queue.body.contains(&format!("href=\"{REPORT}\"")),
        "{}",
        queue.body
    );
}

#[tokio::test]
async fn an_entry_stores_its_first_50_unknown_fields() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    submit_as(&app, &app.token, "Many", &fields(0..60)).await;

    let names: Vec<String> = rows(&app.page(REPORT, Some(&cookie)).await)
        .into_iter()
        .map(|row| row.0)
        .collect();

    let first_50: Vec<String> = (0..50).map(|n| format!("field_{n:03}")).collect();
    assert_eq!(names, first_50);
}

#[tokio::test]
async fn a_stored_name_is_cut_to_100_characters() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    // Names that differ only past the cut are stored as one name.
    for (title, end) in [("A", "a"), ("B", "b")] {
        let name = format!("{}{end}", "y".repeat(100));
        submit_as(&app, &app.token, title, &format!("{name}: 1\n")).await;
    }
    // One that differs at the 100th character is another field.
    let other = format!("{}z", "y".repeat(99));
    submit_as(&app, &app.token, "C", &format!("{other}: 1\n")).await;

    let entries: Vec<String> = rows(&app.page(REPORT, Some(&cookie)).await)
        .into_iter()
        .map(|row| row.2)
        .collect();

    assert_eq!(entries, ["2", "1"]);
}
