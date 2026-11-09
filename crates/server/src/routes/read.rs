//! Reading: `/e/<id>`, `/search`, `/cards.txt` and `/tags` (design.md, Agent interface).

use analyzer::{
    ANALYZER_VERSION, Claim, Kind, Outcome, Reference, Results, Stage, Vocabulary, analyze,
    row_note,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use analyzer::render_card;
use axum::http::HeaderMap;

use crate::auth::{TokenAccount, optional_token_account};
use crate::embargo::parse_date;
use crate::entries::{self, Filters};
use crate::error::AppError;
use crate::link_checks::{self, LinkFlags};
use crate::markdown;
use crate::pages::{
    Drawer, EntryPage, FileView, SearchPage, SeriesBlock, Stamp, fields_above, page,
};
use crate::respond::{
    DELIMITER, Format, Reading, author_name, card_and_warnings, echoed, one_line, wants_page,
};
use crate::series;
use crate::{AppState, allowance, attachments, deletion, time};
use analyzer::slug_references;
use url::Url;

/// `GET /e/<id>`: the entry as plain text, as JSON when asked, or as a page for a
/// browser. `/e/<id>.txt` is the plain text for every client, so a person can read
/// the entry as agents do.
pub async fn entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    format: Format,
) -> Result<Response, AppError> {
    let Reading {
        path: id,
        format,
        as_page,
    } = Reading::of(id, format, &headers);
    // The author's held entry first, then the public one. An entry only moves from a
    // held status to public, so this order can't miss one that publishes between the
    // two reads; the other order answered 404 for it, seen as smoke.rs's eval tests
    // failing when the screen published an entry they were polling.
    let account = optional_token_account(&headers, &state).await?;
    match own_entry(&state, &id, account.as_ref(), format).await {
        Err(AppError::NotFound) => {}
        shown => return shown,
    }
    let Some(entry) = entries::public_entry(&state.pool, &id).await? else {
        // The account that deleted it is told when; anyone else, nothing.
        if let Some(account) = &account
            && let Some(at) = deletion::deleted_entry(&state.pool, &account.id, &id).await?
        {
            return Err(AppError::Gone(format!("entry {id} was deleted at {at}")));
        }
        return Err(AppError::NotFound);
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
    let card = entry.card_from(
        &analysis,
        entry.superseded_by.clone().or(resolved_successor),
    );
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
    let mut run_notes = Vec::new();
    for (run, description) in &runs {
        match description {
            Some(description) => {
                let users = entries::run_entries(&state.pool, &entry.account_id, run).await?;
                text.push_str(&format!("run: {run}: entries {}\n", users.join(" ")));
                run_notes.push(format!("run: {run}: {}", one_line(description)));
            }
            None => text.push_str(&format!("run: {run}: description not public yet\n")),
        }
    }
    let mut descriptions: String = run_notes.iter().map(|note| format!("{note}\n")).collect();
    // The set's files: each path is author text, so the lines go below the delimiter.
    let set = attachments::entry_set(&state.pool, entry.seq).await?;
    let files_url = state.files_url.as_deref().unwrap_or("");
    if let Some(set) = &set {
        text.push_str(&format!(
            "attachments: {}, {} files\n",
            set.id,
            set.files.len()
        ));
        for file in &set.files {
            descriptions.push_str(&file_line(
                files_url,
                &file.sha256,
                &file.path,
                file.removed,
            ));
        }
    }
    if notes.is_empty() {
        text.push_str("rows: Results table not read\n");
    }
    for (number, note) in (1..).zip(&notes) {
        text.push_str(&format!("row {number}: {note}\n"));
    }
    if as_page {
        let files = set.as_ref().map_or_else(Vec::new, |set| {
            set.files
                .iter()
                .map(|file| FileView {
                    path: one_line(&file.path),
                    href: (!file.removed).then(|| format!("{files_url}/f/{}", file.sha256)),
                })
                .collect()
        });
        return Ok(entry_page(
            &state,
            &entry,
            &analysis,
            &text,
            &links,
            EntryParts {
                notes: run_notes,
                files,
                card: &card,
                cited: cited.as_ref(),
            },
        ));
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
        "attachments": set.as_ref().map(|set| json!({
            "id": set.id,
            "files": set.files.iter().map(|file| json!({
                "path": file.path,
                "origin": file.origin,
                "repo_url": file.repo_url,
                "sha256": file.sha256,
                "bytes": file.bytes,
                "url": (!file.removed).then(|| format!("{files_url}/f/{}", file.sha256)),
                "removed": file.removed,
            })).collect::<Vec<_>>(),
        })),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}

/// What an entry's page shows below the delimiter, besides the entry's own text.
struct EntryParts<'a> {
    /// Run descriptions, one line each.
    notes: Vec<String>,
    files: Vec<FileView>,
    card: &'a str,
    cited: Option<&'a CitedSeries>,
}

/// The public entry's page for a person: the server's fields as the plain text
/// `text` has them, the delimiter, then the card, the cited series and the entry's
/// markdown, rendered.
fn entry_page(
    state: &AppState,
    entry: &entries::PublicEntry,
    analysis: &analyzer::Analysis,
    text: &str,
    links: &[(String, String)],
    parts: EntryParts,
) -> Response {
    let base = Url::parse(&format!("{}/e/{}", state.public_url, entry.id)).ok();
    let (front_matter, body) = analyzer::front_matter_and_body(&entry.source).unwrap_or_default();
    let body = markdown::to_html(&body, base.as_ref(), links);
    let series = parts.cited.and_then(|cited| {
        let (title, description) = cited.shown.as_ref()?;
        let href = format!("/series/{}", cited.cite.text());
        let base = Url::parse(&format!("{}{href}", state.public_url)).ok();
        let rendered = markdown::to_html(description, base.as_ref(), &[]);
        Some((
            SeriesBlock {
                cite: cited.cite.text(),
                href,
                title: one_line(title),
                html: rendered.html,
            },
            rendered.dropped,
        ))
    });
    let dropped = body.dropped || series.as_ref().is_some_and(|(_, dropped)| *dropped);
    page(
        StatusCode::OK,
        &EntryPage {
            id: &entry.id,
            stamp: Stamp::of(&analysis.fields.outcome),
            fields: fields_above(text),
            dropped,
            title: analysis.fields.title.as_deref().map(one_line),
            notes: parts.notes,
            files: parts.files,
            card_lines: parts.card.lines().map(str::to_string).collect(),
            series: series.map(|(block, _)| block),
            front_matter,
            body: body.html,
        },
    )
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
    account: Option<&TokenAccount>,
    format: Format,
) -> Result<Response, AppError> {
    let Some(account) = account else {
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
                    | "frozen"
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
    // An embargoed version releases with its chain, so it shows the chain's date.
    let release = if entry.status == "embargoed" {
        let chain = entries::chain_embargo(&state.pool, entry.seq).await?;
        parse_date(chain.as_deref()).map(|date| date.to_string())
    } else {
        None
    };
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
    // Set only while its screening waits for the account's allowance.
    let screen_from =
        allowance::screen_from(&state.pool, entry.seq, time::now_utc().date()).await?;
    let mut text = format!(
        "id: {}\nstatus: {}\n{when}{}analyzer: v{ANALYZER_VERSION}\nurl: {}/e/{}\n",
        entry.id,
        entry.status,
        allowance::screen_from_lines(screen_from),
        state.public_url,
        entry.id
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
    let links = link_checks::stored(&state.pool, entry.seq).await?;
    let context = LinkFlags::of(&links).on(entries::own_card_context(
        &entry.id,
        &account.id,
        &account.display_name,
        &analysis,
    ));
    let card = render_card(&analysis, &context);
    let mut warnings: Vec<String> = analysis.warnings.iter().map(ToString::to_string).collect();
    warnings.extend(link_checks::warnings(&links));
    // A model wrote the reasons from the entry, so they are author text.
    let reasons: Vec<String> = entry
        .rejection_reasons
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    let mut reason_lines: String = reasons
        .iter()
        .map(|reason| format!("reason: {}\n", one_line(reason)))
        .collect();
    // Its attached files, as the public page will list them.
    let files_url = state.files_url.as_deref().unwrap_or("");
    for file in attachments::entry_files(&state.pool, entry.seq).await? {
        reason_lines.push_str(&file_line(
            files_url,
            &file.sha256,
            &file.path,
            file.removed,
        ));
    }
    text.push_str(&format!(
        "{DELIMITER}\n{reason_lines}{}\n{}",
        card_and_warnings(&card, &warnings),
        entry.source
    ));
    let json = json!({
        "id": entry.id,
        "status": entry.status,
        "publish_at": entry.hold_until,
        "screen_from": screen_from.map(allowance::day_text),
        "release": release,
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

/// An attached file's line on an entry's page, below the delimiter since its path is
/// author text: its URL on the files host, or, once the archive's owner removed it,
/// a line of its own that holds no URL.
fn file_line(files_url: &str, sha256: &str, path: &str, removed: bool) -> String {
    if removed {
        format!("removed_file: {}\n", one_line(path))
    } else {
        format!("file: {files_url}/f/{sha256} {}\n", one_line(path))
    }
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
    cites: Option<String>,
    limit: Option<i64>,
    page: Option<i64>,
}

pub(crate) const DEFAULT_LIMIT: i64 = 20;
pub(crate) const MAX_LIMIT: i64 = 100;

pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
    headers: HeaderMap,
    format: Format,
) -> Result<Response, AppError> {
    let filters = Filters {
        words: query.q.as_deref().and_then(entries::fts_query),
        outcome: known::<Outcome>("outcome", query.outcome.as_deref())?,
        kind: known::<Kind>("kind", query.kind.as_deref())?,
        stage: known::<Stage>("stage", query.stage.as_deref())?,
        tag: query.tag.clone().filter(|tag| !tag.is_empty()),
        model: query.model.clone().filter(|model| !model.is_empty()),
        row_model: query.row_model.clone().filter(|model| !model.is_empty()),
        row_claim: known::<Claim>("row_claim", query.row_claim.as_deref())?,
        cites: cited_key(query.cites.as_deref())?,
    };
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let page_number = query.page.unwrap_or(1).max(1);
    let offset = (page_number - 1).saturating_mul(limit);
    // One more than a page, to know whether another page follows.
    let mut results = entries::search(&state.pool, &filters, limit + 1, offset).await?;
    let more = i64::try_from(results.len()).is_ok_and(|found| found > limit);
    results.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    if wants_page(&headers) {
        let outcome = query.outcome.as_deref().unwrap_or("");
        let words = query.q.as_deref().unwrap_or("");
        return Ok(page(
            StatusCode::OK,
            &SearchPage {
                title: echoed(words),
                drawer: Drawer::new(outcome, words, &results),
                previous: (page_number > 1).then(|| query.page_href(page_number - 1)),
                next: more.then(|| query.page_href(page_number + 1)),
            },
        ));
    }
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

impl SearchQuery {
    /// The URL of page `number` of these results, with every filter kept.
    fn page_href(&self, number: i64) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in [
            ("q", &self.q),
            ("outcome", &self.outcome),
            ("kind", &self.kind),
            ("stage", &self.stage),
            ("tag", &self.tag),
            ("model", &self.model),
            ("row_model", &self.row_model),
            ("row_claim", &self.row_claim),
            ("cites", &self.cites),
        ] {
            if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
                query.append_pair(name, value);
            }
        }
        if let Some(limit) = self.limit {
            query.append_pair("limit", &limit.to_string());
        }
        query.append_pair("page", &number.to_string());
        format!("/search?{}", query.finish())
    }
}

/// A filter value checked against the field's vocabulary. Empty means no filter.
fn known<T: Vocabulary>(name: &str, value: Option<&str>) -> Result<Option<T>, AppError> {
    match value.filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => T::parse(value)
            .map(Some)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "`{name}={}` is not a value the server knows; see docs for {} (a null result is `no_effect`)",
                    echoed(value),
                    T::FIELD
                ))
            }),
    }
}

/// The key `cites=` asks for. A value in no form a reference takes is refused rather
/// than matching nothing, which would read as "nobody followed it up"; so is a slug,
/// which names an entry only within its own account.
fn cited_key(value: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match Reference::parse(value) {
        Some(Reference::Slug(_)) => Err(AppError::BadRequest(format!(
            "`cites={}` names a slug, which names an entry only within its author's account; use the entry's ID",
            echoed(value)
        ))),
        Some(reference) => Ok(Some(reference.key())),
        None => Err(AppError::BadRequest(format!(
            "`cites={}` isn't a form the server reads; give an entry ID, `arXiv:<id>`, a DOI or a URL",
            echoed(value)
        ))),
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
