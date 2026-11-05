//! Reading: `/e/<id>`, `/search`, `/cards.txt` and `/tags` (design.md, Agent interface).

use analyzer::{
    ANALYZER_VERSION, Claim, Kind, Outcome, Results, Stage, Vocabulary, analyze, row_note,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::entries::{self, Filters};
use crate::error::AppError;
use crate::respond::{DELIMITER, Format};

pub async fn entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    format: Format,
) -> Result<Response, AppError> {
    let entry = entries::public_entry(&state.pool, &id)
        .await?
        .ok_or(AppError::NotFound)?;
    let analysis = analyze(&entry.source);
    let notes: Vec<String> = match &analysis.results {
        Results::Read(table) => table.rows.iter().map(row_note).collect(),
        Results::NotRead(_) => Vec::new(),
    };
    let card = entry.card();
    let mut text = format!(
        "id: {}\nstatus: public\nanalyzer: v{ANALYZER_VERSION}\nsubmitted: {}\npublished: {}\nauthor: {}, account {}\n",
        entry.id,
        entry.submitted_at,
        entry.published_at.as_deref().unwrap_or(""),
        one_line(&entry.display_name),
        entry.account_id
    );
    if notes.is_empty() {
        text.push_str("rows: Results table not read\n");
    }
    for (number, note) in (1..).zip(&notes) {
        text.push_str(&format!("row {number}: {note}\n"));
    }
    text.push_str("card:\n");
    for line in card.lines() {
        text.push_str(&format!("  {line}\n"));
    }
    text.push_str(&format!("{DELIMITER}\n{}", entry.source));
    let json = json!({
        "id": entry.id,
        "status": "public",
        "analyzer_version": ANALYZER_VERSION,
        "submitted": entry.submitted_at,
        "published": entry.published_at,
        "author": {"display_name": entry.display_name, "account_id": entry.account_id},
        "row_notes": notes,
        "card": card,
        "text": entry.source,
    });
    Ok(format.reply(StatusCode::OK, text, json))
}

/// A display name on one line, so it can't forge a server field above the delimiter.
fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    outcome: Option<String>,
    kind: Option<String>,
    stage: Option<String>,
    tag: Option<String>,
    model: Option<String>,
    row_model: Option<String>,
    row_claim: Option<String>,
    limit: Option<i64>,
    page: Option<i64>,
}

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
    format: Format,
) -> Result<Response, AppError> {
    let filters = Filters {
        words: query.q.as_deref().and_then(entries::fts_query),
        outcome: known::<Outcome>("outcome", query.outcome)?,
        kind: known::<Kind>("kind", query.kind)?,
        stage: known::<Stage>("stage", query.stage)?,
        tag: query.tag.filter(|tag| !tag.is_empty()),
        model: query.model.filter(|model| !model.is_empty()),
        row_model: query.row_model.filter(|model| !model.is_empty()),
        row_claim: known::<Claim>("row_claim", query.row_claim)?,
    };
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = (query.page.unwrap_or(1).max(1) - 1).saturating_mul(limit);
    let results = entries::search(&state.pool, &filters, limit, offset).await?;
    let cards: Vec<(String, String)> = results
        .iter()
        .map(|entry| (entry.id.clone(), entry.card()))
        .collect();
    let text = if cards.is_empty() {
        "no results\n".to_string()
    } else {
        cards_text(cards.iter().map(|(_, card)| card.as_str()))
    };
    let json = json!({
        "results": cards.iter().map(|(id, card)| json!({"id": id, "card": card})).collect::<Vec<_>>(),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}

/// A filter value checked against the field's vocabulary. Empty means no filter.
fn known<T: Vocabulary>(name: &str, value: Option<String>) -> Result<Option<T>, AppError> {
    match value.filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => T::parse(&value)
            .map(Some)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "`{name}={value}` is not a value the server knows; see docs for {} (a null result is `no_effect`)",
                    T::FIELD
                ))
            }),
    }
}

/// Cards separated by blank lines.
fn cards_text<'a>(cards: impl Iterator<Item = &'a str>) -> String {
    cards.map(|card| format!("{card}\n\n")).collect()
}

pub async fn cards(State(state): State<AppState>) -> Result<String, AppError> {
    let entries = entries::all_public(&state.pool).await?;
    let cards: Vec<String> = entries.iter().map(|entry| entry.card()).collect();
    Ok(cards_text(cards.iter().map(String::as_str)))
}

pub async fn tags(State(state): State<AppState>, format: Format) -> Result<Response, AppError> {
    let tags = entries::tags(&state.pool).await?;
    let text: String = tags
        .iter()
        .map(|(tag, count)| format!("{tag} {count}\n"))
        .collect();
    let json = json!({
        "tags": tags.iter().map(|(tag, count)| json!({"tag": tag, "count": count})).collect::<Vec<_>>(),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}
