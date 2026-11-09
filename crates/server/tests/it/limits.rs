//! Seam: the router, for the rate limits (design.md, Limits and abuse; issue #14):
//! every request per client IP, token-authenticated writes per account, and signups
//! and account emails per IP and per address.

use crate::common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::through_caddy::{
    CADDY, get_from, hourly, per_minute, reset_from, retry_after, sign_up_from, wide,
};
use common::{Reply, TestApp, test_app};
use filedrawer_server::limits::Limits;

async fn log_in(app: &TestApp, email: &str, password: &str) -> Reply {
    app.form("/login", &[("email", email), ("password", password)], None)
        .await
}

#[tokio::test]
async fn failed_logins_lock_the_address_for_the_window_right_password_or_not() {
    let app = test_app().await.with_limits(Limits {
        failed_logins_per_address: hourly(2),
        ..wide()
    });
    common::signed_up_and_verified(&app, "grace@example.com").await;
    // Successes don't count.
    for _ in 0..3 {
        assert_eq!(
            log_in(&app, "grace@example.com", common::PASSWORD)
                .await
                .status,
            StatusCode::SEE_OTHER
        );
    }
    for _ in 0..2 {
        let wrong = log_in(&app, "grace@example.com", "wrong guess").await;
        assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.body);
    }
    let locked = log_in(&app, "grace@example.com", common::PASSWORD).await;
    assert_eq!(
        locked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        locked.body
    );
    assert!(retry_after(&locked) > 60);
    assert!(locked.body.contains("failed logins"), "{}", locked.body);
}

#[tokio::test]
async fn a_burst_of_wrong_guesses_gets_no_more_than_the_limit() {
    let app = test_app().await.with_limits(Limits {
        failed_logins_per_address: hourly(2),
        ..wide()
    });
    common::signed_up_and_verified(&app, "grace@example.com").await;
    let guesses = (0..8).map(|n| {
        let app = &app;
        async move {
            log_in(app, "grace@example.com", &format!("guess {n}"))
                .await
                .status
        }
    });
    let statuses = futures::future::join_all(guesses).await;
    let judged = statuses
        .iter()
        .filter(|s| **s == StatusCode::UNAUTHORIZED)
        .count();
    assert_eq!(judged, 2, "{statuses:?}");
    assert!(
        statuses
            .iter()
            .all(|s| *s == StatusCode::UNAUTHORIZED || *s == StatusCode::TOO_MANY_REQUESTS)
    );
}

#[tokio::test]
async fn an_address_with_no_account_locks_the_same_way() {
    let app = test_app().await.with_limits(Limits {
        failed_logins_per_address: hourly(2),
        ..wide()
    });
    for _ in 0..2 {
        assert_eq!(
            log_in(&app, "nobody@example.com", "guess").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let locked = log_in(&app, "nobody@example.com", "guess").await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    let other = log_in(&app, "someone@example.com", "guess").await;
    assert_eq!(other.status, StatusCode::UNAUTHORIZED);
}

/// GETs /tags from `forwarded` through Caddy.
async fn read_as(app: &TestApp, forwarded: &str) -> Reply {
    get_from(app, forwarded, "/tags").await
}

#[tokio::test]
async fn a_client_past_its_rate_waits_and_others_dont() {
    let app = test_app().await.with_limits(Limits {
        per_ip: per_minute(3),
        ..wide()
    });
    for _ in 0..3 {
        assert_eq!(read_as(&app, "203.0.113.5").await.status, StatusCode::OK);
    }
    let refused = read_as(&app, "203.0.113.5").await;
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        refused.body
    );
    // One request comes back every 20 seconds.
    assert!(
        (1..=20).contains(&retry_after(&refused)),
        "{}",
        retry_after(&refused)
    );
    assert!(
        refused.body.contains("too many requests"),
        "{}",
        refused.body
    );
    assert_eq!(read_as(&app, "203.0.113.6").await.status, StatusCode::OK);
}

#[tokio::test]
async fn forwarded_for_counts_only_from_the_proxy_on_the_same_host() {
    let app = test_app().await.with_limits(Limits {
        per_ip: per_minute(2),
        ..wide()
    });
    // From elsewhere, the header is the client's own claim: the peer is the key.
    for (n, forwarded) in ["198.51.100.1", "198.51.100.2", "198.51.100.3"]
        .iter()
        .enumerate()
    {
        let request = Request::get("/tags")
            .header("x-forwarded-for", *forwarded)
            .body(Body::empty())
            .unwrap();
        let reply = app.send_from("192.0.2.9:4000", request).await;
        let expected = if n < 2 {
            StatusCode::OK
        } else {
            StatusCode::TOO_MANY_REQUESTS
        };
        assert_eq!(reply.status, expected, "{forwarded}");
    }
    // Through Caddy, the last address is the one Caddy saw.
    assert_eq!(
        read_as(&app, "192.0.2.9, 203.0.113.7").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "192.0.2.9, 203.0.113.7").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "192.0.2.9, 203.0.113.7").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn ipv6_clients_share_their_64() {
    let app = test_app().await.with_limits(Limits {
        per_ip: per_minute(2),
        ..wide()
    });
    assert_eq!(
        read_as(&app, "2001:db8:1:2::1").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "2001:db8:1:2::ffff").await.status,
        StatusCode::OK
    );
    assert_eq!(
        read_as(&app, "2001:db8:1:2:aaaa::3").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        read_as(&app, "2001:db8:1:3::1").await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn an_accounts_writes_are_limited_refused_ones_included() {
    let app = test_app().await.with_limits(Limits {
        writes_per_account: per_minute(2),
        ..wide()
    });
    // Each is refused by the mechanical checks, which costs no allowance.
    for _ in 0..2 {
        let refused = app.post("/entries", "not an entry").await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            refused.body
        );
    }
    let limited = app.post("/entries", "not an entry").await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert!(retry_after(&limited) >= 1);
    // Reads with the token aren't writes.
    assert_eq!(app.get("/tags").await.status, StatusCode::OK);
    // Another account's writes have their own count.
    let other = app.other_token().await;
    let theirs = app.post_as("/entries", Some(&other), "not an entry").await;
    assert_eq!(
        theirs.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        theirs.body
    );
}

#[tokio::test]
async fn signups_are_limited_per_ip_and_survive_a_restart() {
    let app = test_app().await.with_limits(Limits {
        signups_per_ip: hourly(2),
        ..wide()
    });
    for n in 0..2 {
        let reply = sign_up_from(&app, "203.0.113.5", &format!("a{n}@example.com"), None).await;
        assert!(
            reply.status.is_success() || reply.status.is_redirection(),
            "{}",
            reply.body
        );
    }
    let limited = sign_up_from(&app, "203.0.113.5", "a9@example.com", None).await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert!(retry_after(&limited) > 60, "{}", retry_after(&limited));
    // The count is kept in the database, so a deploy doesn't reset it.
    let restarted = app.with_limits(Limits {
        signups_per_ip: hourly(2),
        ..wide()
    });
    let again = sign_up_from(&restarted, "203.0.113.5", "a8@example.com", None).await;
    assert_eq!(
        again.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        again.body
    );
    let elsewhere = sign_up_from(&restarted, "203.0.113.6", "a7@example.com", None).await;
    assert!(elsewhere.status.is_success() || elsewhere.status.is_redirection());
}

#[tokio::test]
async fn account_emails_are_limited_per_address_whether_or_not_it_has_an_account() {
    let app = test_app().await.with_limits(Limits {
        emails_per_address: hourly(2),
        ..wide()
    });
    let nobody = "nobody@example.com";
    for forwarded in ["203.0.113.5", "203.0.113.6"] {
        assert_eq!(
            reset_from(&app, forwarded, nobody).await.status,
            StatusCode::OK
        );
    }
    // From a third IP, the address is still full.
    let limited = reset_from(&app, "203.0.113.7", nobody).await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert_eq!(
        reset_from(&app, "203.0.113.7", "other@example.com")
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn account_emails_are_limited_per_ip() {
    let app = test_app().await.with_limits(Limits {
        emails_per_ip: hourly(2),
        ..wide()
    });
    for n in 0..2 {
        let reply = reset_from(&app, "203.0.113.5", &format!("p{n}@example.com")).await;
        assert_eq!(reply.status, StatusCode::OK);
    }
    let limited = reset_from(&app, "203.0.113.5", "p9@example.com").await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
}

#[tokio::test]
async fn a_429_speaks_json_when_asked() {
    let app = test_app().await.with_limits(Limits {
        per_ip: per_minute(1),
        ..wide()
    });
    read_as(&app, "203.0.113.5").await;
    let request = Request::get("/tags")
        .header("x-forwarded-for", "203.0.113.5")
        .header("accept", "application/json")
        .body(Body::empty())
        .unwrap();
    let refused = app.send_from(CADDY, request).await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    let body: serde_json::Value = serde_json::from_str(&refused.body).unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("too many requests"),
        "{body}"
    );
    assert!(retry_after(&refused) >= 1);
}

#[tokio::test]
async fn a_signup_the_email_limit_stops_says_so() {
    let app = test_app().await.with_limits(Limits {
        emails_per_address: hourly(1),
        ..wide()
    });
    assert_eq!(
        reset_from(&app, "203.0.113.5", "ada@example.com")
            .await
            .status,
        StatusCode::OK
    );
    let limited = sign_up_from(&app, "203.0.113.6", "ada@example.com", None).await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert!(
        limited.body.contains("emails to this email address"),
        "{}",
        limited.body
    );
}

#[tokio::test]
async fn resending_a_verification_counts_as_an_email() {
    let app = test_app().await.with_limits(Limits {
        emails_per_address: hourly(2),
        ..wide()
    });
    // The signup sent one; one resend fits, a second doesn't.
    let signup = app
        .form(
            "/signup",
            &[
                ("email", "grace@example.com"),
                ("password", common::PASSWORD),
                ("display_name", "Grace"),
            ],
            None,
        )
        .await;
    assert_eq!(signup.status, StatusCode::OK, "{}", signup.body);
    let cookie = common::logged_in(&app, "grace@example.com", common::PASSWORD).await;
    let resent = app.form("/account/verify-email", &[], Some(&cookie)).await;
    assert_eq!(resent.status, StatusCode::SEE_OTHER, "{}", resent.body);
    let limited = app.form("/account/verify-email", &[], Some(&cookie)).await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
}

mod windows {
    //! `count_event` with times of the test's choosing.

    use filedrawer_server::limits::{Counted, Limits, count_event, tidy};
    use time::macros::datetime;

    use super::*;

    fn one_an_hour() -> Limits {
        Limits {
            emails_per_address: hourly(1),
            ..wide()
        }
    }

    #[tokio::test]
    async fn a_full_window_refuses_even_in_its_last_second() {
        let app = test_app().await;
        let limits = one_an_hour();
        let counted = [Counted::emails_to(&limits, "ada@example.com")];
        count_event(&app.pool, &counted, datetime!(2026-11-06 11:00:01 UTC))
            .await
            .unwrap();
        // Half a second before the first leaves the window, it is still full.
        let edge = count_event(&app.pool, &counted, datetime!(2026-11-06 12:00:00.5 UTC)).await;
        let error = edge.expect_err("the window is still full");
        assert!(error.to_string().contains("try again in 1 s"), "{error}");
        count_event(&app.pool, &counted, datetime!(2026-11-06 12:00:02 UTC))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn counts_keep_no_address_in_plain_and_go_after_a_day() {
        let app = test_app().await;
        let limits = one_an_hour();
        let counted = [Counted::emails_to(&limits, "ada@example.com")];
        count_event(&app.pool, &counted, datetime!(2026-11-06 11:00:00 UTC))
            .await
            .unwrap();
        let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM rate_events")
            .fetch_all(&app.pool)
            .await
            .unwrap();
        assert_eq!(keys.len(), 1);
        assert!(!keys[0].contains("ada"), "{keys:?}");
        tidy(&app.pool, datetime!(2026-11-07 10:59:59 UTC))
            .await
            .unwrap();
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rate_events")
            .fetch_one(&app.pool)
            .await
            .unwrap();
        assert_eq!(kept, 1, "a day hasn't passed");
        tidy(&app.pool, datetime!(2026-11-07 11:00:00 UTC))
            .await
            .unwrap();
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rate_events")
            .fetch_one(&app.pool)
            .await
            .unwrap();
        assert_eq!(kept, 0);
    }

    #[test]
    fn a_bucket_lends_its_count_then_refills_one_every_share_of_the_period() {
        use filedrawer_server::limits::RateLimits;
        use std::time::Instant;
        let rate = per_minute(3);
        let limits = RateLimits::new(wide());
        let start = Instant::now();
        for _ in 0..3 {
            assert_eq!(limits.take("k", rate, start), Ok(()));
        }
        assert_eq!(limits.take("k", rate, start).unwrap_err().as_secs(), 20);
        let later = start + Duration::from_secs(20);
        assert_eq!(limits.take("k", rate, later), Ok(()));
        assert!(limits.take("k", rate, later).is_err());
        // A long rest refills it to its count, no more.
        let rested = later + Duration::from_secs(3600);
        for _ in 0..3 {
            assert_eq!(limits.take("k", rate, rested), Ok(()));
        }
        assert!(limits.take("k", rate, rested).is_err());
    }
}
