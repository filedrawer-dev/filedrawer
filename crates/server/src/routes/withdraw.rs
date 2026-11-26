//! Withdrawing an entry from the account page (design.md, Entry lifecycle;
//! Accounts): a confirm page, then its POST deletes an entry that was never public,
//! with its whole batch, or withdraws a public version with a reason, leaving a
//! notice in its place. Only the account's browser session withdraws.

use axum::Form;
use axum::extract::rejection::FormRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;

use crate::AppState;
use crate::error::AppError;
use crate::pages::{Message, WithdrawPage, page};
use crate::routes::account::SessionAccount;
use crate::withdraw::{self, Withdrawal};

/// The confirm page: what withdrawing the entry deletes and makes public, or the
/// reason a public version's notice will give, or why it can't be withdrawn.
pub async fn confirm_page(
    State(state): State<AppState>,
    session: SessionAccount,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    confirm(&state, &session.id, &id, StatusCode::OK, None).await
}

async fn confirm(
    state: &AppState,
    account_id: &str,
    id: &str,
    status: StatusCode,
    error: Option<&str>,
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
        public: preview.public,
        back_to: preview.back_to.as_deref(),
        error,
        max_reason_chars: crate::notices::MAX_REASON_CHARS,
    };
    Ok(page(status, &template))
}

#[derive(Deserialize)]
pub struct WithdrawForm {
    /// Why a public version is withdrawn; an entry that was never public needs none.
    reason: Option<String>,
}

/// Withdraws the entry, or its whole batch, if the rules still allow it.
pub async fn withdraw(
    State(state): State<AppState>,
    session: SessionAccount,
    Path(id): Path<String>,
    form: Result<Form<WithdrawForm>, FormRejection>,
) -> Result<Response, AppError> {
    let reason = form.ok().and_then(|Form(form)| form.reason);
    let text = match withdraw::withdraw(&state, &session.id, &id, reason.as_deref()).await? {
        Withdrawal::Deleted { ids } => format!("Deleted, as never public: {}.", ids.join(", ")),
        Withdrawal::Withdrawn { at } => format!(
            "Withdrawn at {at}. Its page shows a notice in its place now, and your reason once screening has checked it."
        ),
        Withdrawal::Already { at } => format!("It was withdrawn at {at}."),
        Withdrawal::NotFound => return Err(AppError::NotFound),
        Withdrawal::BadReason(bad) => {
            let error = bad.to_string();
            return confirm(
                &state,
                &session.id,
                &id,
                StatusCode::UNPROCESSABLE_ENTITY,
                Some(&error),
            )
            .await;
        }
        // The page says why, as it stands now.
        Withdrawal::Refused => {
            return confirm(&state, &session.id, &id, StatusCode::CONFLICT, None).await;
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
