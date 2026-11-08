//! Withdrawing an entry that was never public from the account page (design.md,
//! Entry lifecycle; Accounts): a confirm page, then its POST deletes the entry, or
//! its whole batch. Only the account's browser session withdraws.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;

use crate::AppState;
use crate::error::AppError;
use crate::pages::{Message, WithdrawPage, page};
use crate::routes::account::SessionAccount;
use crate::withdraw::{self, Asker, Outcome};

/// The confirm page: what withdrawing the entry deletes and makes public, or why it
/// can't be withdrawn.
pub async fn confirm_page(
    State(state): State<AppState>,
    session: SessionAccount,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    confirm(&state, &session.id, &id, StatusCode::OK).await
}

async fn confirm(
    state: &AppState,
    account_id: &str,
    id: &str,
    status: StatusCode,
) -> Result<Response, AppError> {
    let preview = withdraw::preview(&state.pool, account_id, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let refusal = preview.refusal.as_ref().map(ToString::to_string);
    let template = WithdrawPage {
        id,
        batch_id: preview.batch_id.as_deref(),
        members: &preview.members,
        refusal: refusal.as_deref(),
        made_public: &preview.made_public,
    };
    Ok(page(status, &template))
}

/// Withdraws the entry, or its whole batch, if the rules still allow it.
pub async fn withdraw(
    State(state): State<AppState>,
    session: SessionAccount,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let asker = Asker::Session {
        account_id: &session.id,
    };
    let text = match withdraw::delete_entry(&state, asker, &id).await? {
        Outcome::Deleted { ids, .. } => format!("Deleted, as never public: {}.", ids.join(", ")),
        Outcome::AlreadyDeleted { at, .. } => format!("It was withdrawn at {at}."),
        Outcome::NotFound => return Err(AppError::NotFound),
        // The page says why, as it stands now.
        Outcome::Refused(_) => {
            return confirm(&state, &session.id, &id, StatusCode::CONFLICT).await;
        }
    };
    Ok(page(
        StatusCode::OK,
        &Message {
            title: "Withdrawn",
            text: &text,
            link: Some(("/account", "Your account")),
        },
    ))
}
