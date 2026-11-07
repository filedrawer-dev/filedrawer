//! Rate limits (design.md, Limits and abuse; issue #14): every request per client IP,
//! token-authenticated writes per account, and signups and account emails per IP
//! and per address. The per-minute limits keep their counts in memory, where a
//! restart forgets at most a minute's; the hourly and daily ones keep theirs in
//! SQLite, so a deploy doesn't reset them.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;
use time::OffsetDateTime;

use crate::AppState;
use crate::error::AppError;
use crate::time::stamp;

/// So many requests in so long, refilled evenly.
#[derive(Debug, Clone, Copy)]
pub struct Rate {
    pub count: u32,
    pub per: Duration,
}

/// At most `count` events in any `seconds`.
#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub seconds: i64,
    pub count: i64,
}

/// Every limit's numbers.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Every request, read or write, per client IP.
    pub per_ip: Rate,
    /// Token-authenticated writes per account, refused ones included (#14).
    pub writes_per_account: Rate,
    pub signups_per_ip: Vec<Window>,
    /// Verification and password-reset emails, counted per request to send one,
    /// whether or not the address has an account.
    pub emails_per_address: Vec<Window>,
    pub emails_per_ip: Vec<Window>,
}

impl Limits {
    /// The numbers the site runs with (the owner's decision, 2026-11-07).
    pub fn site() -> Self {
        Limits {
            per_ip: per_minute(300),
            writes_per_account: per_minute(60),
            signups_per_ip: vec![hourly(10), daily(40)],
            emails_per_address: vec![hourly(5), daily(20)],
            emails_per_ip: vec![hourly(20), daily(60)],
        }
    }
}

fn per_minute(count: u32) -> Rate {
    Rate {
        count,
        per: Duration::from_secs(60),
    }
}

fn hourly(count: i64) -> Window {
    Window {
        seconds: 3600,
        count,
    }
}

fn daily(count: i64) -> Window {
    Window {
        seconds: 86_400,
        count,
    }
}

/// The longest window, past which counted events are deleted.
const LONGEST_WINDOW_SECONDS: i64 = 86_400;

/// How many per-minute buckets are kept before the full ones are dropped. A full
/// bucket is what a new one would be, so dropping it changes nothing.
const MAX_BUCKETS: usize = 100_000;

/// The limits and the per-minute counts.
#[derive(Debug)]
pub struct RateLimits {
    pub limits: Limits,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimits {
    pub fn new(limits: Limits) -> Self {
        RateLimits {
            limits,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one request from `key`'s bucket at `rate`, or says how long until one
    /// is free.
    fn take(&self, key: &str, rate: Rate, now: Instant) -> Result<(), Duration> {
        let mut buckets = self.buckets.lock().expect("no thread panics holding it");
        if buckets.len() >= MAX_BUCKETS {
            buckets.retain(|_, bucket| bucket.refilled(rate, now) < f64::from(rate.count));
        }
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: f64::from(rate.count),
            at: now,
        });
        bucket.take(rate, now)
    }

    /// Counts a token-authenticated write by `account_id`.
    pub fn write_by(&self, account_id: &str) -> Result<(), AppError> {
        let rate = self.limits.writes_per_account;
        self.take(&format!("account:{account_id}"), rate, Instant::now())
            .map_err(|wait| too_many("this account's writes", wait))
    }
}

/// A token bucket: up to `rate.count` requests at once, refilled at `rate`.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn refilled(&self, rate: Rate, now: Instant) -> f64 {
        let per_second = f64::from(rate.count) / rate.per.as_secs_f64();
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        (self.tokens + elapsed * per_second).min(f64::from(rate.count))
    }

    fn take(&mut self, rate: Rate, now: Instant) -> Result<(), Duration> {
        self.tokens = self.refilled(rate, now);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return Ok(());
        }
        let per_second = f64::from(rate.count) / rate.per.as_secs_f64();
        Err(Duration::from_secs_f64((1.0 - self.tokens) / per_second))
    }
}

/// The refusal for a limit `what` reached, to try again after `wait`.
fn too_many(what: &str, wait: Duration) -> AppError {
    AppError::TooManyRequests {
        what: what.to_string(),
        retry_after_secs: wait.as_secs_f64().ceil().max(1.0) as u64,
    }
}

/// The client a request comes from, as the limits key it: the TCP peer, or, when
/// the peer is loopback (Caddy on the same host), the last X-Forwarded-For address,
/// the one Caddy saw. IPv6 addresses are grouped by /64, which one host often has
/// to itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIp(pub String);

impl ClientIp {
    pub fn of(peer: Option<SocketAddr>, headers: &HeaderMap) -> Self {
        let forwarded = || {
            headers
                .get("x-forwarded-for")?
                .to_str()
                .ok()?
                .rsplit(',')
                .next()?
                .trim()
                .parse::<IpAddr>()
                .ok()
        };
        let peer = peer.map(|peer| peer.ip().to_canonical());
        let ip = match peer {
            Some(peer) if !peer.is_loopback() => Some(peer),
            _ => forwarded().or(peer),
        };
        ClientIp(ip.map_or_else(|| "unknown".to_string(), key_of))
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(peer)| *peer);
        Ok(ClientIp::of(peer, &parts.headers))
    }
}

/// An address as the limits key it: IPv4 whole, IPv6 by its /64.
fn key_of(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
    }
}

/// Middleware: every request counts against its client IP's rate.
pub async fn per_ip(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    request: Request,
    next: Next,
) -> Response {
    let rate = state.rate_limits.limits.per_ip;
    match state
        .rate_limits
        .take(&format!("ip:{ip}"), rate, Instant::now())
    {
        Ok(()) => next.run(request).await,
        Err(wait) => {
            let json = crate::respond::wants_json(request.headers());
            let refusal = too_many("requests from this address", wait).into_response();
            crate::respond::as_json_if(json, refusal).await
        }
    }
}

/// Counts one event under each of `keys` (such as `signup-ip:203.0.113.5`), each
/// with its windows. When any window is full, refuses it and records nothing.
pub async fn count_event(
    pool: &SqlitePool,
    what: &str,
    keys: &[(String, &[Window])],
    now: OffsetDateTime,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    let mut wait = 0i64;
    for (key, windows) in keys {
        for window in *windows {
            let since = stamp(now - time::Duration::seconds(window.seconds));
            let row = sqlx::query!(
                r#"SELECT COUNT(*) AS "count!: i64", MIN(at) AS "oldest?: String"
                   FROM rate_events WHERE key = ? AND at > ?"#,
                key,
                since
            )
            .fetch_one(&mut *tx)
            .await?;
            if row.count >= window.count {
                // Room comes back when the oldest event in the window leaves it.
                let oldest = row
                    .oldest
                    .and_then(|at| crate::time::parse(&at))
                    .unwrap_or(now);
                let back = (oldest + time::Duration::seconds(window.seconds) - now).whole_seconds();
                wait = wait.max(back);
            }
        }
    }
    if wait > 0 {
        return Err(too_many(what, Duration::from_secs(wait.unsigned_abs())));
    }
    let at = stamp(now);
    for (key, _) in keys {
        sqlx::query!("INSERT INTO rate_events (key, at) VALUES (?, ?)", key, at)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The keys a verification or password-reset email to `address` counts under.
pub fn email_keys<'a>(
    limits: &'a Limits,
    ip: &ClientIp,
    address: &str,
) -> Vec<(String, &'a [Window])> {
    vec![
        (format!("email-ip:{}", ip.0), &limits.emails_per_ip[..]),
        (
            format!("email-address:{address}"),
            &limits.emails_per_address[..],
        ),
    ]
}

/// Deletes counted events older than the longest window.
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let before = stamp(now - time::Duration::seconds(LONGEST_WINDOW_SECONDS));
    sqlx::query!("DELETE FROM rate_events WHERE at <= ?", before)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: Rate = Rate {
        count: 3,
        per: Duration::from_secs(60),
    };

    #[test]
    fn a_bucket_lends_its_count_then_refills_one_every_share_of_the_period() {
        let limits = RateLimits::new(Limits::site());
        let start = Instant::now();
        for _ in 0..3 {
            assert_eq!(limits.take("k", RATE, start), Ok(()));
        }
        let wait = limits.take("k", RATE, start).unwrap_err();
        assert_eq!(wait.as_secs(), 20);
        let later = start + Duration::from_secs(20);
        assert_eq!(limits.take("k", RATE, later), Ok(()));
        assert!(limits.take("k", RATE, later).is_err());
        // A long rest refills it to its count, no more.
        let rested = later + Duration::from_secs(3600);
        for _ in 0..3 {
            assert_eq!(limits.take("k", RATE, rested), Ok(()));
        }
        assert!(limits.take("k", RATE, rested).is_err());
    }
}
