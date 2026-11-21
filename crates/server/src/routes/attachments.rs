//! `POST /attachments`: an attachment set, uploaded once and cited by entries as
//! `attachments: as-...` (design.md, Attached code).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::attachments::{self, Invalid, Limit, SetRequest, Stored};
use crate::auth::TokenAccount;
use crate::error::AppError;
use crate::mail_queue::{self, Queued};
use crate::removed_files::REMOVED_BY_OWNER;
use crate::respond::{DELIMITER, Format, one_line};
use crate::time::now_utc;
use crate::{AppState, accounts, db};

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
        // The parser's message can repeat a value from the body, so only its start.
        refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![format!(
                "send JSON: {{\"repos\": {{\"<name>\": \"<url or no remote>\"}}, \"files\": [{{\"origin\": \"...\", \"text\": \"...\"}}]}} ({})",
                attachments::shown(&error.to_string())
            )],
        )
    })?;
    let set = attachments::read(&set).map_err(|invalid| match invalid {
        Invalid::OverCap(reason) => refused(StatusCode::PAYLOAD_TOO_LARGE, vec![reason]),
        Invalid::Problems(problems) => refused(StatusCode::UNPROCESSABLE_ENTITY, problems),
    })?;
    let set = Arc::new(set);
    // In a task of its own, so a client that hangs up can't stop it between
    // writing the files' bytes and committing their records.
    let storing = {
        let (state, set) = (state.clone(), set.clone());
        tokio::spawn(async move { attachments::store(&state, &account.id, &set).await })
    };
    let stored = storing.await.map_err(std::io::Error::from)??;
    let files = &set.files;
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
        Stored::Banned(paths) => {
            let reasons = paths
                .iter()
                .map(|path| format!("file {path} was {REMOVED_BY_OWNER}, so it can't be attached"))
                .collect();
            return Err(refused(StatusCode::UNPROCESSABLE_ENTITY, reasons));
        }
        Stored::Full(limit) => {
            tell_owner(&state, limit).await?;
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
    for file in files {
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

/// Emails the admins, once a day for each limit, that `limit` refuses uploads.
async fn tell_owner(state: &AppState, limit: Limit) -> Result<(), AppError> {
    let (kind, what, why) = match limit {
        Limit::StoreCap => (
            "attachment storage full",
            "attachment storage is full",
            format!(
                "the files it stores have reached the cap of {} bytes",
                state.attachment_store_cap_bytes
            ),
        ),
        Limit::DiskFloor => (
            "attachment disk low",
            "the disk for attachments is nearly full",
            format!(
                "storing more would leave under {} bytes free on the disk that holds them",
                state.attachment_disk_floor_bytes
            ),
        ),
    };
    // The day's notice is recorded with its emails, so a failure leaves neither.
    let mut tx = db::write_tx(&state.pool).await?;
    if !attachments::first_notice_today(&mut tx, kind).await? {
        return Ok(());
    }
    tracing::error!(%why, "{what}; uploads are refused");
    let notice = Queued::Notice {
        subject: format!("File Drawer: {what}"),
        body: format!("{} refuses new attachments: {why}.\n", state.public_url),
    };
    for admin in accounts::admin_ids(&mut tx).await? {
        mail_queue::queue_in(&mut tx, &admin, &notice, now_utc()).await?;
    }
    tx.commit().await?;
    mail_queue::wake(state);
    Ok(())
}
