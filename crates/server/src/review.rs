//! The owner's review queue (design.md, Admin; Entry status): the owner publishes,
//! holds or rejects an entry in `review`, and publishes or rejects a `held` one.

use time::OffsetDateTime;

use crate::{AppState, attachments, entries, queue};

/// The one reason the author reads for a rejection in review. The author already saw
/// `review`, which `/skill.md` says means a possible info hazard, so a category would
/// say no more, or contradict it (the owner's decision, 2026-11-06).
const REJECTED_IN_REVIEW: &str = "rejected in review";

#[derive(Debug, Clone, Copy)]
pub enum Decision {
    Publish,
    Hold,
    Reject,
    /// A frozen entry goes back to screening.
    Release,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decided {
    Done,
    NoEntry,
    /// The entry isn't waiting on this decision: publish needs `review` or `held`,
    /// reject those or `frozen`, hold needs `review`, and release `frozen`.
    NotWaiting,
}

/// Applies the owner's `decision` to entry `id`. Publish and reject go through the
/// screening queue's placing, so a batch is still decided as one.
pub async fn decide(
    state: &AppState,
    id: &str,
    decision: Decision,
    now: OffsetDateTime,
) -> Result<Decided, sqlx::Error> {
    let Some(seq) = entries::seq_of(&state.pool, id).await? else {
        return Ok(Decided::NoEntry);
    };
    let changed = match decision {
        Decision::Hold => entries::hold(&state.pool, seq).await?,
        Decision::Publish => {
            let passed = entries::owner_pass(&state.pool, seq).await?;
            if passed {
                attachments::pass_unjudged_files(&state.pool, seq).await?;
            }
            passed
        }
        Decision::Reject => {
            let reasons = serde_json::json!([REJECTED_IN_REVIEW]).to_string();
            entries::owner_reject(&state.pool, seq, &reasons).await?
                || entries::reject_frozen(&state.pool, seq, &reasons).await?
        }
        Decision::Release => entries::release_frozen(&state.pool, seq).await?,
    };
    if !changed {
        return Ok(Decided::NotWaiting);
    }
    // The decision is stored before it is placed. Should placing fail, the entry sits
    // in `screening` with a verdict, which the worker's `settle_screened` places on its
    // next run. A held or released entry has nothing to place yet.
    if !matches!(decision, Decision::Hold | Decision::Release) {
        queue::settle_decided(state, seq, now).await?;
    }
    Ok(Decided::Done)
}
