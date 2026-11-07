//! `POST /entries/batch` and `POST /batches/<id>/publish` (design.md, Agent interface):
//! several entries held, and published, together, with slugs that let them refer to
//! each other and runs declared once. Also the checks on slugs, runs and references
//! that single submissions and versions share.

use std::collections::BTreeSet;

use analyzer::render_card;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::TokenAccount;
use crate::entries::{self, NewEntry};
use crate::error::AppError;
use crate::jobs::hold_until;
use crate::links::{self, Linked, SlugRule};
use crate::queue;
use crate::respond::{DELIMITER, Format, card_and_warnings};
use crate::routes::submit::{Accepted, accept, cited_series, cited_set};
use crate::series::Cited;
use crate::{AppState, allowance, time};
use ::time::{Date, OffsetDateTime};

/// Every string in `value`, however deeply nested.
fn strings_in(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::String(text) => vec![text.as_str()],
        serde_json::Value::Array(items) => items.iter().flat_map(strings_in).collect(),
        serde_json::Value::Object(fields) => fields.values().flat_map(strings_in).collect(),
        _ => Vec::new(),
    }
}

/// The most members one batch may carry.
const MAX_MEMBERS: usize = 100;

#[derive(Deserialize)]
struct BatchBody {
    #[serde(default)]
    runs: Vec<Value>,
    entries: Vec<String>,
}

fn refused(reasons: Vec<String>) -> AppError {
    AppError::Refused {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        reasons,
    }
}

/// Checks a batch's run declarations and returns their IDs with the JSON to store.
async fn declared_runs(
    state: &AppState,
    account: &TokenAccount,
    runs: &[Value],
    problems: &mut Vec<String>,
) -> Result<Vec<(String, String)>, AppError> {
    let mut declared = Vec::new();
    for (position, run) in (1..).zip(runs) {
        let id = run["id"].as_str().map(str::trim).unwrap_or("");
        // Run IDs are shown beside the server's words, so they follow the slug rule.
        if !analyzer::is_valid_slug(id) || !run.is_object() {
            problems.push(format!(
                "run {position}: it needs an `id` of 1 to 64 letters, digits, `.`, `_` or `-`"
            ));
            continue;
        }
        if run["description"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        {
            problems.push(format!("run {position} (`{id}`): it needs a `description`"));
        }
        // Every string in the declaration, nested ones too, is shown with the
        // entries that use it.
        for text in strings_in(run) {
            for reason in analyzer::encoded_text(text) {
                problems.push(format!("run {position} (`{id}`): {reason}"));
            }
        }
        if declared
            .iter()
            .any(|(seen, _): &(String, String)| seen == id)
        {
            problems.push(format!("run `{id}` is declared twice in this batch"));
        } else if entries::run_declared(&state.pool, &account.id, id).await? {
            problems.push(format!(
                "run `{id}` is already declared; cite it in `uses_runs` without declaring it again"
            ));
        }
        declared.push((id.to_string(), run.to_string()));
    }
    Ok(declared)
}

pub async fn submit_batch(
    State(state): State<AppState>,
    account: TokenAccount,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let batch: BatchBody = serde_json::from_slice(&body).map_err(|error| {
        refused(vec![format!(
            "send JSON: {{\"runs\": [...], \"entries\": [\"<markdown>\", ...]}} ({error})"
        )])
    })?;
    if batch.entries.is_empty() || batch.entries.len() > MAX_MEMBERS {
        return Err(refused(vec![format!(
            "a batch holds 1 to {MAX_MEMBERS} entries"
        )]));
    }
    let mut problems = Vec::new();
    let runs = declared_runs(&state, &account, &batch.runs, &mut problems).await?;
    let mut accepted: Vec<Accepted> = Vec::new();
    let mut labels = Vec::new();
    for (position, text) in (1..).zip(&batch.entries) {
        let bytes = Bytes::copy_from_slice(text.as_bytes());
        let slug = analyzer::analyze(text).fields.slug;
        let label = match &slug {
            Some(slug) => format!("entry {position} (slug {slug})"),
            None => format!("entry {position}"),
        };
        match accept(&bytes, None) {
            Ok(member) => {
                if member.analysis.fields.slug.is_none() {
                    problems.push(format!("{label}: every entry in a batch needs a `slug`"));
                }
                accepted.push(member);
            }
            Err(AppError::Refused { reasons, .. }) => {
                problems.extend(
                    reasons
                        .into_iter()
                        .map(|reason| format!("{label}: {reason}")),
                );
            }
            Err(other) => return Err(other),
        }
        labels.push(label);
    }
    let run_ids: BTreeSet<String> = runs.iter().map(|(id, _)| id.clone()).collect();
    if problems.is_empty() {
        let linked: Vec<Linked<'_>> = accepted
            .iter()
            .zip(&labels)
            .map(|(member, label)| Linked {
                label: label.clone(),
                analysis: &member.analysis,
                source: &member.source,
                slug_rule: SlugRule::New,
                seq: None,
            })
            .collect();
        problems.extend(links::problems(&state, &account, &linked, &run_ids).await?);
    }
    // Each member's series cite and attachment set.
    let mut cites = Vec::new();
    for (member, label) in accepted.iter().zip(&labels) {
        let series = cited_series(&state, &account, member).await?;
        let set = cited_set(&state, &account, member).await?;
        match (series, set) {
            (Ok(series), Ok(set)) => cites.push((series, set)),
            (series, set) => {
                problems.extend(series.err().map(|reason| format!("{label}: {reason}")));
                problems.extend(set.err().map(|reason| format!("{label}: {reason}")));
            }
        }
    }
    if !problems.is_empty() {
        return Err(refused(problems));
    }

    let now = time::now_utc();
    let publish_at = hold_until(now);
    let stored = store(&state, &account, &runs, &accepted, &cites, &publish_at, now).await;
    let (batch_id, members) = match stored {
        Ok(Some(stored)) => stored,
        Ok(None) => return Err(allowance::refusal(accepted.len())),
        // Another request took one of these slugs or runs since the checks above.
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(AppError::Conflict(
                "a slug or run in this batch was taken by another submission at the same moment; submit the batch again".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let seqs: Vec<i64> = members.iter().map(|(seq, _, _)| *seq).collect();
    let screen_from = allowance::last_screen_from(&state.pool, &seqs, now.date()).await?;
    let members: Vec<(String, &Accepted)> = members
        .into_iter()
        .map(|(_, id, member)| (id, member))
        .collect();
    Ok(batch_reply(
        &state,
        &account,
        format,
        &batch_id,
        &publish_at,
        &members,
        screen_from,
    ))
}

/// Stores a checked batch in one transaction: the batch, its runs and its members,
/// each with its allowance unit. Returns each member's row and ID, or `None`, storing
/// nothing, when the account's allowance has no room for them all.
async fn store<'a>(
    state: &AppState,
    account: &TokenAccount,
    runs: &[(String, String)],
    accepted: &'a [Accepted],
    cites: &[(Option<Cited>, Option<i64>)],
    publish_at: &str,
    now: OffsetDateTime,
) -> Result<Option<(String, Vec<(i64, String, &'a Accepted)>)>, sqlx::Error> {
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    let (batch_seq, batch_id) = entries::insert_batch(&mut transaction, &account.id).await?;
    for (run_id, declaration) in runs {
        entries::insert_run(&mut transaction, &account.id, run_id, declaration).await?;
    }
    let mut members = Vec::new();
    for (member, (cited, set)) in accepted.iter().zip(cites) {
        let new = NewEntry {
            account_id: &account.id,
            source: &member.source,
            analysis: &member.analysis,
            hold_until: publish_at,
            previous: None,
            batch_seq: Some(batch_seq),
            register_slug: true,
            series: cited.as_ref(),
            attachments: *set,
        };
        let (seq, id) = entries::insert_on(&mut transaction, &new).await?;
        members.push((seq, id, member));
    }
    let seqs: Vec<i64> = members.iter().map(|(seq, _, _)| *seq).collect();
    if !allowance::take(&mut transaction, state, &account.id, &seqs, now.date()).await? {
        return Ok(None);
    }
    transaction.commit().await?;
    Ok(Some((batch_id, members)))
}

/// The reply to a batch: its fields and each member's ID and url above the delimiter,
/// then each member's slug, headline row, card and warnings below it.
fn batch_reply(
    state: &AppState,
    account: &TokenAccount,
    format: Format,
    batch_id: &str,
    publish_at: &str,
    members: &[(String, &Accepted)],
    screen_from: Option<Date>,
) -> Response {
    let publish = format!("{}/batches/{batch_id}/publish", state.public_url);
    let mut text = format!(
        "id: {batch_id}\nstatus: pending\npublish_at: {publish_at}\npublish: {publish}\n\
         Fix a member before then by sending it again with PUT to its url, or publish the whole batch now with a POST to the publish url.\n{}",
        allowance::screen_from_lines(screen_from)
    );
    for (id, _) in members {
        text.push_str(&format!("entry: {id} {}/e/{id}\n", state.public_url));
    }
    text.push_str(&format!("{DELIMITER}\n"));
    let mut json_members = Vec::new();
    for (id, member) in members {
        let context =
            entries::own_card_context(id, &account.id, &account.display_name, &member.analysis);
        let card = render_card(&member.analysis, &context);
        let warnings: Vec<String> = member
            .analysis
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect();
        text.push_str(&format!(
            "\nentry {id}, slug {}\nprimary row: {}\n{}",
            member.analysis.fields.slug.as_deref().unwrap_or(""),
            member.analysis.primary_row_summary(),
            card_and_warnings(&card, &warnings)
        ));
        json_members.push(json!({
            "id": id,
            "slug": member.analysis.fields.slug,
            "url": format!("{}/e/{id}", state.public_url),
            "primary_row": member.analysis.primary_row_summary(),
            "card": card,
            "warnings": warnings,
        }));
    }
    let json = json!({
        "id": batch_id,
        "status": "pending",
        "publish_at": publish_at,
        "screen_from": screen_from.map(allowance::day_text),
        "publish": publish,
        "entries": json_members,
    });
    format.reply(StatusCode::CREATED, text, json)
}

/// Ends a batch's hold early: every pending member is screened and placed now.
pub async fn publish_batch(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
) -> Result<Response, AppError> {
    let members = entries::batch_members(&state.pool, &id, &account.id)
        .await?
        .ok_or(AppError::NotFound)?;
    if members.is_empty() {
        return Err(AppError::Conflict(format!(
            "batch {id} has no pending entries; its hold has already ended"
        )));
    }
    let now = time::now_utc();
    // Every hold ends first, so the batch is decided once its last member is screened.
    for seq in &members {
        entries::start_screening(&state.pool, *seq, None).await?;
    }
    for seq in &members {
        queue::screen_now(&state, *seq, now).await?;
    }
    // Ending the hold early doesn't skip the account's queue.
    let screen_from = allowance::last_screen_from(&state.pool, &members, now.date()).await?;
    let statuses = entries::batch_statuses(&state.pool, &id).await?;
    let mut text = format!("id: {id}\n{}", allowance::screen_from_lines(screen_from));
    for (member, status) in &statuses {
        text.push_str(&format!("entry: {member} {status}\n"));
    }
    let json = json!({
        "id": id,
        "screen_from": screen_from.map(allowance::day_text),
        "entries": statuses.iter().map(|(member, status)| json!({"id": member, "status": status})).collect::<Vec<_>>(),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}
