//! Seams: the router, for which reads of an entry count and how Most read ranks
//! them (design.md, Agent interface; #107), and `reads::record` and `reads::tidy`
//! with a chosen time, for how days close and roll into months.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, browser_account, entry, owner, test_app};
use filedrawer_server::reads::{self, Kind, ReadCounter};
use time::macros::datetime;

const SKILL: &str = "filedrawer-skill (claude-code; claude-sonnet-5-5)";

/// A GET of `path` by the skills, from `ip`, with `token` if given.
fn skill_get(path: &str, ip: &str, token: Option<&str>) -> Request<Body> {
    let mut request = Request::get(path)
        .header("user-agent", SKILL)
        .header("x-forwarded-for", ip);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request.body(Body::empty()).unwrap()
}

/// A browser opening `path` from `ip`, with `cookie` if given.
fn browser_get(path: &str, ip: &str, cookie: Option<&str>) -> Request<Body> {
    let mut request = Request::get(path)
        .header("sec-fetch-dest", "document")
        .header("sec-fetch-mode", "navigate")
        .header("user-agent", "Mozilla/5.0 Firefox/140.0")
        .header("x-forwarded-for", ip);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    request.body(Body::empty()).unwrap()
}

/// Sends `request` over a connection from Caddy on the same host, which forwards the
/// client's address.
async fn send(app: &TestApp, request: Request<Body>) -> common::Reply {
    let reply = app.send_from("127.0.0.1:40000", request).await;
    assert!(reply.status.is_success(), "{}", reply.body);
    reply
}

/// The page of Most read at `path`.
async fn most_read_page(app: &TestApp, path: &str) -> common::Reply {
    let page = app
        .send(
            Request::get(path)
                .header("sec-fetch-dest", "document")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    page
}

/// Most read's first page as (entry ID, readers), in its order.
async fn most_read(app: &TestApp) -> Vec<(String, i64)> {
    ranked(&most_read_page(app, "/?view=read").await.body)
}

/// The cards in `body` as (entry ID, the count its call strip shows).
fn ranked(body: &str) -> Vec<(String, i64)> {
    body.split(r#"<div class="call">"#)
        .skip(1)
        .map(|call| {
            let call = &call[..call.find("</div>").expect("the call's end")];
            let id = call
                .split_once(r#"<a href="/e/"#)
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(id, _)| id.to_string())
                .expect("the card's ID");
            let count = call
                .split_once(r#"<span class="reads">"#)
                .and_then(|(_, rest)| rest.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|digits| digits.parse().ok())
                .unwrap_or_else(|| panic!("no count in {call}"));
            (id, count)
        })
        .collect()
}

#[tokio::test]
async fn a_skill_read_counts_once_per_reader_a_day() {
    let app = test_app().await;
    let id = app.submit(&entry("Read twice", "")).await;
    let path = format!("/e/{id}");
    send(&app, skill_get(&path, "203.0.113.1", None)).await;
    send(&app, skill_get(&path, "203.0.113.1", None)).await;
    send(&app, skill_get(&format!("{path}.txt"), "203.0.113.1", None)).await;
    assert_eq!(most_read(&app).await, [(id, 1)]);
}

#[tokio::test]
async fn readers_count_apart_by_ip_by_user_agent_and_by_account() {
    let app = test_app().await;
    let id = app.submit(&entry("Read by many", "")).await;
    let path = format!("/e/{id}");
    send(&app, skill_get(&path, "203.0.113.1", None)).await;
    send(&app, skill_get(&path, "203.0.113.2", None)).await;
    // The same address, another harness.
    send(
        &app,
        Request::get(&path)
            .header("user-agent", "filedrawer-skill (codex; gpt-6)")
            .header("x-forwarded-for", "203.0.113.1")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    // Two accounts behind one address, as in a sprint room.
    let one = app.other_token().await;
    let two = app.other_token().await;
    send(&app, skill_get(&path, "198.51.100.7", Some(&one))).await;
    send(&app, skill_get(&path, "198.51.100.7", Some(&two))).await;
    assert_eq!(most_read(&app).await, [(id, 5)]);
}

#[tokio::test]
async fn a_page_view_counts_but_other_clients_a_head_and_a_prefetch_dont() {
    let app = test_app().await;
    let id = app.submit(&entry("Opened in a browser", "")).await;
    let path = format!("/e/{id}");
    // A plain-text fetch by a tool that isn't the skills.
    send(
        &app,
        Request::get(&path)
            .header("user-agent", "curl/8.5.0")
            .header("x-forwarded-for", "203.0.113.9")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    send(
        &app,
        Request::head(&path)
            .header("user-agent", SKILL)
            .header("x-forwarded-for", "203.0.113.8")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let mut prefetch = browser_get(&path, "203.0.113.7", None);
    prefetch
        .headers_mut()
        .insert("sec-purpose", "prefetch;prerender".parse().unwrap());
    send(&app, prefetch).await;
    assert_eq!(most_read(&app).await, []);
    send(&app, browser_get(&path, "203.0.113.6", None)).await;
    // A person reading the plain text in a browser.
    send(
        &app,
        browser_get(&format!("{path}.txt"), "203.0.113.5", None),
    )
    .await;
    assert_eq!(most_read(&app).await, [(id, 2)]);
}

#[tokio::test]
async fn the_author_an_admin_and_a_suspended_account_dont_count() {
    let app = test_app().await;
    let id = app.submit(&entry("Read by its author", "")).await;
    let path = format!("/e/{id}");
    send(&app, skill_get(&path, "203.0.113.1", Some(&app.token))).await;
    let owner_cookie = owner(&app).await;
    send(&app, browser_get(&path, "203.0.113.2", Some(&owner_cookie))).await;
    let (cookie, suspended) = browser_account(&app, "mallory@example.com").await;
    let account = common::account_id(&app, &cookie).await;
    app.form(
        &format!("/admin/accounts/{account}/suspend"),
        &[],
        Some(&owner_cookie),
    )
    .await;
    send(&app, skill_get(&path, "203.0.113.3", Some(&suspended))).await;
    assert_eq!(most_read(&app).await, []);
}

#[tokio::test]
async fn a_read_of_an_old_version_goes_to_the_latest() {
    let app = test_app().await;
    let first = app.submit(&entry("Steering", "")).await;
    let reply = app
        .post(
            &format!("/e/{first}/versions"),
            entry("Steering, corrected", ""),
        )
        .await;
    let second = common::id_in(&reply.body);
    app.post(&format!("/e/{second}/publish"), "").await;
    send(&app, skill_get(&format!("/e/{first}"), "203.0.113.1", None)).await;
    send(
        &app,
        skill_get(&format!("/e/{second}"), "203.0.113.2", None),
    )
    .await;
    assert_eq!(most_read(&app).await, [(second, 2)]);
}

#[tokio::test]
async fn most_read_ranks_by_readers_then_newest_and_leaves_out_the_unread() {
    let app = test_app().await;
    let unread = app.submit(&entry("Nobody read this", "")).await;
    let once = app.submit(&entry("Read once", "")).await;
    let twice = app.submit(&entry("Read twice", "")).await;
    let also_once = app.submit(&entry("Also read once, later", "")).await;
    for (id, ips) in [
        (&once, &["203.0.113.1"][..]),
        (&twice, &["203.0.113.1", "203.0.113.2"][..]),
        (&also_once, &["203.0.113.1"][..]),
    ] {
        for ip in ips {
            send(&app, skill_get(&format!("/e/{id}"), ip, None)).await;
        }
    }
    let ranking = most_read(&app).await;
    assert_eq!(ranking, [(twice, 2), (also_once, 1), (once, 1)]);
    assert!(!ranking.iter().any(|(id, _)| *id == unread));
}

#[tokio::test]
async fn most_read_with_no_reads_says_so_and_has_its_tab() {
    let app = test_app().await;
    app.submit(&entry("Unread", "")).await;
    let page = most_read_page(&app, "/?view=read").await;
    assert!(page.has_class("empty"), "{}", page.body);
    assert!(page.has_class("tabnote"), "{}", page.body);
    assert!(
        page.body.contains(r#"href="/?view=read" aria-current"#),
        "{}",
        page.body
    );
    let home = most_read_page(&app, "/").await;
    assert!(!home.has_class("tabnote"), "{}", home.body);
}

#[tokio::test]
async fn a_page_past_the_last_doesnt_say_nothing_was_read() {
    let app = test_app().await;
    let id = app.submit(&entry("Read once", "")).await;
    send(&app, skill_get(&format!("/e/{id}"), "203.0.113.1", None)).await;
    let first = most_read_page(&app, "/?view=read").await;
    let past = most_read_page(&app, "/?view=read&page=3").await;
    assert_eq!(empty_text(&first.body), None);
    assert!(empty_text(&past.body).is_some(), "{}", past.body);
    let unread = test_app().await;
    let none = most_read_page(&unread, "/?view=read").await;
    assert_ne!(empty_text(&none.body), empty_text(&past.body));
}

/// The text of `body`'s `empty` paragraph, if it has one.
fn empty_text(body: &str) -> Option<&str> {
    let (_, rest) = body.split_once(r#"<p class="empty">"#)?;
    rest.split_once("</p>").map(|(text, _)| text)
}

#[tokio::test]
async fn most_read_pages_on_past_twenty() {
    let app = test_app().await;
    let other = app.other_token().await;
    let mut ids = Vec::new();
    for n in 0..21 {
        let id = if n < 20 {
            app.submit(&entry(&format!("Result {n}"), "")).await
        } else {
            let reply = app
                .post_as("/entries", Some(&other), entry("Result 20", ""))
                .await;
            let id = common::id_in(&reply.body);
            app.post_as(&format!("/e/{id}/publish"), Some(&other), "")
                .await;
            id
        };
        send(&app, skill_get(&format!("/e/{id}"), "203.0.113.1", None)).await;
        ids.push(id);
    }
    let first = most_read_page(&app, "/?view=read").await;
    assert_eq!(ranked(&first.body).len(), 20);
    assert!(
        first.body.contains(r#"href="/?view=read&#38;page=2""#),
        "{}",
        first.body
    );
    let second = most_read_page(&app, "/?view=read&page=2").await;
    // Ties go to the newest, so the oldest is last.
    assert_eq!(ranked(&second.body), [(ids[0].clone(), 1)]);
    assert!(
        second.body.contains(r#"href="/?view=read""#),
        "{}",
        second.body
    );
}

async fn seq_of(app: &TestApp, id: &str) -> i64 {
    sqlx::query_scalar("SELECT seq FROM entries WHERE id = ?")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// What the owner reads through SQL: an entry's readers by day, kind and count,
/// once its days are closed.
async fn daily(app: &TestApp, seq: i64) -> Vec<(String, String, i64)> {
    sqlx::query_as(
        "SELECT day, kind, readers FROM entry_reads_daily WHERE entry_seq = ?
         ORDER BY day, kind",
    )
    .bind(seq)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_day_closes_after_ten_past_midnight_into_counts_and_keeps_no_reader() {
    let app = test_app().await;
    let id = app.submit(&entry("Read on a day", "")).await;
    let seq = seq_of(&app, &id).await;
    let counter = ReadCounter::default();
    let day = datetime!(2026-11-19 12:00 UTC);
    for reader in ["a", "b"] {
        reads::record(&app.pool, &counter, seq, Kind::Agent, reader, day)
            .await
            .unwrap();
    }
    reads::record(&app.pool, &counter, seq, Kind::Page, "a", day)
        .await
        .unwrap();
    reads::tidy(&app.pool, datetime!(2026-11-20 00:05 UTC))
        .await
        .unwrap();
    assert_eq!(daily(&app, seq).await, []);
    // A read just before midnight that lands late still counts for its day.
    reads::record(
        &app.pool,
        &counter,
        seq,
        Kind::Agent,
        "c",
        datetime!(2026-11-19 23:59:59 UTC),
    )
    .await
    .unwrap();
    for _ in 0..2 {
        reads::tidy(&app.pool, datetime!(2026-11-20 00:11 UTC))
            .await
            .unwrap();
    }
    assert_eq!(
        daily(&app, seq).await,
        [
            ("2026-11-19".to_string(), "agent".to_string(), 3),
            ("2026-11-19".to_string(), "page".to_string(), 1),
        ]
    );
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entry_readers")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn days_leaving_the_window_add_into_their_month() {
    let app = test_app().await;
    let id = app.submit(&entry("Read long ago", "")).await;
    let seq = seq_of(&app, &id).await;
    let counter = ReadCounter::default();
    for (at, reader) in [
        (datetime!(2026-10-01 09:00 UTC), "a"),
        (datetime!(2026-10-02 09:00 UTC), "a"),
        (datetime!(2026-10-02 10:00 UTC), "b"),
    ] {
        reads::record(&app.pool, &counter, seq, Kind::Agent, reader, at)
            .await
            .unwrap();
    }
    // On 2026-10-31 the window, that day and the 29 before, reaches back to 10-02.
    for _ in 0..2 {
        reads::tidy(&app.pool, datetime!(2026-10-31 01:00 UTC))
            .await
            .unwrap();
    }
    assert_eq!(
        daily(&app, seq).await,
        [("2026-10-02".to_string(), "agent".to_string(), 2)]
    );
    let monthly: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT month, kind, readers FROM entry_reads_monthly WHERE entry_seq = ?")
            .bind(seq)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(monthly, [("2026-10".to_string(), "agent".to_string(), 1)]);
}

#[tokio::test]
async fn past_the_daily_cap_reads_go_uncounted_and_are_counted_as_dropped() {
    let app = test_app().await;
    let id = app.submit(&entry("Read too often", "")).await;
    let seq = seq_of(&app, &id).await;
    let counter = ReadCounter::new(2);
    let at = datetime!(2026-11-19 12:00 UTC);
    for reader in ["a", "b", "c", "d", "d"] {
        reads::record(&app.pool, &counter, seq, Kind::Agent, reader, at)
            .await
            .unwrap();
    }
    reads::tidy(&app.pool, datetime!(2026-11-20 01:00 UTC))
        .await
        .unwrap();
    let day: (i64, i64) =
        sqlx::query_as("SELECT readers, dropped FROM read_days WHERE day = '2026-11-19'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(day, (2, 2));
}
