//! Ending the hour a submission is held (design.md, Entry status: `pending`), on its
//! author's request or when the hour runs out, and running the screening queue.

use time::{Duration, OffsetDateTime};

use crate::attachments::{self, StoreError};
use crate::entries::{Due, Released};
use crate::{AppState, entries, queue, series, time::stamp};

/// How long an accepted submission is held so its author can fix it. Each
/// resubmission starts it again.
pub const HOLD: Duration = Duration::minutes(60);

/// When a hold that starts at `now` ends, as the database stores it.
pub fn hold_until(now: OffsetDateTime) -> String {
    stamp(now + HOLD)
}

/// Ends the hold on one entry now, at its author's request, and screens it at once
/// if the day's caps allow. Returns its status afterwards: `screening` while it
/// waits, or where screening placed it. `None` if it was no longer pending.
pub async fn end_hold(
    state: &AppState,
    seq: i64,
    now: OffsetDateTime,
) -> Result<Option<String>, sqlx::Error> {
    if entries::start_screening(&state.pool, seq, None)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    queue::screen_now(state, seq, now).await
}

/// Publishes every entry in `waiting_on_series` that no longer waits, as when its
/// series' creator makes the series public, and returns how many.
pub async fn place_waiting(state: &AppState) -> Result<usize, sqlx::Error> {
    let mut placed = 0;
    for (seq, source) in entries::in_status(&state.pool, "waiting_on_series").await? {
        if series::waits(&state.pool, seq).await? {
            continue;
        }
        let analysis = analyzer::analyze(&source);
        if entries::publish(&state.pool, seq, &analysis)
            .await?
            .is_some()
        {
            placed += 1;
        }
    }
    Ok(placed)
}

/// One run of the worker: ends the hold on every pending entry whose hour has run
/// out at `now`, screens every entry in `screening` without a verdict within the
/// caps, in the fair queue's order, and publishes waiting entries whose series are
/// now public.
/// Returns how many entries it screened or placed.
pub async fn publish_due(state: &AppState, now: OffsetDateTime) -> Result<usize, sqlx::Error> {
    let mut done = place_waiting(state).await?;
    let now_text = stamp(now);
    for seq in entries::due(&state.pool, &now_text).await? {
        // Rechecks the hold: a PUT since `due` ran restarts it, and wins.
        entries::start_screening(&state.pool, seq, Some(&now_text)).await?;
    }
    queue::settle_screened(state, now).await?;
    let queued = queue::unscreened(&state.pool, now).await?;
    for seq in &queued {
        queue::screen_entry(state, *seq, now).await?;
    }
    // Counted after the loop: a batch member stays in screening until its last
    // member is screened.
    for seq in &queued {
        if entries::status(&state.pool, *seq).await?.as_deref() != Some("screening") {
            done += 1;
        }
    }
    Ok(done)
}

/// Releases every entry due at `now` (design.md, Embargo; Series): an `embargoed`
/// entry once its chain's date has come, and one `waiting_on_series` at the
/// 12-month cap. Returns how many it moved.
pub async fn release_due(state: &AppState, now: OffsetDateTime) -> Result<usize, sqlx::Error> {
    let mut releases = Vec::new();
    for held in entries::held_back(&state.pool).await? {
        if held.date_unreadable() {
            tracing::error!(
                seq = held.seq,
                "an embargoed entry's stored date isn't a date; it is held until the date is fixed or the 12-month cap"
            );
        }
        let to = match held.due(now) {
            None => continue,
            Some(Due::Cap) => Released::Public,
            Some(Due::Embargo) if series::waits(&state.pool, held.seq).await? => {
                Released::WaitingOnSeries
            }
            Some(Due::Embargo) => Released::Public,
        };
        releases.push((held.seq, to));
    }
    entries::release(&state.pool, &releases, now).await
}

/// Runs `release_due` every minute, for `filedrawer-server serve`, so an embargo ends
/// within a minute of 23:59 at UTC−12 on its date. An error is logged and the loop
/// goes on.
pub async fn run_releaser(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        ticks.tick().await;
        if let Err(error) = release_due(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "releasing embargoed entries failed");
        }
    }
}

/// How long a set may wait for an entry to cite it (design.md, Attached code).
pub const UNCITED_SET_DAYS: i64 = 7;

/// Tidies attached files at `now`: deletes the sets no entry cites that were
/// uploaded more than [`UNCITED_SET_DAYS`] ago, with the files no other set holds,
/// and removes bytes on disk that no record holds.
pub async fn tidy_files(state: &AppState, now: OffsetDateTime) -> Result<(), StoreError> {
    let before = stamp(now - Duration::days(UNCITED_SET_DAYS));
    let writer = state.files.write().await;
    let expired = attachments::expire_uncited_sets(&state.pool, &writer, &before).await;
    // Bytes no record holds go even when expiring sets failed.
    let swept = attachments::sweep(&state.pool, &writer).await;
    expired.map_err(StoreError::from).and(swept)
}

/// Runs `tidy_files`, and deletes the rate limits' expired counts and the search
/// log's old rows, every hour, for `filedrawer-server serve`. An error is logged and the
/// loop goes on.
pub async fn run_tidier(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
    loop {
        ticks.tick().await;
        if let Err(error) = tidy_files(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying attached files failed");
        }
        if let Err(error) = crate::limits::tidy(&state.pool, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying rate-limit counts failed");
        }
        if let Err(error) = crate::search_log::tidy(&state.pool, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying the search log failed");
        }
    }
}

/// Rechecks the links whose next check has come, every minute, for
/// `filedrawer-server serve`, so a link due an hour after its check is checked then.
/// An error is logged and the loop goes on.
pub async fn run_link_rechecker(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        ticks.tick().await;
        if let Err(error) = crate::link_checks::recheck_due(&state, OffsetDateTime::now_utc()).await
        {
            tracing::error!(%error, "rechecking links failed");
            state
                .link_checker
                .events()
                .recheck_failures
                .record(error.to_string());
        }
    }
}

/// Runs `publish_due` every `tick` (`Config::publisher_tick`: 30 seconds, unless dev sets another),
/// for `filedrawer-server serve`. An error is logged and the loop goes on.
pub async fn run_publisher(state: AppState, tick: std::time::Duration) {
    let mut ticks = tokio::time::interval(tick);
    loop {
        ticks.tick().await;
        if let Err(error) = publish_due(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "running the screening queue failed");
        }
    }
}

/// How often the daily email's job looks whether today's run is due.
const DAILY_EMAIL_TICK: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How long the job waits after a failed run before trying again.
const DAILY_EMAIL_RETRY: Duration = Duration::hours(1);

/// Runs `send_daily_email` every 5 minutes, for `filedrawer-server serve`, so today's run
/// goes within 5 minutes of midnight UTC, or of the server starting. A failed run is
/// logged and tried again an hour later.
pub async fn run_daily_email(state: AppState) {
    let mut ticks = tokio::time::interval(DAILY_EMAIL_TICK);
    let mut failed_at: Option<OffsetDateTime> = None;
    loop {
        ticks.tick().await;
        let now = OffsetDateTime::now_utc();
        if failed_at.is_some_and(|at| now < at + DAILY_EMAIL_RETRY && now >= at) {
            continue;
        }
        match crate::daily_email::send_daily_email(&state, now).await {
            Ok(_) => failed_at = None,
            Err(error) => {
                tracing::error!(%error, "the daily email failed; it is tried again in an hour");
                failed_at = Some(now);
            }
        }
    }
}
