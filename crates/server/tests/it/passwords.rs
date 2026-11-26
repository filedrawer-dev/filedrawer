//! Seams: `passwords::Hasher`, and the router for the routes that hash a password
//! (uncertainties.md, The server's own bounds): hashes run a few at a time, and a
//! request that gets no place in time is refused before anything is counted
//! against it.

use crate::common;

use std::time::Duration;

use axum::http::StatusCode;
use common::through_caddy::{hourly, retry_after, wide};
use common::{PASSWORD, Reply, test_app};
use filedrawer_server::limits::Limits;
use filedrawer_server::passwords::Hasher;

/// A wait too short to matter, for a hasher whose refusals a test wants at once.
const NO_TIME: Duration = Duration::from_millis(1);
/// A wait no test reaches.
const PATIENT: Duration = Duration::from_secs(3600);

/// A hasher with every place taken.
fn full() -> Hasher {
    Hasher::new(0, NO_TIME)
}

#[tokio::test]
async fn no_more_requests_hold_a_place_than_there_are_places() {
    let hasher = Hasher::new(2, NO_TIME);
    let first = hasher.place().await.unwrap();
    let _second = hasher.place().await.unwrap();
    for _ in 0..6 {
        assert!(hasher.place().await.is_err());
    }
    let hash = first.hash(PASSWORD.to_string()).await;
    assert!(hash.starts_with("$argon2id$"), "{hash}");
}

#[tokio::test]
async fn a_place_is_free_again_once_its_hash_ends() {
    let hasher = Hasher::new(1, NO_TIME);
    let place = hasher.place().await.unwrap();
    let hash = place.hash(PASSWORD.to_string()).await;
    let place = hasher.place().await.expect("the first hash ended");
    assert!(place.matches(PASSWORD.to_string(), Some(hash)).await);
}

#[test]
fn a_request_that_goes_away_before_its_hash_ends_keeps_its_place_until_then() {
    // One thread for blocking work, which the test holds, so the hash can't end
    // before the test lets it start.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (let_go, held) = std::sync::mpsc::channel::<()>();
        let holder = tokio::task::spawn_blocking(move || held.recv());
        let hasher = Hasher::new(1, PATIENT);
        let place = hasher.place().await.unwrap();
        let mut hashing = Box::pin(place.hash(PASSWORD.to_string()));
        // Polled once, the hash is handed to the blocking thread; then the request
        // is dropped.
        assert!(futures::poll!(hashing.as_mut()).is_pending());
        drop(hashing);
        let early = tokio::time::timeout(NO_TIME, hasher.place()).await;
        assert!(early.is_err(), "the hash hasn't run yet");
        let_go.send(()).unwrap();
        holder.await.unwrap().unwrap();
        hasher.place().await.expect("free once the hash has run");
    });
}

#[tokio::test]
async fn a_waiting_request_gets_the_next_place() {
    let hasher = Hasher::new(1, PATIENT);
    let first = hasher.place().await.unwrap();
    let (first, second) = tokio::join!(first.hash(PASSWORD.to_string()), async {
        let place = hasher.place().await.expect("waits for the first to end");
        place.hash("another password".to_string()).await
    });
    assert_ne!(first, second);
}

/// A refusal for want of a place: a 429 that says when to try again.
fn assert_busy(reply: &Reply) {
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.body
    );
    assert!(retry_after(reply) >= 1);
    assert!(reply.body.contains("in progress"), "{}", reply.body);
}

#[tokio::test]
async fn every_route_that_hashes_a_password_is_refused_while_no_place_is_free() {
    let app = test_app().await;
    common::signed_up_and_verified(&app, "grace@example.com").await;
    let cookie = common::logged_in(&app, "grace@example.com", PASSWORD).await;
    app.form("/reset", &[("email", "grace@example.com")], None)
        .await;
    let link = app.link_in_last_email_to("grace@example.com").await;
    let token = link.strip_prefix("/reset/confirm?token=").expect(&link);
    let app = app.with_passwords(full());

    let login = [("email", "grace@example.com"), ("password", PASSWORD)];
    assert_busy(&app.form("/login", &login, None).await);
    // An address with no account does the same work, so it waits the same way.
    let unknown = [("email", "nobody@example.com"), ("password", PASSWORD)];
    assert_busy(&app.form("/login", &unknown, None).await);
    let signup = [
        ("email", "ada@example.com"),
        ("password", PASSWORD),
        ("display_name", "Ada"),
    ];
    assert_busy(&app.form("/signup", &signup, None).await);
    let reset = [("token", token), ("password", PASSWORD)];
    assert_busy(&app.form("/reset/confirm", &reset, None).await);
    let delete = [("password", PASSWORD)];
    assert_busy(&app.form("/account/delete", &delete, Some(&cookie)).await);
}

/// A reset link that doesn't work is answered before any hashing, so it takes no
/// place from the logins waiting for one.
#[tokio::test]
async fn a_reset_with_a_link_that_doesnt_work_takes_no_place() {
    let app = test_app().await.with_passwords(full());
    let reset = [("token", "no-such-link"), ("password", PASSWORD)];
    let reply = app.form("/reset/confirm", &reset, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    assert!(
        reply.body.contains("It was used or expired."),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn a_refusal_for_want_of_a_place_counts_against_no_limit() {
    let one_each = Limits {
        failed_logins_per_address: hourly(1),
        signups_per_ip: hourly(1),
        emails_per_address: hourly(1),
        ..wide()
    };
    let app = test_app()
        .await
        .with_limits(one_each)
        .with_passwords(full());
    let wrong = [
        ("email", "grace@example.com"),
        ("password", "a wrong guess"),
    ];
    let signup = [
        ("email", "grace@example.com"),
        ("password", PASSWORD),
        ("display_name", "Grace"),
    ];
    assert_busy(&app.form("/login", &wrong, None).await);
    assert_busy(&app.form("/signup", &signup, None).await);

    let app = app.with_passwords(Hasher::default());
    let counted = app.form("/login", &wrong, None).await;
    assert_eq!(counted.status, StatusCode::UNAUTHORIZED, "{}", counted.body);
    let signed_up = app.form("/signup", &signup, None).await;
    assert_eq!(signed_up.status, StatusCode::OK, "{}", signed_up.body);
}

/// The parameters argon2id records in a hash: `$argon2id$v=19$m=…,t=…,p=…$salt$hash`.
fn parameters(hash: &str) -> &str {
    hash.split('$').nth(3).unwrap()
}

#[tokio::test]
async fn a_hasher_at_minimum_cost_hashes_with_argon2s_least_and_checks_as_any() {
    let hasher = Hasher::new(1, PATIENT).at_minimum_cost();
    let hash = hasher
        .place()
        .await
        .unwrap()
        .hash(PASSWORD.to_string())
        .await;
    assert!(hash.starts_with("$argon2id$v=19$"), "{hash}");
    assert_eq!(parameters(&hash), "m=8,t=1,p=1");
    let check = |password: &str, stored: Option<String>| {
        let password = password.to_string();
        let hasher = &hasher;
        async move {
            hasher
                .place()
                .await
                .unwrap()
                .matches(password, stored)
                .await
        }
    };
    assert!(check(PASSWORD, Some(hash.clone())).await);
    assert!(!check("another password", Some(hash)).await);
    assert!(!check(PASSWORD, None).await);
}

#[tokio::test]
async fn the_sites_hasher_keeps_argon2s_defaults_and_any_hasher_checks_its_hashes() {
    let site = Hasher::new(1, PATIENT);
    let hash = site.place().await.unwrap().hash(PASSWORD.to_string()).await;
    // argon2id's defaults (design.md, Accounts): 19 MiB, 2 passes, 1 lane.
    assert_eq!(parameters(&hash), "m=19456,t=2,p=1");
    let cheap = Hasher::new(1, PATIENT).at_minimum_cost();
    let place = cheap.place().await.unwrap();
    assert!(place.matches(PASSWORD.to_string(), Some(hash)).await);
}
