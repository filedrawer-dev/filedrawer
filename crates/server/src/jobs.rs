//! Ending the hour a submission is held (design.md, Entry status: `pending`), on its
//! author's request or when the hour runs out, and running the screening queue.

use time::{Duration, OffsetDateTime};

use crate::attachments::{self, StoreError};
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
    let queued = queue::unscreened(&state.pool).await?;
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

/// Runs `tidy_files` every hour, for `filedrawer serve`. An error is logged and the
/// loop goes on.
pub async fn run_tidier(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
    loop {
        ticks.tick().await;
        if let Err(error) = tidy_files(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying attached files failed");
        }
    }
}

/// Runs `publish_due` every 30 seconds, for `filedrawer serve`. An error is logged and
/// the loop goes on.
pub async fn run_publisher(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        ticks.tick().await;
        if let Err(error) = publish_due(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "running the screening queue failed");
        }
    }
}
