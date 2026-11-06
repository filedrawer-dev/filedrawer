//! `POST /attachments`: an attachment set, uploaded once and cited by entries as
//! `attachments: as-...` (design.md, Attached code).

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::attachments::{self, Invalid, SetRequest, Stored};
use crate::auth::TokenAccount;
use crate::error::AppError;
use crate::mail::{Email, send_later};
use crate::respond::{DELIMITER, Format, one_line};
use crate::{AppState, accounts};

fn refused(status: StatusCode, reasons: Vec<String>) -> AppError {
    AppError::Refused { status, reasons }
}

pub async fn upload(
    State(state): State<AppState>,
    account: TokenAccount,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    if state.files_url.is_none() {
        return Err(refused(
            StatusCode::NOT_IMPLEMENTED,
            vec!["attachments aren't enabled on this server; link the code instead".into()],
        ));
    }
    let set: SetRequest = serde_json::from_slice(&body).map_err(|error| {
        refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![format!(
                "send JSON: {{\"repos\": {{\"<name>\": \"<url or no remote>\"}}, \"files\": [{{\"origin\": \"...\", \"text\": \"...\"}}]}} ({error})"
            )],
        )
    })?;
    let files = attachments::read(&set).map_err(|invalid| match invalid {
        Invalid::TooLarge(bytes) => refused(
            StatusCode::PAYLOAD_TOO_LARGE,
            vec![format!(
                "the set's files hold {bytes} bytes; the cap is {} bytes",
                attachments::MAX_SET_BYTES
            )],
        ),
        Invalid::Problems(problems) => refused(StatusCode::UNPROCESSABLE_ENTITY, problems),
    })?;
    let stored = attachments::store(
        &state.pool,
        &account.id,
        &files,
        state.attachment_store_cap_bytes,
    )
    .await?;
    let (id, bytes) = match stored {
        Stored::Saved { id, bytes } => (id, bytes),
        Stored::DailyCap => {
            return Err(refused(
                StatusCode::TOO_MANY_REQUESTS,
                vec![format!(
                    "this account has uploaded its {} bytes of attachments for today (UTC); send it tomorrow, or publish the code on GitHub or Zenodo",
                    attachments::ACCOUNT_DAILY_BYTES
                )],
            ));
        }
        Stored::StoreFull => {
            tell_owner_storage_is_full(&state).await?;
            return Err(refused(
                StatusCode::INSUFFICIENT_STORAGE,
                vec!["the site's storage for attachments is full; publish the code on GitHub or Zenodo instead".into()],
            ));
        }
    };
    // Each path is the sender's own text, so the file lines go below the delimiter.
    let mut text = format!(
        "id: {id}\nbytes: {bytes}\nCite it in each entry's front matter as `attachments: {id}`.\n{DELIMITER}\n"
    );
    for file in &files {
        text.push_str(&format!("file: {} {}\n", file.sha256, one_line(&file.path)));
    }
    let json = json!({
        "id": id,
        "bytes": bytes,
        "files": files
            .iter()
            .map(|file| json!({"path": file.path, "sha256": file.sha256}))
            .collect::<Vec<_>>(),
    });
    Ok(format.reply(StatusCode::CREATED, text, json))
}

/// Emails the admins, once a day, that attachment storage is full.
async fn tell_owner_storage_is_full(state: &AppState) -> Result<(), AppError> {
    if !attachments::first_notice_today(&state.pool, "attachment storage full").await? {
        return Ok(());
    }
    tracing::error!("attachment storage is full; uploads are refused");
    for to in accounts::admin_emails(&state.pool).await? {
        send_later(
            &state.mailer,
            Email {
                to,
                subject: "File Drawer: attachment storage is full".into(),
                body: format!(
                    "{} refuses new attachments: the files it stores have reached the cap of {} bytes.\n",
                    state.public_url, state.attachment_store_cap_bytes
                ),
            },
        );
    }
    Ok(())
}
