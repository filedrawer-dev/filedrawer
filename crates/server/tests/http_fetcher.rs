//! Seam: `HttpFetcher`, the link checks' real HTTP client, against a local server.
//! It must never follow a redirect by itself: the checks follow each hop only when
//! it stays on a listed host.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::IntoResponse;
use axum::routing::get;
use filedrawer_server::link_checks::{Fetcher, HttpFetcher};

/// Every request the server saw: its path and its Authorization header.
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A local server: `/moved` redirects to `/elsewhere`, `/repos/a/b` answers JSON with
/// GitHub's rate-limit headers, and everything else answers 200. Returns its base URL.
async fn serve(seen: Seen) -> String {
    let record = move |uri: Uri, headers: HeaderMap| {
        let seen = seen.clone();
        async move {
            let auth = headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            seen.lock().unwrap().push((uri.path().to_string(), auth));
            match uri.path() {
                "/moved" => (StatusCode::FOUND, [(header::LOCATION, "/elsewhere")]).into_response(),
                "/busy" => (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, "120")],
                )
                    .into_response(),
                "/repos/c/d" => {
                    (StatusCode::FORBIDDEN, [(header::RETRY_AFTER, "30")]).into_response()
                }
                "/slow" => {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    "late".into_response()
                }
                "/repos/a/b" => (
                    [
                        ("x-ratelimit-remaining", "4321"),
                        ("x-ratelimit-reset", "1800000000"),
                    ],
                    axum::Json(serde_json::json!({"status": "identical"})),
                )
                    .into_response(),
                _ => "ok".into_response(),
            }
        }
    };
    let router = Router::new().fallback(get(record));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test]
async fn it_answers_a_redirect_without_following_it() {
    let seen = Seen::default();
    let base = serve(seen.clone()).await;
    let fetcher = HttpFetcher::with_api_base(None, &base, TIMEOUT);
    let answer = fetcher.get(&format!("{base}/moved")).await.unwrap();
    assert_eq!(answer.status, 302);
    assert_eq!(answer.location.as_deref(), Some("/elsewhere"));
    let paths: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    assert_eq!(paths, ["/moved"]);
}

#[tokio::test]
async fn it_sends_the_token_to_githubs_api_only_and_reads_its_quota() {
    let seen = Seen::default();
    let base = serve(seen.clone()).await;
    let fetcher = HttpFetcher::with_api_base(Some("github_pat_not_real".into()), &base, TIMEOUT);
    let answer = fetcher.github("/repos/a/b").await.unwrap();
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body.unwrap()["status"], "identical");
    assert_eq!(answer.remaining, Some(4321));
    assert_eq!(answer.reset, Some(1_800_000_000));
    fetcher.get(&format!("{base}/page")).await.unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (
                "/repos/a/b".to_string(),
                Some("Bearer github_pat_not_real".to_string())
            ),
            ("/page".to_string(), None),
        ]
    );
}

/// Long enough for a loopback answer, short enough for a test.
const TIMEOUT: Duration = Duration::from_millis(500);

#[tokio::test]
async fn a_fetch_that_takes_too_long_is_given_up() {
    let base = serve(Seen::default()).await;
    let fetcher = HttpFetcher::with_api_base(None, &base, TIMEOUT);
    let started = std::time::Instant::now();
    let answer = fetcher.get(&format!("{base}/slow")).await;
    assert_eq!(answer, Err("timed out".to_string()));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn it_reads_retry_after_from_a_page_and_from_the_api() {
    let base = serve(Seen::default()).await;
    let fetcher = HttpFetcher::with_api_base(None, &base, TIMEOUT);
    let page = fetcher.get(&format!("{base}/busy")).await.unwrap();
    assert_eq!((page.status, page.retry_after), (429, Some(120)));
    let api = fetcher.github("/repos/c/d").await.unwrap();
    assert_eq!((api.status, api.retry_after), (403, Some(30)));
}

#[tokio::test]
async fn a_host_that_isnt_there_is_an_error_not_an_answer() {
    let fetcher = HttpFetcher::new(None);
    // Port 9 (discard) on loopback: nothing listens there.
    assert!(fetcher.get("http://127.0.0.1:9/").await.is_err());
}
