//! Ending the hour a submission is held (design.md, Entry status: `pending`), on its
//! author's request or when the hour runs out.

use analyzer::analyze;
use time::{Duration, OffsetDateTime};

use crate::embargo::embargoed_until;
use crate::entries::{self, Placed};
use crate::screening::Verdict;
use crate::series;
use crate::{AppState, time::stamp};

/// How long an accepted submission is held so its author can fix it. Each
/// resubmission starts it again.
pub const HOLD: Duration = Duration::minutes(60);

/// When a hold that starts at `now` ends, as the database stores it.
pub fn hold_until(now: OffsetDateTime) -> String {
    stamp(now + HOLD)
}

/// Ends the hold on one entry now, at its author's request: screens it, then makes
/// it public, or embargoed while its embargo date is ahead. `None` if the entry was
/// no longer pending.
pub async fn end_hold(
    state: &AppState,
    seq: i64,
    now: OffsetDateTime,
) -> Result<Option<Placed>, sqlx::Error> {
    let Some(source) = entries::start_screening(&state.pool, seq, None).await? else {
        return Ok(None);
    };
    screen_and_place(state, seq, &source, now).await
}

/// Screens an entry in `screening` and places it. `None` if something else placed it
/// first.
async fn screen_and_place(
    state: &AppState,
    seq: i64,
    source: &str,
    now: OffsetDateTime,
) -> Result<Option<Placed>, sqlx::Error> {
    let analysis = analyze(source);
    match state.screener.screen(source).await {
        Verdict::Pass => {
            if embargoed_until(analysis.fields.embargo_until.as_deref(), now).is_some() {
                entries::embargo(&state.pool, seq).await
            } else if series::waits(&state.pool, seq).await? {
                entries::wait_on_series(&state.pool, seq).await
            } else {
                entries::publish(&state.pool, seq, &analysis).await
            }
        }
    }
}

/// Publishes every entry in `waiting_on_series` that no longer waits, as when its
/// series' creator makes the series public, and returns how many.
pub async fn place_waiting(state: &AppState) -> Result<usize, sqlx::Error> {
    let mut placed = 0;
    for (seq, source) in entries::waiting(&state.pool).await? {
        if series::waits(&state.pool, seq).await? {
            continue;
        }
        if entries::publish(&state.pool, seq, &analyze(&source))
            .await?
            .is_some()
        {
            placed += 1;
        }
    }
    Ok(placed)
}

/// Ends the hold on every pending entry whose hour has run out at `now`, places any
/// entry left in `screening` by a failure, and publishes waiting entries whose series
/// are now public, then returns how many it placed.
pub async fn publish_due(state: &AppState, now: OffsetDateTime) -> Result<usize, sqlx::Error> {
    let mut placed = place_waiting(state).await?;
    let now_text = stamp(now);
    for (seq, source) in entries::in_screening(&state.pool).await? {
        if screen_and_place(state, seq, &source, now).await?.is_some() {
            placed += 1;
        }
    }
    for seq in entries::due(&state.pool, &now_text).await? {
        // Rechecks the hold: a PUT since `due` ran restarts it, and wins.
        let Some(source) = entries::start_screening(&state.pool, seq, Some(&now_text)).await?
        else {
            continue;
        };
        if screen_and_place(state, seq, &source, now).await?.is_some() {
            placed += 1;
        }
    }
    Ok(placed)
}

/// Runs `publish_due` every 30 seconds, for `filedrawer serve`. An error is logged and
/// the loop goes on.
pub async fn run_publisher(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        ticks.tick().await;
        if let Err(error) = publish_due(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "publishing held entries failed");
        }
    }
}
