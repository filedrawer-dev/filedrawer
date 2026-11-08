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
use crate::entries::{self, NewEntry, OwnEntry};
use crate::error::AppError;
use crate::jobs::{end_hold, hold_until};
use crate::link_checks::{self, CheckedLink, LinkFlags};
use crate::links::{self, Linked, SlugRule};
use crate::respond::{DELIMITER, Format, card_and_warnings};
use crate::series::{self, Cited};
use crate::{AppState, allowance, attachments, removed_files, time};
use ::time::format_description::well_known::Rfc3339;
use ::time::{Date, OffsetDateTime};
use analyzer::{SeriesCite, parse_series_cite, without_series};

/// A submission that passed every check: the text to store, what the analyzer read
/// from it, and the series version it cites, which the stored text no longer names
/// (design.md, Series).
pub(crate) struct Accepted {
    pub source: String,
    pub analysis: Analysis,
    pub cite: Option<SeriesCite>,
}

/// What an earlier submission fixes for a resubmission or a new version: the first
/// submission date the embargo cap counts from, and the embargo in force, which a
/// token may not drop or bring within 7 days.
pub(crate) struct Prior<'a> {
    first_submitted_at: &'a str,
    embargo_in_force: Option<&'a str>,
}

impl<'a> Prior<'a> {
    /// For a new version of `entry`. Its embargo is in force only while it is
    /// embargoed; a public entry's old date no longer holds anything back.
    fn of(entry: &'a OwnEntry) -> Prior<'a> {
        Prior {
            first_submitted_at: &entry.first_submitted_at,
            embargo_in_force: entry
                .embargo_until
                .as_deref()
                .filter(|_| entry.status == "embargoed"),
        }
    }

    /// For a PUT on the pending `entry`. A draft's own embargo holds nothing back yet,
    /// so its author may drop or shorten it (the owner's decision, 2026-11-05). A
    /// pending new version of an embargoed entry is bound by that entry's embargo.
    fn for_put(entry: &'a OwnEntry, previous: Option<&'a OwnEntry>) -> Prior<'a> {
        Prior {
            first_submitted_at: &entry.first_submitted_at,
            embargo_in_force: previous.and_then(|previous| Prior::of(previous).embargo_in_force),
        }
    }
}

/// Runs every check a submission must pass, given what an earlier submission fixes,
/// and lists every reason it fails.
pub(crate) fn accept(body: &Bytes, prior: Option<Prior<'_>>) -> Result<Accepted, AppError> {
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
    let (source, cite) = match analysis.fields.series.as_deref() {
        None => (source, None),
        Some(value) => {
            // A cite the analyzer can't read is among its refusals already.
            let cite = parse_series_cite(value);
            let stored = without_series(&source);
            if analyze(&stored).fields.series.is_some() {
                reasons.push(
                    "the server couldn't take `series` out of this front matter; put it on a line of its own".into(),
                );
            }
            (stored, cite)
        }
    };
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
    Ok(Accepted {
        source,
        analysis,
        cite,
    })
}

/// Checks the series version an accepted entry cites against the database. `Err`
/// holds the reason to refuse it.
pub(crate) async fn cited_series(
    state: &AppState,
    account: &TokenAccount,
    accepted: &Accepted,
) -> Result<Result<Option<Cited>, String>, AppError> {
    let Some(cite) = &accepted.cite else {
        return Ok(Ok(None));
    };
    Ok(series::check_cite(&state.pool, &account.id, cite)
        .await?
        .map(Some))
}

/// The attachment set an accepted entry cites, which must be its account's own.
/// `Err` holds the reason to refuse it.
pub(crate) async fn cited_set(
    state: &AppState,
    account: &TokenAccount,
    accepted: &Accepted,
) -> Result<Result<Option<i64>, String>, AppError> {
    let Some(id) = accepted.analysis.fields.attachments.as_deref() else {
        return Ok(Ok(None));
    };
    let Some(seq) = attachments::owned_set(&state.pool, &account.id, id).await? else {
        return Ok(Err(format!(
            "`attachments: {id}` names no attachment set this account uploaded; upload one with POST /attachments"
        )));
    };
    let banned = removed_files::banned_paths(&state.pool, seq).await?;
    if !banned.is_empty() {
        return Ok(Err(format!(
            "`attachments: {id}` holds {}, {}; upload a set without it",
            banned.join(", "),
            removed_files::REMOVED_BY_OWNER
        )));
    }
    Ok(Ok(Some(seq)))
}

/// `cited_set` for one entry, refusing it with the reason.
async fn one_set(
    state: &AppState,
    account: &TokenAccount,
    accepted: &Accepted,
) -> Result<Option<i64>, AppError> {
    cited_set(state, account, accepted)
        .await?
        .map_err(|reason| AppError::Refused {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            reasons: vec![reason],
        })
}

/// `cited_series` for one entry, refusing it with the reason.
async fn one_cite(
    state: &AppState,
    account: &TokenAccount,
    accepted: &Accepted,
) -> Result<Option<Cited>, AppError> {
    cited_series(state, account, accepted)
        .await?
        .map_err(|reason| AppError::Refused {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            reasons: vec![reason],
        })
}

/// Checks one entry's `code` and `logs` links online, refusing it when a pin is bad.
async fn one_check(
    state: &AppState,
    account: &TokenAccount,
    accepted: &Accepted,
) -> Result<Vec<CheckedLink>, AppError> {
    let links = link_checks::check(state, &[&accepted.analysis.fields], Some(&account.id))
        .await
        .pop()
        .unwrap_or_default();
    let reasons = link_checks::refusals(&links);
    if !reasons.is_empty() {
        return Err(AppError::Refused {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            reasons,
        });
    }
    Ok(links)
}

/// A held entry, as a reply describes it.
struct Held<'a> {
    id: &'a str,
    account: &'a TokenAccount,
    analysis: &'a Analysis,
    links: &'a [CheckedLink],
    publish_at: &'a str,
    /// The entry this one is a new version of.
    previous: Option<&'a str>,
    /// The day its screening waits for, when the account's allowance is taken
    /// until then.
    screen_from: Option<Date>,
}

/// The reply to a submission or resubmission: the server's fields, then below the
/// delimiter the headline row as read, the card and the warnings.
fn held_reply(state: &AppState, format: Format, status: StatusCode, held: &Held<'_>) -> Response {
    let Held {
        id,
        account,
        analysis,
        links,
        publish_at,
        previous,
        screen_from,
    } = *held;
    let context = LinkFlags::of(links).on(entries::own_card_context(
        id,
        &account.id,
        &account.display_name,
        analysis,
    ));
    let card = render_card(analysis, &context);
    let mut warnings: Vec<String> = analysis.warnings.iter().map(ToString::to_string).collect();
    warnings.extend(link_checks::warnings(links));
    let embargo = parse_date(analysis.fields.embargo_until.as_deref()).map(|date| date.to_string());
    let previous_line = previous.map_or(String::new(), |id| format!("previous: {id}\n"));
    let embargo_line = embargo
        .as_deref()
        .map_or(String::new(), |date| format!("embargo_until: {date}\n"));
    let text = format!(
        "id: {id}\nstatus: pending\npublish_at: {publish_at}\n{embargo_line}{previous_line}url: {}/e/{id}\n\
         Fix it before then by sending the whole entry again with PUT to the url, or publish it now with a POST to the url plus /publish.\n\
         {}{DELIMITER}\nprimary row: {}\n{}",
        state.public_url,
        allowance::screen_from_lines(screen_from),
        analysis.primary_row_summary(),
        card_and_warnings(&card, &warnings)
    );
    let json = json!({
        "id": id,
        "status": "pending",
        "publish_at": publish_at,
        "screen_from": screen_from.map(allowance::day_text),
        "embargo_until": embargo,
        "previous": previous,
        "url": format!("{}/e/{id}", state.public_url),
        "primary_row": analysis.primary_row_summary(),
        "card": card,
        "warnings": warnings,
    });
    format.reply(status, text, json)
}

/// Takes the allowance unit of entry `seq`, which `account_id` has just stored in
/// `transaction`, and commits; refuses the request, storing nothing, when the
/// account's allowance has no room (design.md, Limits and abuse). Returns the day its
/// screening waits for, if that is after `today`.
async fn commit_with_unit(
    state: &AppState,
    mut transaction: sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: &str,
    seq: i64,
    today: Date,
) -> Result<Option<Date>, AppError> {
    if !allowance::take(&mut transaction, state, account_id, &[seq], today).await? {
        return Err(allowance::refusal(1));
    }
    transaction.commit().await?;
    Ok(allowance::screen_from(&state.pool, seq, today).await?)
}

pub async fn submit(
    State(state): State<AppState>,
    account: TokenAccount,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let accepted = accept(&body, None)?;
    check_references(&state, &account, &accepted, SlugRule::New, None).await?;
    let cited = one_cite(&state, &account, &accepted).await?;
    let set = one_set(&state, &account, &accepted).await?;
    let links = one_check(&state, &account, &accepted).await?;
    let now = time::now_utc();
    let publish_at = hold_until(now);
    let new = NewEntry {
        account_id: &account.id,
        source: &accepted.source,
        analysis: &accepted.analysis,
        hold_until: &publish_at,
        previous: None,
        batch_seq: None,
        register_slug: true,
        series: cited.as_ref(),
        attachments: set,
    };
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    let (seq, id) = entries::insert_on(&mut transaction, &new).await?;
    link_checks::store(&mut transaction, seq, &links, now).await?;
    let screen_from = commit_with_unit(&state, transaction, &account.id, seq, now.date()).await?;
    let held = Held {
        id: &id,
        account: &account,
        analysis: &accepted.analysis,
        links: &links,
        publish_at: &publish_at,
        previous: None,
        screen_from,
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
    let chain_slug = entries::chain_slug(&state.pool, entry.seq).await?;
    let register_slug = chain_slug.is_none() && accepted.analysis.fields.slug.is_some();
    check_references(
        &state,
        &account,
        &accepted,
        SlugRule::Version(chain_slug),
        None,
    )
    .await?;
    let cited = one_cite(&state, &account, &accepted).await?;
    let set = one_set(&state, &account, &accepted).await?;
    let links = one_check(&state, &account, &accepted).await?;
    let now = time::now_utc();
    let publish_at = hold_until(now);
    let new = NewEntry {
        account_id: &account.id,
        source: &accepted.source,
        analysis: &accepted.analysis,
        hold_until: &publish_at,
        previous: Some(entries::Previous {
            seq: entry.seq,
            first_submitted_at: &entry.first_submitted_at,
        }),
        batch_seq: None,
        register_slug,
        series: cited.as_ref(),
        attachments: set,
    };
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    let inserted = entries::insert_on(&mut transaction, &new).await;
    let (seq, new_id) = match inserted {
        Ok(inserted) => inserted,
        // Another version won the race between the check above and this insert; the
        // index allows one live successor.
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            // Rolled back first: the lookup below needs a connection of its own.
            drop(transaction);
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
    link_checks::store(&mut transaction, seq, &links, now).await?;
    let screen_from = commit_with_unit(&state, transaction, &account.id, seq, now.date()).await?;
    let held = Held {
        id: &new_id,
        account: &account,
        analysis: &accepted.analysis,
        links: &links,
        publish_at: &publish_at,
        previous: Some(&id),
        screen_from,
    };
    Ok(held_reply(&state, format, StatusCode::CREATED, &held))
}

/// Refuses one entry whose slug, runs or references don't check out. `seq` is the
/// stored entry a PUT replaces.
async fn check_references(
    state: &AppState,
    account: &TokenAccount,
    accepted: &Accepted,
    slug_rule: SlugRule,
    seq: Option<i64>,
) -> Result<(), AppError> {
    let linked = [Linked {
        label: "the entry".into(),
        analysis: &accepted.analysis,
        source: &accepted.source,
        slug_rule,
        seq,
    }];
    let problems = links::problems(state, account, &linked, &Default::default()).await?;
    if problems.is_empty() {
        return Ok(());
    }
    Err(AppError::Refused {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        reasons: problems,
    })
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
    let previous = match &entry.previous_id {
        Some(previous) => entries::own_entry(&state.pool, previous, &account.id).await?,
        None => None,
    };
    let accepted = accept(&body, Some(Prior::for_put(&entry, previous.as_ref())))?;
    // A slug names the entry for good, and other entries may already refer to it.
    let kept = analyzer::analyze(&entry.source).fields.slug;
    check_references(
        &state,
        &account,
        &accepted,
        SlugRule::Put(kept),
        Some(entry.seq),
    )
    .await?;
    let cited = one_cite(&state, &account, &accepted).await?;
    let set = one_set(&state, &account, &accepted).await?;
    let links = one_check(&state, &account, &accepted).await?;
    let now = time::now_utc();
    let publish_at = hold_until(now);
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    let replaced = entries::replace_on(
        &mut transaction,
        entry.seq,
        &accepted.source,
        &accepted.analysis,
        &publish_at,
        cited.as_ref(),
        set,
    )
    .await?;
    if !replaced {
        return Err(AppError::Conflict(format!(
            "entry {id}'s hold ended while it was being replaced"
        )));
    }
    link_checks::store(&mut transaction, entry.seq, &links, now).await?;
    // The entry keeps the earliest day among its units, so a PUT never delays it.
    let screen_from =
        commit_with_unit(&state, transaction, &account.id, entry.seq, now.date()).await?;
    let held = Held {
        id: &id,
        account: &account,
        analysis: &accepted.analysis,
        links: &links,
        publish_at: &publish_at,
        previous: entry.previous_id.as_deref(),
        screen_from,
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
    if entry.batch_seq.is_some() {
        return Err(AppError::Conflict(format!(
            "entry {id} came in a batch, which publishes as one: POST to its batch's publish url"
        )));
    }
    let now = time::now_utc();
    let Some(status) = end_hold(&state, entry.seq, now).await? else {
        return Err(AppError::Conflict(format!(
            "entry {id}'s hold has already ended"
        )));
    };
    // Ending the hold early doesn't skip the account's queue.
    let screen_from = allowance::screen_from(&state.pool, entry.seq, now.date()).await?;
    let release = entry
        .embargo_until
        .clone()
        .filter(|_| status == "embargoed");
    let release_line = release
        .as_deref()
        .map_or(String::new(), |date| format!("release: {date}\n"));
    let text = format!(
        "id: {id}\nstatus: {status}\n{release_line}url: {}/e/{id}\n{}",
        state.public_url,
        allowance::screen_from_lines(screen_from)
    );
    let json = json!({
        "id": id,
        "status": status,
        "screen_from": screen_from.map(allowance::day_text),
        "release": release,
        "url": format!("{}/e/{id}", state.public_url),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}
