//! The owner's review queue (design.md, Admin; Entry status): the owner publishes,
//! holds or rejects an entry in `review`, and publishes or rejects a `held` one.

use time::OffsetDateTime;

use crate::{AppState, entries, queue};

/// The one reason the author reads for a rejection in review. The author already saw
/// `review`, which `/skill.md` says means a possible info hazard, so a category would
/// say no more, or contradict it (the owner's decision, 2026-11-06).
const REJECTED_IN_REVIEW: &str = "rejected in review";

#[derive(Debug, Clone, Copy)]
pub enum Decision {
    Publish,
    Hold,
    Reject,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decided {
    Done,
    NoEntry,
    /// The entry isn't waiting on this decision: publish and reject need `review` or
    /// `held`, and hold needs `review`.
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
        Decision::Publish => entries::owner_pass(&state.pool, seq).await?,
        Decision::Reject => {
            let reasons = serde_json::json!([REJECTED_IN_REVIEW]).to_string();
            entries::owner_reject(&state.pool, seq, &reasons).await?
        }
    };
    if !changed {
        return Ok(Decided::NotWaiting);
    }
    // The decision is stored before it is placed. Should placing fail, the entry sits
    // in `screening` with a verdict, which the worker's `settle_screened` places on its
    // next run.
    if !matches!(decision, Decision::Hold) {
        queue::settle_decided(state, seq, now).await?;
    }
    Ok(Decided::Done)
}
