//! Rate limits (design.md, Limits and abuse; issue #14): every request per client IP,
//! by the minute and by how many are in progress at once,
//! token-authenticated writes per account, and signups and account emails per IP
//! and per address, and stored feedback per account (#60). The per-minute limits
//! keep their counts in memory, where a restart forgets at most a minute's; the
//! hourly and daily ones keep theirs in SQLite, so a deploy doesn't reset them.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use hyper::body::{Frame, SizeHint};
use sqlx::SqlitePool;
use time::OffsetDateTime;

use crate::AppState;
use crate::error::AppError;
use crate::time::stamp;
use sha2::{Digest, Sha256};

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
    /// Requests one client IP may have in progress at once: a request counts from
    /// its headers until the last of its reply is written, so one a client sends
    /// or reads slowly counts for as long as it takes.
    pub in_flight_per_ip: u32,
    /// Token-authenticated writes per account, refused ones included (#14).
    pub writes_per_account: Rate,
    pub signups_per_ip: Vec<Window>,
    /// Accounts agents make with `POST /accounts`, per client IP, counted apart
    /// from signups, so its refusal can send a person to `/signup` (#130).
    pub agent_accounts_per_ip: Vec<Window>,
    /// Accounts agents make, across the whole site.
    pub agent_accounts_site: Vec<Window>,
    /// Requests to claim an account an agent made, each of which sends an email:
    /// per account, and across the whole site.
    pub claims_per_account: Vec<Window>,
    pub claims_site: Vec<Window>,
    /// Verification and password-reset emails, counted per request to send one,
    /// whether or not the address has an account.
    pub emails_per_address: Vec<Window>,
    pub emails_per_ip: Vec<Window>,
    /// Failed logins per email address, whether or not an account uses it.
    pub failed_logins_per_address: Vec<Window>,
    /// Feedback items stored per account (#60).
    pub feedback_per_account: Vec<Window>,
}

impl Limits {
    /// The numbers the site runs with (the owner's decision, 2026-11-06).
    pub fn site() -> Self {
        Limits {
            per_ip: per_minute(300),
            in_flight_per_ip: 32,
            writes_per_account: per_minute(60),
            signups_per_ip: vec![hourly(10), daily(40)],
            agent_accounts_per_ip: vec![hourly(10), daily(40)],
            agent_accounts_site: vec![daily(2000)],
            claims_per_account: vec![hourly(3)],
            claims_site: vec![daily(2000)],
            emails_per_address: vec![hourly(5), daily(20)],
            emails_per_ip: vec![hourly(20), daily(60)],
            failed_logins_per_address: vec![hourly(10)],
            feedback_per_account: vec![hourly(10), daily(50)],
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

/// How many buckets are kept before the full ones are dropped: the per-minute ones,
/// and each account's hourly link checks and share of GitHub's API calls
/// (`link_checks`). A full bucket is what a new one would be, so dropping it changes
/// nothing; if none is full, all go, which forgives at most a minute's requests, or
/// an hour's link checks and GitHub calls, and only happens to a server that 100,000
/// clients reach within a minute.
pub const MAX_BUCKETS: usize = 100_000;

/// The limits and the per-minute counts.
#[derive(Debug)]
pub struct RateLimits {
    pub limits: Limits,
    buckets: Mutex<HashMap<String, Bucket>>,
    /// How many requests each client IP has in progress; no entry for none.
    in_flight: Mutex<HashMap<String, u32>>,
}

/// One request in progress, counted against its client IP until this is dropped.
#[derive(Debug)]
pub struct InFlight {
    limits: Arc<RateLimits>,
    ip: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut in_flight = self
            .limits
            .in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = in_flight.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                in_flight.remove(&self.ip);
            }
        }
    }
}

impl RateLimits {
    pub fn new(limits: Limits) -> Self {
        RateLimits {
            limits,
            buckets: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    /// Counts a request from `ip` as in progress until what it returns is dropped,
    /// or returns `None` when the client has as many as it may. The map holds only
    /// clients with a request in progress, so the server's connections bound it.
    pub fn begin(self: &Arc<Self>, ip: &str) -> Option<InFlight> {
        let mut in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = in_flight.get(ip).copied().unwrap_or(0);
        if count >= self.limits.in_flight_per_ip {
            return None;
        }
        in_flight.insert(ip.to_string(), count + 1);
        Some(InFlight {
            limits: self.clone(),
            ip: ip.to_string(),
        })
    }

    /// Takes one request from `key`'s bucket at `rate`, or says how long until one
    /// is free.
    pub fn take(&self, key: &str, rate: Rate, now: Instant) -> Result<(), Duration> {
        self.take_many(key, rate, 1, now)
    }

    /// Takes `count` requests at once from `key`'s bucket at `rate`, all or none, or
    /// says how long until that many are free. More than the bucket holds counts as
    /// all it holds, so a full bucket never refuses.
    pub fn take_many(
        &self,
        key: &str,
        rate: Rate,
        count: u32,
        now: Instant,
    ) -> Result<(), Duration> {
        let mut buckets = self.buckets.lock().expect("no thread panics holding it");
        if buckets.len() >= MAX_BUCKETS {
            buckets.retain(|_, bucket| !bucket.is_full(now));
            if buckets.len() >= MAX_BUCKETS {
                buckets.clear();
            }
        }
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            rate,
            tokens: f64::from(rate.count),
            at: now,
        });
        bucket.take(f64::from(count.min(rate.count)), now)
    }

    /// Counts `count` link checks that a request by `account_id` is about to cause
    /// (design.md, Submission and screening, step 2), all or none.
    pub fn link_checks_by(&self, account_id: &str, rate: Rate, count: u32) -> Result<(), AppError> {
        self.take_many(&format!("links:{account_id}"), rate, count, Instant::now())
            .map_err(|wait| too_many("this account's link checks for the hour", wait))
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
    rate: Rate,
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn per_second(&self) -> f64 {
        f64::from(self.rate.count) / self.rate.per.as_secs_f64()
    }

    fn refilled(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        (self.tokens + elapsed * self.per_second()).min(f64::from(self.rate.count))
    }

    fn is_full(&self, now: Instant) -> bool {
        self.refilled(now) >= f64::from(self.rate.count)
    }

    /// Takes `count` tokens, all or none.
    fn take(&mut self, count: f64, now: Instant) -> Result<(), Duration> {
        self.tokens = self.refilled(now);
        self.at = now;
        if self.tokens >= count {
            self.tokens -= count;
            return Ok(());
        }
        Err(Duration::from_secs_f64(
            (count - self.tokens) / self.per_second(),
        ))
    }
}

/// The refusal for a limit `what` reached, to try again after `wait`.
fn too_many(what: &str, wait: Duration) -> AppError {
    AppError::TooManyRequests {
        what: what.to_string(),
        retry_after_secs: wait.as_secs_f64().ceil().max(1.0) as u64,
        instead: None,
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

/// How long a client with too many requests in progress is told to wait: most
/// requests end within a second.
const IN_FLIGHT_RETRY: Duration = Duration::from_secs(1);

/// A reply's body that keeps its request counted as in progress until the last of
/// it is written or the connection closes.
struct CountedBody {
    body: Body,
    _in_flight: InFlight,
}

impl hyper::body::Body for CountedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Pin::new(&mut self.body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

/// Middleware: every request counts against its client IP's rate, and, until the
/// last of its reply is written, against the requests the IP may have in progress.
/// A request refused for either takes nothing from the other.
pub async fn per_ip(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    request: Request,
    next: Next,
) -> Response {
    let limits = &state.rate_limits;
    let admitted = match limits.begin(&ip) {
        None => Err(too_many(
            "requests in progress from this IP address",
            IN_FLIGHT_RETRY,
        )),
        Some(in_flight) => limits
            .take(&format!("ip:{ip}"), limits.limits.per_ip, Instant::now())
            .map(|()| in_flight)
            .map_err(|wait| too_many("requests from this IP address", wait)),
    };
    match admitted {
        Ok(_in_flight) => next
            .run(request)
            .await
            .map(|body| Body::new(CountedBody { body, _in_flight })),
        Err(refusal) => {
            let asked = crate::respond::Asked::of(request.headers());
            asked.shape(refusal.into_response()).await
        }
    }
}

/// One hourly or daily count an event goes under: its key, its windows, and what
/// a refusal calls it. The key is stored as a SHA-256, so the counts keep no email
/// address or IP in plain.
#[derive(Debug, Clone)]
pub struct Counted<'a> {
    key: String,
    windows: &'a [Window],
    what: &'static str,
}

impl<'a> Counted<'a> {
    fn new(kind: &str, value: &str, windows: &'a [Window], what: &'static str) -> Self {
        let digest = Sha256::digest(format!("{kind}:{value}").as_bytes());
        let key = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Counted { key, windows, what }
    }

    pub fn signups_from(limits: &'a Limits, ip: &ClientIp) -> Self {
        Counted::new(
            "signup-ip",
            &ip.0,
            &limits.signups_per_ip,
            "signups from this IP address",
        )
    }

    pub fn agent_accounts_from(limits: &'a Limits, ip: &ClientIp) -> Self {
        Counted::new(
            "agent-account-ip",
            &ip.0,
            &limits.agent_accounts_per_ip,
            "accounts agents made from this IP address",
        )
    }

    /// Every account an agent makes, whatever its address, since a client with
    /// many addresses passes the per-IP count (#114).
    pub fn agent_accounts(limits: &'a Limits) -> Self {
        Counted::new(
            "agent-account-site",
            "all",
            &limits.agent_accounts_site,
            "accounts agents made on this site",
        )
    }

    pub fn claims_of(limits: &'a Limits, account_id: &str) -> Self {
        Counted::new(
            "claim-account",
            account_id,
            &limits.claims_per_account,
            "this account's requests to be claimed",
        )
    }

    /// Every request to claim an account, whatever the account or address, since
    /// each sends an email to an address the server knows nothing of.
    pub fn claims(limits: &'a Limits) -> Self {
        Counted::new(
            "claim-site",
            "all",
            &limits.claims_site,
            "requests to claim an account on this site",
        )
    }

    pub fn emails_to(limits: &'a Limits, address: &str) -> Self {
        Counted::new(
            "email-address",
            address,
            &limits.emails_per_address,
            "account emails to this email address",
        )
    }

    pub fn failed_logins_to(limits: &'a Limits, address: &str) -> Self {
        Counted::new(
            "login-address",
            address,
            &limits.failed_logins_per_address,
            "failed logins to this email address",
        )
    }

    /// Wrong passwords typed into a logged-in account's own form, per account, at
    /// the failed logins' numbers; counted apart from the address's failed logins,
    /// so neither locks the other.
    pub fn wrong_passwords_of(limits: &'a Limits, account_id: &str) -> Self {
        Counted::new(
            "password-account",
            account_id,
            &limits.failed_logins_per_address,
            "wrong passwords on this account's forms",
        )
    }

    pub fn emails_from(limits: &'a Limits, ip: &ClientIp) -> Self {
        Counted::new(
            "email-ip",
            &ip.0,
            &limits.emails_per_ip,
            "account emails asked for from this IP address",
        )
    }
}

/// Counts one event under each of `counts`. When any window is full, refuses it,
/// naming the fullest, and records nothing.
pub async fn count_event(
    pool: &SqlitePool,
    counts: &[Counted<'_>],
    now: OffsetDateTime,
) -> Result<(), AppError> {
    take_slot(pool, counts, now).await.map(|_| ())
}

/// The rows an event took in its counts, so that one which turns out not to count,
/// such as a login whose password was right, can give them back.
#[derive(Debug)]
pub struct Slot(Vec<i64>);

/// Counts one event under each of `counts`, as [`count_event`] does, and returns
/// its slot. An event whose outcome isn't known yet takes its slot first, so a
/// burst of them can't all pass a window with one place left.
pub async fn take_slot(
    pool: &SqlitePool,
    counts: &[Counted<'_>],
    now: OffsetDateTime,
) -> Result<Slot, AppError> {
    // IMMEDIATE takes the write lock first, so two requests can't both read a
    // window with one place left, and the second doesn't fail on upgrading.
    let mut tx = crate::db::write_tx(pool).await?;
    if let Some(refusal) = full_window(&mut tx, counts, now).await? {
        return Err(refusal);
    }
    let rows = insert(&mut tx, counts, now).await?;
    tx.commit().await?;
    Ok(Slot(rows))
}

/// Gives back a slot whose event turned out not to count.
pub async fn give_back(pool: &SqlitePool, slot: Slot) -> Result<(), sqlx::Error> {
    for row in slot.0 {
        sqlx::query!("DELETE FROM rate_events WHERE rowid = ?", row)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// The refusal when any window of `counts` is full, naming the one that frees up
/// last.
async fn full_window(
    conn: &mut sqlx::SqliteConnection,
    counts: &[Counted<'_>],
    now: OffsetDateTime,
) -> Result<Option<AppError>, sqlx::Error> {
    let mut refusal: Option<(&str, u64)> = None;
    for counted in counts {
        for window in counted.windows {
            let since = stamp(now - time::Duration::seconds(window.seconds));
            let row = sqlx::query!(
                r#"SELECT COUNT(*) AS "count!: i64", MIN(at) AS "oldest?: String"
                   FROM rate_events WHERE key = ? AND at > ?"#,
                counted.key,
                since
            )
            .fetch_one(&mut *conn)
            .await?;
            let Some(back) = wait_for_room(*window, row.count, row.oldest, now) else {
                continue;
            };
            if refusal.is_none_or(|(_, wait)| back > wait) {
                refusal = Some((counted.what, back));
            }
        }
    }
    Ok(refusal.map(|(what, wait)| too_many(what, Duration::from_secs(wait))))
}

/// The seconds until `window` has room again, when `count` events fill it, the
/// oldest of them at `oldest`; `None` while it has room.
fn wait_for_room(
    window: Window,
    count: i64,
    oldest: Option<String>,
    now: OffsetDateTime,
) -> Option<u64> {
    if count < window.count {
        return None;
    }
    // Room comes back when the oldest event in the window leaves it; at the
    // window's edge that is under a second, which still waits one.
    let oldest = oldest.and_then(|at| crate::time::parse(&at)).unwrap_or(now);
    let back = (oldest + time::Duration::seconds(window.seconds) - now).whole_seconds();
    Some(back.max(1).unsigned_abs())
}

/// The refusal when `account_id` has stored as much feedback as a window of
/// `limits` allows, naming the one that frees up last. The count is the account's
/// rows in `feedback`, so only stored items count, and the caller inserts in the
/// transaction `conn` belongs to, so two requests can't both take the last place.
pub async fn full_feedback_window(
    conn: &mut sqlx::SqliteConnection,
    limits: &Limits,
    account_id: &str,
    now: OffsetDateTime,
) -> Result<Option<AppError>, sqlx::Error> {
    let mut longest: Option<u64> = None;
    for window in &limits.feedback_per_account {
        let since = stamp(now - time::Duration::seconds(window.seconds));
        let row = sqlx::query!(
            r#"SELECT COUNT(*) AS "count!: i64", MIN(created_at) AS "oldest?: String"
               FROM feedback WHERE account_id = ? AND created_at > ?"#,
            account_id,
            since
        )
        .fetch_one(&mut *conn)
        .await?;
        longest = longest.max(wait_for_room(*window, row.count, row.oldest, now));
    }
    Ok(longest.map(|wait| too_many("this account's feedback", Duration::from_secs(wait))))
}

async fn insert(
    conn: &mut sqlx::SqliteConnection,
    counts: &[Counted<'_>],
    now: OffsetDateTime,
) -> Result<Vec<i64>, sqlx::Error> {
    let at = stamp(now);
    let mut rows = Vec::new();
    for counted in counts {
        let done = sqlx::query!(
            "INSERT INTO rate_events (key, at) VALUES (?, ?)",
            counted.key,
            at
        )
        .execute(&mut *conn)
        .await?;
        rows.push(done.last_insert_rowid());
    }
    Ok(rows)
}

/// Counts an account email to `address`, such as a verification, a password reset or
/// the link of a change of address, and against the IP
/// that asked for it when `from` names one: an invited account's emails count per
/// address only (design.md, Limits and abuse).
pub async fn count_email(
    state: &AppState,
    from: Option<&ClientIp>,
    address: &str,
) -> Result<(), AppError> {
    let limits = &state.rate_limits.limits;
    let mut counts = vec![Counted::emails_to(limits, address)];
    counts.extend(from.map(|ip| Counted::emails_from(limits, ip)));
    count_event(&state.pool, &counts, OffsetDateTime::now_utc()).await
}

/// Deletes counted events older than the longest window.
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let before = stamp(now - time::Duration::seconds(LONGEST_WINDOW_SECONDS));
    sqlx::query!("DELETE FROM rate_events WHERE at <= ?", before)
        .execute(pool)
        .await?;
    Ok(())
}
