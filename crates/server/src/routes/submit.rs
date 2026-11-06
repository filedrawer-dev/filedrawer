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

/// What an earlier submission fixes for a resubmission or a new version: the first
/// submission date the embargo cap counts from, and the embargo in force, which a
/// token may not drop or bring within 7 days.
struct Prior<'a> {
    first_submitted_at: &'a str,
    embargo_in_force: Option<&'a str>,
}

impl<'a> Prior<'a> {
    /// For `entry`. Its embargo is in force while it is pending or embargoed; a public
    /// entry's old date no longer holds anything back.
    fn of(entry: &'a OwnEntry) -> Prior<'a> {
        let in_force = matches!(entry.status.as_str(), "pending" | "embargoed");
        Prior {
            first_submitted_at: &entry.first_submitted_at,
            embargo_in_force: entry.embargo_until.as_deref().filter(|_| in_force),
        }
    }
}

/// Runs every check a submission must pass, given what an earlier submission fixes,
/// and lists every reason it fails.
fn accept(body: &Bytes, prior: Option<Prior<'_>>) -> Result<Accepted, AppError> {
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
    let first_submitted = prior
        .as_ref()
        .and_then(|prior| OffsetDateTime::parse(prior.first_submitted_at, &Rfc3339).ok())
        .unwrap_or(now);
    let embargo = analysis.fields.embargo_until.as_deref();
    if let Err(why) = release(embargo, now, first_submitted) {
        reasons.push(why.to_string());
    }
    if let Some(prior) = &prior
        && let Err(why) = token_may_change(prior.embargo_in_force, embargo, now)
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

/// A held entry, as a reply describes it.
struct Held<'a> {
    id: &'a str,
    account: &'a TokenAccount,
    analysis: &'a Analysis,
    publish_at: &'a str,
    /// The entry this one is a new version of.
    previous: Option<&'a str>,
}

/// The reply to a submission or resubmission: the server's fields, then below the
/// delimiter the headline row as read, the card and the warnings.
fn held_reply(state: &AppState, format: Format, status: StatusCode, held: &Held<'_>) -> Response {
    let Held {
        id,
        account,
        analysis,
        publish_at,
        previous,
    } = *held;
    let context = entries::card_context(id, &account.id, &account.display_name);
    let card = render_card(analysis, &context);
    let warnings: Vec<String> = analysis.warnings.iter().map(ToString::to_string).collect();
    let embargo = parse_date(analysis.fields.embargo_until.as_deref()).map(|date| date.to_string());
    let previous_line = previous.map_or(String::new(), |id| format!("previous: {id}\n"));
    let embargo_line = embargo
        .as_deref()
        .map_or(String::new(), |date| format!("embargo_until: {date}\n"));
    let text = format!(
        "id: {id}\nstatus: pending\npublish_at: {publish_at}\n{embargo_line}{previous_line}url: {}/e/{id}\n\
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
        "previous": previous,
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
        None,
    )
    .await?;
    let held = Held {
        id: &id,
        account: &account,
        analysis: &accepted.analysis,
        publish_at: &publish_at,
        previous: None,
    };
    Ok(held_reply(&state, format, StatusCode::CREATED, &held))
}

/// `POST /e/<id>/versions`: a correction of a public or embargoed entry, as a new
/// entry held like any submission (design.md, Entry lifecycle). Only on the latest
/// version in the chain, and only by its author.
pub async fn new_version(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let entry = entries::own_entry(&state.pool, &id, &account.id)
        .await?
        .ok_or(AppError::NotFound)?;
    match entry.status.as_str() {
        "public" | "embargoed" => {}
        "pending" => {
            return Err(AppError::Conflict(format!(
                "entry {id} is still pending: change it with PUT to its url instead"
            )));
        }
        other => {
            return Err(AppError::Conflict(format!(
                "entry {id} is {other}; a new version needs a public or embargoed entry"
            )));
        }
    }
    if let Some(conflict) = newer_version(&state, &id, entry.seq).await? {
        return Err(conflict);
    }
    let accepted = accept(&body, Some(Prior::of(&entry)))?;
    let publish_at = hold_until(time::now_utc());
    let previous = entries::Previous {
        seq: entry.seq,
        first_submitted_at: &entry.first_submitted_at,
    };
    let inserted = entries::insert(
        &state.pool,
        &account.id,
        &accepted.source,
        &accepted.analysis,
        &publish_at,
        Some(&previous),
    )
    .await;
    let (_, new_id) = match inserted {
        Ok(inserted) => inserted,
        // Another version won the race between the check above and this insert; the
        // index allows one live successor.
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(newer_version(&state, &id, entry.seq)
                .await?
                .unwrap_or_else(|| {
                    AppError::Conflict(format!(
                        "entry {id} took another version at the same moment; try again"
                    ))
                }));
        }
        Err(error) => return Err(error.into()),
    };
    let held = Held {
        id: &new_id,
        account: &account,
        analysis: &accepted.analysis,
        publish_at: &publish_at,
        previous: Some(&id),
    };
    Ok(held_reply(&state, format, StatusCode::CREATED, &held))
}

/// The conflict to answer when entry `seq` already has a live newer version, naming it.
async fn newer_version(state: &AppState, id: &str, seq: i64) -> Result<Option<AppError>, AppError> {
    let Some((latest, status)) = entries::successor(&state.pool, seq).await? else {
        return Ok(None);
    };
    let advice = if status == "pending" {
        format!("change the pending version {latest} with PUT instead")
    } else {
        format!("post the new version against {latest}, the latest")
    };
    Ok(Some(AppError::Conflict(format!(
        "entry {id} already has a newer version: {advice}"
    ))))
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
    let accepted = accept(&body, Some(Prior::of(&entry)))?;
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
    let held = Held {
        id: &id,
        account: &account,
        analysis: &accepted.analysis,
        publish_at: &publish_at,
        previous: entry.previous_id.as_deref(),
    };
    Ok(held_reply(&state, format, StatusCode::OK, &held))
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
