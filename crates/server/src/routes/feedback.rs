//! `POST /feedback`: private feedback from agents (design.md, Feedback and usability
//! cycles).

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use crate::auth::TokenAccount;
use crate::error::AppError;
use crate::respond::Format;
use crate::{AppState, ids, time::now};

/// The most text one field may hold.
const MAX_FIELD_CHARS: usize = 10_000;

#[derive(Debug, Deserialize)]
struct Feedback {
    experiment: Option<String>,
    wanted: Option<String>,
    workaround: Option<String>,
}

pub async fn feedback(
    State(state): State<AppState>,
    account: TokenAccount,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let refuse = |reasons: Vec<String>| AppError::Refused {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        reasons,
    };
    let feedback: Feedback = serde_json::from_slice(&body).map_err(|_| {
        refuse(vec![
            "send JSON: {\"experiment\": ..., \"wanted\": ..., \"workaround\": ...}".into(),
        ])
    })?;
    let fields = [
        ("experiment", feedback.experiment),
        ("wanted", feedback.wanted),
        ("workaround", feedback.workaround),
    ];
    let mut reasons = Vec::new();
    for (name, value) in &fields {
        match value.as_deref().map(str::trim) {
            None | Some("") => reasons.push(format!(
                "`{name}` is missing: say the experiment, what you wanted to submit, and the workaround you used"
            )),
            Some(text) if text.chars().count() > MAX_FIELD_CHARS => {
                reasons.push(format!("`{name}` is over {MAX_FIELD_CHARS} characters"))
            }
            Some(_) => {}
        }
    }
    let [
        (_, Some(experiment)),
        (_, Some(wanted)),
        (_, Some(workaround)),
    ] = fields
    else {
        return Err(refuse(reasons));
    };
    if !reasons.is_empty() {
        return Err(refuse(reasons));
    }
    let id = ids::new_id();
    let created_at = now();
    sqlx::query!(
        "INSERT INTO feedback (id, account_id, experiment, wanted, workaround, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        id,
        account.id,
        experiment,
        wanted,
        workaround,
        created_at
    )
    .execute(&state.pool)
    .await?;
    Ok(format.reply(
        StatusCode::CREATED,
        format!("feedback received: {id}\n"),
        json!({"id": id}),
    ))
}
