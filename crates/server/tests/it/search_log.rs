//! The search log (#45): each search's query, how many entries matched and the UTC
//! day, with no IP, account or token, kept 90 days and at most 10,000 a day.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, entry, test_app};
use filedrawer_server::search_log;
use time::{Duration, OffsetDateTime};

/// Each logged search as (day, query, hits), oldest row first.
async fn logged(app: &TestApp) -> Vec<(String, String, i64)> {
    sqlx::query_as("SELECT day, query, hits FROM search_log ORDER BY rowid")
        .fetch_all(&app.pool)
        .await
        .unwrap()
}

fn today() -> String {
    OffsetDateTime::now_utc().date().to_string()
}

async fn search(app: &TestApp, path: &str, headers: &[(&str, &str)]) {
    let mut request = Request::get(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let reply = app.send(request.body(Body::empty()).unwrap()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

/// An app with three public entries titled Lantern and one titled Probes.
async fn app_with_entries() -> TestApp {
    let app = test_app().await;
    for title in ["Lantern one", "Lantern two", "Lantern three", "Probes"] {
        app.submit(&entry(title, "")).await;
    }
    app
}

#[tokio::test]
async fn a_text_search_is_logged_with_its_query_hits_and_day() {
    let app = app_with_entries().await;
    search(&app, "/search?q=Probes", &[]).await;
    assert_eq!(logged(&app).await, vec![(today(), "q=Probes".into(), 1)]);
}

#[tokio::test]
async fn json_and_page_searches_are_logged_too() {
    let app = app_with_entries().await;
    search(&app, "/search?q=Probes", &[("accept", "application/json")]).await;
    search(
        &app,
        "/search?q=nothing+like+this",
        &[("sec-fetch-dest", "document")],
    )
    .await;
    assert_eq!(
        logged(&app).await,
        vec![
            (today(), "q=Probes".into(), 1),
            (today(), "q=nothing+like+this".into(), 0),
        ]
    );
}

#[tokio::test]
async fn the_query_holds_every_filter_in_a_fixed_order_without_limit_or_page() {
    let app = app_with_entries().await;
    search(
        &app,
        "/search?limit=5&tag=steering&outcome=no_effect&q=Lantern&kind=&model=model-a",
        &[],
    )
    .await;
    search(&app, "/search?tag=steering", &[]).await;
    let queries: Vec<String> = logged(&app).await.into_iter().map(|row| row.1).collect();
    assert_eq!(
        queries,
        [
            "q=Lantern&outcome=no_effect&tag=steering&model=model-a",
            "tag=steering"
        ]
    );
}

#[tokio::test]
async fn hits_count_every_match_when_the_first_page_is_full() {
    let app = app_with_entries().await;
    search(&app, "/search?q=Lantern&limit=1", &[]).await;
    search(&app, "/search?q=Lantern&limit=3", &[]).await;
    search(&app, "/search?q=Lantern&limit=2&outcome=no_effect", &[]).await;
    search(&app, "/search?limit=1", &[]).await;
    let hits: Vec<i64> = logged(&app).await.into_iter().map(|row| row.2).collect();
    assert_eq!(hits, [3, 3, 3, 4]);
}

#[tokio::test]
async fn later_pages_continue_a_search_and_are_not_logged() {
    let app = app_with_entries().await;
    search(&app, "/search?q=Lantern&limit=1&page=1", &[]).await;
    search(&app, "/search?q=Lantern&limit=1&page=2", &[]).await;
    search(&app, "/search?q=Lantern&limit=1&page=3", &[]).await;
    assert_eq!(logged(&app).await.len(), 1);
}

#[tokio::test]
async fn a_refused_search_is_not_logged() {
    let app = app_with_entries().await;
    let reply = app.get("/search?outcome=nonsense").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    assert!(logged(&app).await.is_empty());
}

#[tokio::test]
async fn a_search_with_a_token_logs_no_account() {
    let app = app_with_entries().await;
    let reply = app.get_as_author("/search?q=Probes").await;
    assert_eq!(reply.status, StatusCode::OK);
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('search_log') ORDER BY cid")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(columns, ["day", "query", "hits", "client"]);
}

#[tokio::test]
async fn a_query_is_cut_to_500_characters() {
    let app = test_app().await;
    let words = "é".repeat(600);
    search(&app, &format!("/search?tag={words}"), &[]).await;
    let query = &logged(&app).await[0].1;
    assert_eq!(query.chars().count(), 500);
    assert!(query.starts_with("tag=%C3%A9"));
}

#[tokio::test]
async fn a_day_takes_no_more_than_10000_searches() {
    let app = test_app().await;
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 9999)
         INSERT INTO search_log (day, query, hits) SELECT ?, 'q=filler', 0 FROM n",
    )
    .bind(today())
    .execute(&app.pool)
    .await
    .unwrap();
    search(&app, "/search?q=ten+thousandth", &[]).await;
    search(&app, "/search?q=one+too+many", &[]).await;
    let rows = logged(&app).await;
    assert_eq!(rows.len(), 10_000);
    assert_eq!(rows.last().unwrap().1, "q=ten+thousandth");
}

#[tokio::test]
async fn the_tidy_deletes_rows_older_than_90_days() {
    let app = test_app().await;
    let now = OffsetDateTime::now_utc();
    for days in [91, 90, 89, 0] {
        let day = (now - Duration::days(days)).date().to_string();
        sqlx::query("INSERT INTO search_log (day, query, hits) VALUES (?, ?, 0)")
            .bind(&day)
            .bind(format!("q={days}"))
            .execute(&app.pool)
            .await
            .unwrap();
    }
    search_log::tidy(&app.pool, now).await.unwrap();
    let queries: Vec<String> = logged(&app).await.into_iter().map(|row| row.1).collect();
    assert_eq!(queries, ["q=90", "q=89", "q=0"]);
}

#[tokio::test]
async fn a_full_first_pages_count_matches_what_every_page_together_holds() {
    let app = test_app().await;
    for (title, front) in [
        ("Lantern audit", "kind: audit\n"),
        ("Lantern rerun", "replicates: arXiv:2603.20281\n"),
        ("Lantern plain", ""),
        ("Other audit", "kind: audit\nreplicates: arXiv:2603.20281\n"),
        ("Other plain", ""),
    ] {
        app.submit(&entry(title, front)).await;
    }
    let filters = [
        "q=Lantern",
        "kind=audit",
        "cites=arXiv:2603.20281",
        "q=Lantern&kind=audit&cites=arXiv:2603.20281",
        "q=Other&kind=audit",
        "outcome=no_effect&stage=main&tag=steering&model=model-a&row_model=model-c&row_claim=effect",
    ];
    for filter in filters {
        let all = app.get_json(&format!("/search?{filter}&limit=100")).await;
        let expected = i64::try_from(all["results"].as_array().unwrap().len()).unwrap();
        search(&app, &format!("/search?{filter}&limit=1"), &[]).await;
        let hits = logged(&app).await.last().unwrap().2;
        assert_eq!(hits, expected, "{filter}");
    }
}

/// Each logged search's client, oldest row first.
async fn clients(app: &TestApp) -> Vec<Option<String>> {
    sqlx::query_scalar("SELECT client FROM search_log ORDER BY rowid")
        .fetch_all(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_skills_user_agent_is_logged_as_the_client_and_no_other() {
    let app = app_with_entries().await;
    let skill = "filedrawer-skill (codex; gpt-5.2-codex; CODEX_THREAD_ID CODEX_SANDBOX)";
    search(&app, "/search?q=Probes", &[("user-agent", skill)]).await;
    search(
        &app,
        "/search?q=Probes",
        &[(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0",
        )],
    )
    .await;
    search(&app, "/search?q=Probes", &[("user-agent", "curl/8.5.0")]).await;
    search(&app, "/search?q=Probes", &[]).await;
    assert_eq!(
        clients(&app).await,
        [Some(skill.to_string()), None, None, None]
    );
}

#[tokio::test]
async fn a_logged_client_is_cut_to_200_characters() {
    let app = app_with_entries().await;
    let long = format!("filedrawer-skill ({})", "é".repeat(300));
    search(&app, "/search?q=Probes", &[("user-agent", &long)]).await;
    let client = clients(&app).await[0].clone().unwrap();
    assert_eq!(client.chars().count(), 200);
    assert!(client.starts_with("filedrawer-skill (é"));
}
