//! Reading: `/e/<id>`, `/search`, `/cards.txt` and `/tags` (design.md, Agent interface).

use analyzer::{
    ANALYZER_VERSION, Claim, Kind, Outcome, Results, Stage, Vocabulary, analyze, row_note,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use analyzer::render_card;
use axum::http::HeaderMap;

use crate::AppState;
use crate::auth::optional_token_account;
use crate::embargo::parse_date;
use crate::entries::{self, Filters};
use crate::error::AppError;
use crate::respond::{DELIMITER, Format, author_name, card_and_warnings, one_line};
use crate::series;
use analyzer::slug_references;

pub async fn entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    format: Format,
) -> Result<Response, AppError> {
    let Some(entry) = entries::public_entry(&state.pool, &id).await? else {
        return own_entry(&state, &id, &headers, format).await;
    };
    let analysis = analyze(&entry.source);
    let notes: Vec<String> = match &analysis.results {
        Results::Read(table) => table.rows.iter().map(row_note).collect(),
        Results::NotRead(_) => Vec::new(),
    };
    // `slug:<name>` references resolve here, at read time (design.md, Agent
    // interface), to the latest public entry with that slug in the author's account.
    let mut links = Vec::new();
    for name in slug_references(&entry.source) {
        if let Some(id) = entries::public_by_slug(&state.pool, &entry.account_id, &name).await? {
            links.push((name, id));
        }
    }
    let successor_slug = analysis
        .fields
        .superseded_by
        .as_deref()
        .and_then(|value| value.strip_prefix("slug:"));
    let resolved_successor = successor_slug
        .and_then(|name| links.iter().find(|(slug, _)| slug == name))
        .map(|(_, id)| id.clone());
    let card = entry.card_with(entry.superseded_by.clone().or(resolved_successor));
    let runs = entries::entry_runs(&state.pool, &entry.id).await?;
    let cited = shown_series(&state, entry.seq).await?;
    let mut text = format!(
        "id: {}\nstatus: public\nanalyzer: v{ANALYZER_VERSION}\nsubmitted: {}\npublished: {}\nauthor: {}, account {}\n",
        entry.id,
        entry.submitted_at,
        entry.published_at.as_deref().unwrap_or(""),
        author_name(entry.display_name.as_deref()),
        entry.account_id
    );
    if let Some(previous) = &entry.previous_id {
        text.push_str(&format!("previous: {previous}\n"));
    }
    if let Some(newer) = &entry.superseded_by {
        text.push_str(&format!("superseded_by: {newer}\n"));
    }
    // Series IDs are the server's and anchors are checked to `[a-z0-9_-]`, so the
    // cite can't forge a line.
    if let Some(cited) = &cited {
        text.push_str(&format!("series: {}\n", cited.cite.text()));
        if cited.shown.is_none() {
            text.push_str("series_description: not public\n");
        }
    }
    // Slug names are author text, but only ones made of [A-Za-z0-9._-] are read, so
    // they can't forge a line.
    for (name, id) in &links {
        text.push_str(&format!("link: slug:{name} = {id}\n"));
    }
    // A run's description and its list of entries become public together, once every
    // entry using it is. The description is author text, so it goes below the
    // delimiter; run IDs follow the slug rule, so they can't forge a line.
    let mut descriptions = String::new();
    for (run, description) in &runs {
        match description {
            Some(description) => {
                let users = entries::run_entries(&state.pool, &entry.account_id, run).await?;
                text.push_str(&format!("run: {run}: entries {}\n", users.join(" ")));
                descriptions.push_str(&format!("run: {run}: {}\n", one_line(description)));
            }
            None => text.push_str(&format!("run: {run}: description not public yet\n")),
        }
    }
    if notes.is_empty() {
        text.push_str("rows: Results table not read\n");
    }
    for (number, note) in (1..).zip(&notes) {
        text.push_str(&format!("row {number}: {note}\n"));
    }
    // The card holds author text (the title, hypothesis, tags and more), so it goes
    // below the delimiter with the entry itself.
    text.push_str(&format!(
        "{DELIMITER}\n{descriptions}{}\n{}",
        card_and_warnings(&card, &[]),
        entry.source
    ));
    if let Some(CitedSeries {
        cite,
        shown: Some((title, description)),
    }) = &cited
    {
        text.push_str(&format!(
            "\n=== Series {}: {} ===\n\n{description}\n",
            cite.text(),
            one_line(title)
        ));
    }
    let json = json!({
        "id": entry.id,
        "status": "public",
        "analyzer_version": ANALYZER_VERSION,
        "submitted": entry.submitted_at,
        "published": entry.published_at,
        "previous": entry.previous_id,
        "superseded_by": entry.superseded_by,
        "author": {"display_name": entry.display_name, "account_id": entry.account_id},
        "row_notes": notes,
        "card": card,
        "text": entry.source,
        "series": cited.as_ref().map(|cited| json!({
            "id": cited.cite.series_id,
            "version": cited.cite.version,
            "section": cited.cite.section,
            "title": cited.shown.as_ref().map(|(title, _)| title),
            "description": cited.shown.as_ref().map(|(_, description)| description),
        })),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}

/// The series version an entry cites, with its title and description (or the cited
/// section of it) when anyone may read that version.
struct CitedSeries {
    cite: series::EntryCite,
    shown: Option<(String, String)>,
}

async fn shown_series(state: &AppState, seq: i64) -> Result<Option<CitedSeries>, AppError> {
    let Some(cite) = series::entry_cite(&state.pool, seq).await? else {
        return Ok(None);
    };
    let version = series::version(&state.pool, cite.series_seq, cite.version).await?;
    let shown = version.filter(|version| version.readable).map(|version| {
        // Submission checked the section, and versions never change; should it be
        // missing anyway, the whole description is shown rather than nothing.
        let section = cite
            .section
            .as_deref()
            .and_then(|section| analyzer::description_section(&version.description, section));
        let description = section.map_or_else(|| version.description.clone(), str::to_string);
        (version.title, description)
    });
    Ok(Some(CitedSeries { cite, shown }))
}

/// An entry that isn't public, for its author's token only: a pending entry with the
/// time it publishes and its warnings, or an embargoed one with its release date.
/// Anyone else gets the same 404 as for an entry that doesn't exist.
async fn own_entry(
    state: &AppState,
    id: &str,
    headers: &HeaderMap,
    format: Format,
) -> Result<Response, AppError> {
    let Some(account) = optional_token_account(headers, state).await? else {
        return Err(AppError::NotFound);
    };
    let entry = entries::own_entry(&state.pool, id, &account.id)
        .await?
        .filter(|entry| {
            matches!(
                entry.status.as_str(),
                "pending"
                    | "screening"
                    | "review"
                    | "held"
                    | "rejected"
                    | "embargoed"
                    | "waiting_on_series"
            )
        })
        .ok_or(AppError::NotFound)?;
    let analysis = analyze(&entry.source);
    let notes: Vec<String> = match &analysis.results {
        Results::Read(table) => table.rows.iter().map(row_note).collect(),
        Results::NotRead(_) => Vec::new(),
    };
    let release = parse_date(entry.embargo_until.as_deref()).map(|date| date.to_string());
    let when = match entry.status.as_str() {
        "pending" => format!(
            "publish_at: {}\n",
            entry.hold_until.as_deref().unwrap_or("")
        ),
        "embargoed" => format!(
            "release: {}\n",
            release.as_deref().unwrap_or("not recorded")
        ),
        _ => String::new(),
    };
    let mut text = format!(
        "id: {}\nstatus: {}\n{when}analyzer: v{ANALYZER_VERSION}\nurl: {}/e/{}\n",
        entry.id, entry.status, state.public_url, entry.id
    );
    if let Some(previous) = &entry.previous_id {
        text.push_str(&format!("previous: {previous}\n"));
    }
    let cite = series::entry_cite(&state.pool, entry.seq).await?;
    if let Some(cite) = &cite {
        text.push_str(&format!("series: {}\n", cite.text()));
    }
    for (number, note) in (1..).zip(&notes) {
        text.push_str(&format!("row {number}: {note}\n"));
    }
    let context = entries::card_context(&entry.id, &account.id, Some(&account.display_name));
    let card = render_card(&analysis, &context);
    let warnings: Vec<String> = analysis.warnings.iter().map(ToString::to_string).collect();
    // A model wrote the reasons from the entry, so they are author text.
    let reasons: Vec<String> = entry
        .rejection_reasons
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    let reason_lines: String = reasons
        .iter()
        .map(|reason| format!("reason: {}\n", one_line(reason)))
        .collect();
    text.push_str(&format!(
        "{DELIMITER}\n{reason_lines}{}\n{}",
        card_and_warnings(&card, &warnings),
        entry.source
    ));
    let json = json!({
        "id": entry.id,
        "status": entry.status,
        "publish_at": entry.hold_until,
        "release": release.filter(|_| entry.status == "embargoed"),
        "analyzer_version": ANALYZER_VERSION,
        "row_notes": notes,
        "card": card,
        "warnings": warnings,
        "reasons": reasons,
        "text": entry.source,
        "series": cite.as_ref().map(series::EntryCite::text),
    });
    Ok(format.reply(StatusCode::OK, text, json))
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
