//! `DELETE /e/<id>` and `DELETE /batches/<id>` (design.md, Accounts; Agent
//! interface): a token deletes a pending entry, or a whole pending batch, that it
//! submitted itself, so a human who says "pull it" during the hour's hold is heard.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::AppState;
use crate::auth::TokenAccount;
use crate::error::AppError;
use crate::respond::{Format, echoed};
use crate::withdraw::{self, Asker, Outcome, Refusal, Token};

pub async fn entry(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
) -> Result<Response, AppError> {
    let outcome = withdraw::delete_entry(&state, Asker::Token(token_of(&account)), &id).await?;
    reply(&account, format, &id, Deleting::Entry, outcome)
}

pub async fn batch(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
) -> Result<Response, AppError> {
    let outcome = withdraw::delete_batch(&state, token_of(&account), &id).await?;
    reply(&account, format, &id, Deleting::Batch, outcome)
}

/// The request's token, as the deletion rules read it.
fn token_of(account: &TokenAccount) -> Token<'_> {
    Token {
        account_id: &account.id,
        token_id: &account.token_id,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deleting {
    Entry,
    Batch,
}

/// The reply to a deletion, or to its retry once done: the ID, when it was deleted,
/// the account, and a batch's members.
fn reply(
    account: &TokenAccount,
    format: Format,
    id: &str,
    deleting: Deleting,
    outcome: Outcome,
) -> Result<Response, AppError> {
    let (at, ids) = match outcome {
        Outcome::Deleted { at, ids } | Outcome::AlreadyDeleted { at, ids } => (at, ids),
        Outcome::NotFound => {
            let what = match deleting {
                Deleting::Entry => "entry",
                Deleting::Batch => "batch",
            };
            return Err(AppError::NotFoundFor(format!(
                "no {what} {} that this token's account can see; {}",
                echoed(id),
                account.line().trim_end()
            )));
        }
        Outcome::Refused(refusal) => return Err(refused(&refusal)),
    };
    let mut text = format!(
        "id: {id}\nstatus: deleted\ndeleted: {at}\n{}",
        account.line()
    );
    let mut json = json!({
        "id": id,
        "status": "deleted",
        "deleted": at,
        "account": account.json(),
    });
    if deleting == Deleting::Batch {
        for member in &ids {
            text.push_str(&format!("entry: {member}\n"));
        }
        json["entries"] = json!(ids);
    }
    Ok(format.reply(StatusCode::OK, text, json))
}

/// The error a token's refused deletion answers: 403 for another token's entry, 409
/// for the rest.
fn refused(refusal: &Refusal) -> AppError {
    match refusal {
        Refusal::OtherToken { .. } => AppError::Forbidden(refusal.to_string()),
        _ => AppError::Conflict(refusal.to_string()),
    }
}
