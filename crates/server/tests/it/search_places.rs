//! Seams: `search::Places`, and the router's `GET /search`, for how many searches
//! with words run at once (uncertainties.md, How long a search's `q` may be, and how
//! long a search may run).

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use filedrawer_server::search::{Busy, Places};

use crate::common::test_app;

#[test]
fn a_client_gets_its_share_of_places_and_no_more() {
    let places = Arc::new(Places::new(3, 2));
    let first = places.place("192.0.2.1").unwrap();
    let _second = places.place("192.0.2.1").unwrap();
    assert_eq!(places.place("192.0.2.1").unwrap_err(), Busy::Client);
    let _other = places.place("192.0.2.2").unwrap();
    assert_eq!(places.place("192.0.2.3").unwrap_err(), Busy::Site);
    drop(first);
    places.place("192.0.2.1").unwrap();
}

/// A search's statement runs on after its client hangs up, so its place stays
/// taken until the statement ends.
#[tokio::test]
async fn a_place_stays_taken_until_its_search_ends_though_its_caller_is_gone() {
    let places = Arc::new(Places::new(1, 1));
    let place = places.place("192.0.2.1").unwrap();
    let (started, running) = tokio::sync::oneshot::channel::<()>();
    let (finish, finished) = tokio::sync::oneshot::channel::<()>();
    let caller = tokio::spawn(place.run(async move {
        let _ = started.send(());
        let _ = finished.await;
    }));
    running.await.unwrap();
    caller.abort();
    let _ = caller.await;
    assert_eq!(places.place("192.0.2.2").unwrap_err(), Busy::Site);
    finish.send(()).unwrap();
    let freed = async {
        while places.place("192.0.2.2").is_err() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(30), freed)
        .await
        .expect("the place comes free once its search ends");
}

#[tokio::test]
async fn a_search_with_words_is_refused_while_no_place_is_free_and_others_still_answer() {
    let app = test_app().await.with_searches(Places::new(0, 1));
    let refused = app.get("/search?q=probes").await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.body
    );
    assert_eq!(
        refused
            .headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("5")
    );
    assert_eq!(
        refused.body,
        "too many requests: searches in progress reached its limit; try again in 5 s\n"
    );
    // Filters alone and the home page look no words up.
    let filtered = app.get("/search?outcome=no_effect").await;
    assert_eq!(filtered.status, StatusCode::OK, "{}", filtered.body);
    let home = app.get("/").await;
    assert_eq!(home.status, StatusCode::OK, "{}", home.body);
}
