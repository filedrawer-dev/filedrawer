//! The owner's admin pages (design.md, Admin). Every route reads the browser session
//! and never a token, so no agent's token can reach them, and an account that isn't
//! the admin gets the same 404 as a page that doesn't exist.

use analyzer::{analyze, render_card};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::pages::{AdminEntryPage, AdminQueuePage, QueueRow, page};
use crate::review::{self, Decided, Decision};
use crate::routes::account::SessionAccount;
use crate::time::now_utc;
use crate::{AppState, accounts, entries};

/// The admin's browser session.
pub struct AdminSession;

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let session = SessionAccount::from_request_parts(parts, state).await?;
        match accounts::is_admin(&state.pool, &session.id).await {
            Ok(true) => Ok(AdminSession),
            Ok(false) => Err(AppError::NotFound.into_response()),
            Err(error) => Err(AppError::from(error).into_response()),
        }
    }
}

fn title_of(source: &str) -> String {
    analyze(source)
        .fields
        .title
        .unwrap_or_else(|| "(no title)".to_string())
}

/// The entries in review and held, and the accounts under the review guard.
pub async fn queue(_: AdminSession, State(state): State<AppState>) -> Result<Response, AppError> {
    let undecided = entries::undecided(&state.pool, None).await?;
    let rows: Vec<QueueRow> = undecided
        .iter()
        .map(|entry| QueueRow {
            title: title_of(&entry.source),
            entry,
        })
        .collect();
    let (review, held) = rows.iter().partition(|row| row.entry.status == "review");
    let guarded = accounts::guarded(&state.pool).await?;
    Ok(page(
        StatusCode::OK,
        &AdminQueuePage {
            sections: [
                ("In review", review),
                ("Held until a vendor fixes the hole", held),
            ],
            guarded: &guarded,
        },
    ))
}

/// One entry in review or held: its card, its text, and the owner's choices.
pub async fn entry(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let entry = entries::undecided(&state.pool, Some(&id))
        .await?
        .pop()
        .ok_or(AppError::NotFound)?;
    let analysis = analyze(&entry.source);
    let context =
        entries::card_context(&entry.id, &entry.account_id, entry.display_name.as_deref());
    let card = render_card(&analysis, &context);
    // Shown with the entry once public, so the owner reads them too.
    let runs = entries::run_declarations(&state.pool, entry.seq).await?;
    Ok(page(
        StatusCode::OK,
        &AdminEntryPage {
            title: &title_of(&entry.source),
            entry: &entry,
            card: &card,
            runs: &runs,
        },
    ))
}

async fn decide(state: &AppState, id: &str, decision: Decision) -> Result<Response, AppError> {
    match review::decide(state, id, decision, now_utc()).await? {
        Decided::Done => Ok(Redirect::to("/admin").into_response()),
        Decided::NoEntry => Err(AppError::NotFound),
        Decided::NotWaiting => Err(AppError::Conflict(format!(
            "entry {id} isn't waiting on that decision; reload the queue"
        ))),
    }
}

pub async fn publish(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::Publish).await
}

pub async fn hold(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::Hold).await
}

pub async fn reject(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::Reject).await
}

pub async fn clear_guard(
    _: AdminSession,
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> Result<Response, AppError> {
    if !accounts::clear_guard(&state.pool, &account_id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to("/admin").into_response())
}
