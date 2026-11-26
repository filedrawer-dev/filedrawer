//! Ending the hour a submission is held (design.md, Entry status: `pending`), on its
//! author's request or when the hour runs out, and running the screening queue.

use time::{Duration, OffsetDateTime};

use crate::attachments::{self, StoreError};
use crate::{AppState, deletion, entries, queue, time::stamp};

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

/// One run of the worker: ends the hold on every pending entry whose hour has run
/// out at `now`, and screens every entry in `screening` without a verdict within the
/// caps, in the fair queue's order.
/// Returns how many entries it screened or placed.
pub async fn publish_due(state: &AppState, now: OffsetDateTime) -> Result<usize, sqlx::Error> {
    let mut done = 0;
    let now_text = stamp(now);
    for seq in entries::due(&state.pool, &now_text).await? {
        // Rechecks the hold: a PUT since `due` ran restarts it, and wins.
        entries::start_screening(&state.pool, seq, Some(&now_text)).await?;
    }
    queue::settle_screened(state).await?;
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

/// Deletes the entries rejected more than [`deletion::REJECTED_KEPT_DAYS`] before
/// `now` (`deletion::expire_rejected`).
pub async fn expire_rejected(state: &AppState, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    deletion::expire_rejected(state, now).await
}

/// Refreshes the linked GitHub accounts' logins every hour, for `filedrawer-server
/// serve` (`github_accounts::refresh_logins`). An error is logged and the loop goes
/// on.
pub async fn run_github_refresh(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
    loop {
        ticks.tick().await;
        match crate::github_accounts::refresh_logins(&state, OffsetDateTime::now_utc()).await {
            Ok(done) if done.stopped => {
                tracing::info!(asked = done.asked, "the GitHub login refresh stopped early");
            }
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "refreshing GitHub logins failed"),
        }
    }
}

/// Runs `tidy_files`, deletes the rejected entries kept long enough, the rate
/// limits' expired counts, the search log's old rows and GitHub sign-ins that
/// expired, closes the read counter's finished days, drops and deletes the mail
/// queue's old rows, and deletes the changes of address that expired, every hour,
/// for `filedrawer-server serve`. An error is logged and the loop goes on.
pub async fn run_tidier(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
    loop {
        ticks.tick().await;
        if let Err(error) = tidy_files(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying attached files failed");
        }
        if let Err(error) = expire_rejected(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "deleting expired rejected entries failed");
        }
        if let Err(error) = crate::limits::tidy(&state.pool, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying rate-limit counts failed");
        }
        if let Err(error) = crate::search_log::tidy(&state.pool, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying the search log failed");
        }
        if let Err(error) =
            crate::github_accounts::tidy(&state.pool, OffsetDateTime::now_utc()).await
        {
            tracing::error!(%error, "tidying GitHub sign-ins failed");
        }
        if let Err(error) = crate::reads::tidy(&state.pool, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying the read counts failed");
        }
        if let Err(error) = crate::mail_queue::tidy(&state.pool, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying the mail queue failed");
        }
        if let Err(error) = crate::claims::tidy(&state.pool, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "tidying the pending claims failed");
        }
        if let Err(error) = crate::email_change::tidy(&state.pool, OffsetDateTime::now_utc()).await
        {
            tracing::error!(%error, "tidying the pending changes of address failed");
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
