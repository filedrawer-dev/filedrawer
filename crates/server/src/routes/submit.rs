//! Submitting an entry (design.md, Submission and screening; Entry status; Embargo):
//! `POST /entries` holds an accepted submission as `pending` for an hour, in which
//! `PUT /e/<id>` replaces it and `POST /e/<id>/publish` ends the hold early.

use analyzer::{
    Analysis, MAX_ENTRY_BYTES, Refusal, WarningKind, analyze, render_card, stored_text,
};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::auth::TokenAccount;
use crate::embargo::{parse_date, release, token_may_change};
use crate::entries::{self, OwnEntry};
use crate::error::AppError;
use crate::jobs::{end_hold, hold_until};
use crate::respond::{DELIMITER, Format, card_and_warnings};
use crate::{AppState, time};
use ::time::OffsetDateTime;
use ::time::format_description::well_known::Rfc3339;

/// A submission that passed every check: the text to store and what the analyzer
/// read from it.
struct Accepted {
    source: String,
    analysis: Analysis,
}

/// Runs every check a submission must pass, or for a resubmission of `held`, and
/// lists every reason it fails.
fn accept(body: &Bytes, held: Option<&OwnEntry>) -> Result<Accepted, AppError> {
    if body.len() > MAX_ENTRY_BYTES {
        return Err(AppError::Refused {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            reasons: vec![Refusal::TooLarge { bytes: body.len() }.to_string()],
        });
    }
    let text = std::str::from_utf8(body).map_err(|_| AppError::Refused {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        reasons: vec!["the entry is not UTF-8 text".into()],
    })?;
    let source = stored_text(text);
    let analysis = analyze(&source);
    let mut reasons: Vec<String> = analysis.refusals.iter().map(ToString::to_string).collect();
    // stored_text removes reserved fields; one that survives, as in a front matter
    // written as one `{...}` line, is refused rather than stored.
    reasons.extend(
        analysis
            .warnings
            .iter()
            .filter(|warning| warning.kind == WarningKind::StrippedField)
            .map(|warning| {
                format!(
                    "{}, but the server couldn't remove it from this front matter; delete it",
                    warning.message
                )
            }),
    );
    let now = time::now_utc();
    // A resubmission keeps its first submission date, which the embargo cap counts from.
    let first_submitted = held
        .and_then(|entry| OffsetDateTime::parse(&entry.first_submitted_at, &Rfc3339).ok())
        .unwrap_or(now);
    let embargo = analysis.fields.embargo_until.as_deref();
    if let Err(why) = release(embargo, now, first_submitted) {
        reasons.push(why.to_string());
    }
    if let Some(held) = held
        && let Err(why) = token_may_change(held.embargo_until.as_deref(), embargo, now)
    {
        reasons.push(why.to_string());
    }
    if !reasons.is_empty() {
        return Err(AppError::Refused {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            reasons,
        });
    }
    Ok(Accepted { source, analysis })
}

/// The reply to a submission or resubmission: the server's fields, then below the
/// delimiter the headline row as read, the card and the warnings.
fn held_reply(
    state: &AppState,
    format: Format,
    status: StatusCode,
    id: &str,
    account: &TokenAccount,
    analysis: &Analysis,
    publish_at: &str,
) -> Response {
    let context = entries::card_context(id, &account.id, &account.display_name);
    let card = render_card(analysis, &context);
    let warnings: Vec<String> = analysis.warnings.iter().map(ToString::to_string).collect();
    let embargo = parse_date(analysis.fields.embargo_until.as_deref()).map(|date| date.to_string());
    let embargo_line = embargo
        .as_deref()
        .map_or(String::new(), |date| format!("embargo_until: {date}\n"));
    let text = format!(
        "id: {id}\nstatus: pending\npublish_at: {publish_at}\n{embargo_line}url: {}/e/{id}\n\
         Fix it before then by sending the whole entry again with PUT to the url, or publish it now with a POST to the url plus /publish.\n\
         {DELIMITER}\nprimary row: {}\n{}",
        state.public_url,
        analysis.primary_row_summary(),
        card_and_warnings(&card, &warnings)
    );
    let json = json!({
        "id": id,
        "status": "pending",
        "publish_at": publish_at,
        "embargo_until": embargo,
        "url": format!("{}/e/{id}", state.public_url),
        "primary_row": analysis.primary_row_summary(),
        "card": card,
        "warnings": warnings,
    });
    format.reply(status, text, json)
}

pub async fn submit(
    State(state): State<AppState>,
    account: TokenAccount,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let accepted = accept(&body, None)?;
    let publish_at = hold_until(time::now_utc());
    let (_, id) = entries::insert(
        &state.pool,
        &account.id,
        &accepted.source,
        &accepted.analysis,
        &publish_at,
    )
    .await?;
    Ok(held_reply(
        &state,
        format,
        StatusCode::CREATED,
        &id,
        &account,
        &accepted.analysis,
        &publish_at,
    ))
}

/// The author's own entry, which must still be pending. Another account's entry is
/// not found, as if it didn't exist.
async fn own_pending(
    state: &AppState,
    id: &str,
    account: &TokenAccount,
) -> Result<OwnEntry, AppError> {
    let entry = entries::own_entry(&state.pool, id, &account.id)
        .await?
        .ok_or(AppError::NotFound)?;
    if entry.status != "pending" {
        return Err(AppError::Conflict(format!(
            "entry {id} is {}; only a pending entry can be replaced or published",
            entry.status
        )));
    }
    Ok(entry)
}

pub async fn resubmit(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let entry = own_pending(&state, &id, &account).await?;
    let accepted = accept(&body, Some(&entry))?;
    let publish_at = hold_until(time::now_utc());
    let replaced = entries::replace(
        &state.pool,
        entry.seq,
        &accepted.source,
        &accepted.analysis,
        &publish_at,
    )
    .await?;
    if !replaced {
        return Err(AppError::Conflict(format!(
            "entry {id}'s hold ended while it was being replaced"
        )));
    }
    Ok(held_reply(
        &state,
        format,
        StatusCode::OK,
        &id,
        &account,
        &accepted.analysis,
        &publish_at,
    ))
}

pub async fn publish(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
) -> Result<Response, AppError> {
    let entry = own_pending(&state, &id, &account).await?;
    let Some(placed) = end_hold(&state, entry.seq, time::now_utc()).await? else {
        return Err(AppError::Conflict(format!(
            "entry {id}'s hold has already ended"
        )));
    };
    let status = placed.status();
    let release = match placed {
        entries::Placed::Embargoed => entry.embargo_until.clone(),
        entries::Placed::Public => None,
    };
    let release_line = release
        .as_deref()
        .map_or(String::new(), |date| format!("release: {date}\n"));
    let text = format!(
        "id: {id}\nstatus: {status}\n{release_line}url: {}/e/{id}\n",
        state.public_url
    );
    let json = json!({
        "id": id,
        "status": status,
        "release": release,
        "url": format!("{}/e/{id}", state.public_url),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}
