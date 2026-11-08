//! The owner's admin pages (design.md, Admin). Every route reads the browser session
//! and never a token, so no agent's token can reach them, and an account that isn't
//! the admin gets the same 404 as a page that doesn't exist.

use analyzer::{analyze, render_card};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::link_checks::{self, LinkFlags};
use crate::pages::{
    AdminEntryPage, AdminFilePage, AdminInvitesPage, AdminQueuePage, QueueRow, ShownFile, page,
};
use crate::removed_files::{self, Removal};
use crate::review::{self, Decided, Decision};
use crate::routes::account::SessionAccount;
use crate::series::RefusalCall;
use crate::time::now_utc;
use crate::{AppState, accounts, attachments, entries, invites, series};
use axum::Form;
use serde::Deserialize;

/// The admin's browser session.
pub struct AdminSession;

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let session = SessionAccount::from_request_parts(parts, state).await?;
        match accounts::is_admin(&state.pool, &session.id).await {
            Ok(true) => Ok(AdminSession),
            Ok(false) => Err(AppError::NotFound.into_response()),
            Err(error) => Err(AppError::from(error).into_response()),
        }
    }
}

fn title_of(source: &str) -> String {
    analyze(source)
        .fields
        .title
        .unwrap_or_else(|| "(no title)".to_string())
}

/// The entries in review and held, and the accounts under the review guard.
pub async fn queue(_: AdminSession, State(state): State<AppState>) -> Result<Response, AppError> {
    let undecided = entries::undecided(&state.pool, None).await?;
    let rows: Vec<QueueRow> = undecided
        .iter()
        .map(|entry| QueueRow {
            title: title_of(&entry.source),
            entry,
        })
        .collect();
    let of = |status: &str| {
        rows.iter()
            .filter(|row| row.entry.status == status)
            .collect()
    };
    let (review, held, frozen) = (of("review"), of("held"), of("frozen"));
    let guarded = accounts::guarded(&state.pool).await?;
    let refusals = series::refusals(&state.pool).await?;
    let doubted = entries::doubted(&state.pool).await?;
    let doubted: Vec<_> = doubted
        .iter()
        .map(|entry| (entry, title_of(&entry.source)))
        .collect();
    Ok(page(
        StatusCode::OK,
        &AdminQueuePage {
            sections: [
                ("In review", review),
                ("Held until a vendor fixes the hole", held),
                ("Frozen by a revoked invite code", frozen),
            ],
            doubted: &doubted,
            refusals: &refusals,
            guarded: &guarded,
        },
    ))
}

/// One entry in review, held or frozen: its card, its text, and the owner's choices.
pub async fn entry(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let entry = entries::undecided(&state.pool, Some(&id))
        .await?
        .pop()
        .ok_or(AppError::NotFound)?;
    let analysis = analyze(&entry.source);
    let links = link_checks::stored(&state.pool, entry.seq).await?;
    let context = LinkFlags::of(&links).on(entries::card_context(
        &entry.id,
        &entry.account_id,
        entry.display_name.as_deref(),
    ));
    let card = render_card(&analysis, &context);
    // Shown with the entry once public, so the owner reads them too.
    let runs = entries::run_declarations(&state.pool, entry.seq).await?;
    let batch_failed = entries::batch_failed(&state.pool, entry.seq).await?;
    let held_back = entries::held_back_before(&state.pool, entry.seq).await?;
    // Its attached files, which a guarded account's entry reaches here unscreened.
    let mut files = Vec::new();
    for file in attachments::entry_files(&state.pool, entry.seq).await? {
        files.push(ShownFile {
            text: state.files.read(&file.sha256).await?,
            sha256: file.sha256,
            path: file.path,
            verdict: file.verdict,
            removed: file.removed,
            banned: file.banned,
        });
    }
    Ok(page(
        StatusCode::OK,
        &AdminEntryPage {
            title: &title_of(&entry.source),
            entry: &entry,
            card: &card,
            runs: &runs,
            batch_failed,
            files: &files,
            held_back: &held_back,
        },
    ))
}

async fn decide(state: &AppState, id: &str, decision: Decision) -> Result<Response, AppError> {
    match review::decide(state, id, decision, now_utc()).await? {
        Decided::Done => Ok(Redirect::to("/admin").into_response()),
        Decided::NoEntry => Err(AppError::NotFound),
        Decided::NotWaiting => Err(AppError::Conflict(format!(
            "entry {id} isn't waiting on that decision; reload the queue"
        ))),
    }
}

pub async fn publish(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::Publish).await
}

pub async fn hold(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::Hold).await
}

pub async fn reject(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::Reject).await
}

/// Rejects an entry in review or held, and the earlier versions of its chain that
/// are held back with it.
pub async fn reject_chain(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::RejectChain).await
}

/// Releases a frozen entry back to screening.
pub async fn release(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide(&state, &id, Decision::Release).await
}

/// Revokes an invite code, freezing its accounts' unscreened entries.
pub async fn revoke_invite(
    _: AdminSession,
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Response, AppError> {
    match invites::revoke(&state.pool, &code, now_utc()).await? {
        Some(_) => Ok(Redirect::to("/admin/invites").into_response()),
        None => Err(AppError::NotFound),
    }
}

async fn decide_refusal(
    state: &AppState,
    id: &str,
    call: RefusalCall,
) -> Result<Response, AppError> {
    if !series::decide_refusal(&state.pool, id, call).await? {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to("/admin").into_response())
}

/// Confirms a refused series as a hazard: its account goes under the review guard.
pub async fn confirm_refusal(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide_refusal(&state, &id, RefusalCall::Confirm).await
}

/// Dismisses a refused series: a false flag, so no guard.
pub async fn dismiss_refusal(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    decide_refusal(&state, &id, RefusalCall::Dismiss).await
}

/// The owner's page of invite codes, with the form that makes one.
pub async fn invites(_: AdminSession, State(state): State<AppState>) -> Result<Response, AppError> {
    invites_page(&state, StatusCode::OK, None).await
}

async fn invites_page(
    state: &AppState,
    status: StatusCode,
    error: Option<&str>,
) -> Result<Response, AppError> {
    let codes = invites::list(&state.pool).await?;
    Ok(page(
        status,
        &AdminInvitesPage {
            codes: &codes,
            error,
            public_url: &state.public_url,
            max_uses: invites::MAX_USES,
            max_days: invites::MAX_DAYS,
        },
    ))
}

#[derive(Deserialize)]
pub struct InviteForm {
    label: String,
    uses: String,
    days: String,
}

/// Makes an invite code from the owner's form.
pub async fn create_invite(
    _: AdminSession,
    State(state): State<AppState>,
    Form(form): Form<InviteForm>,
) -> Result<Response, AppError> {
    let label = accounts::one_line_name(&form.label, 100);
    let uses = whole_number_to(&form.uses, invites::MAX_USES);
    let days = whole_number_to(&form.days, invites::MAX_DAYS);
    let (Some(label), Some(uses), Some(days)) = (label, uses, days) else {
        let problem = format!(
            "A code needs a label of 1 to 100 characters on one line, 1 to {} uses, and 1 to {} days.",
            invites::MAX_USES,
            invites::MAX_DAYS
        );
        return invites_page(&state, StatusCode::UNPROCESSABLE_ENTITY, Some(&problem)).await;
    };
    invites::create(&state.pool, label, uses, days, now_utc()).await?;
    Ok(Redirect::to("/admin/invites").into_response())
}

/// `text` as a whole number from 1 to `max`, if it is one.
fn whole_number_to(text: &str, max: i64) -> Option<i64> {
    text.trim().parse().ok().filter(|n| (1..=max).contains(n))
}

/// Takes an entry off the list of those that passed with a hazard doubt.
pub async fn doubt_seen(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    if !entries::mark_doubt_seen(&state.pool, &id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to("/admin").into_response())
}

pub async fn clear_guard(
    _: AdminSession,
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> Result<Response, AppError> {
    if !accounts::clear_guard(&state.pool, &account_id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to("/admin").into_response())
}

/// One attached file, by its SHA-256 (design.md, Attached code): its text, where it is
/// used, and the owner's two ways to remove it.
pub async fn file(
    _: AdminSession,
    State(state): State<AppState>,
    Path(sha256): Path<String>,
) -> Result<Response, AppError> {
    let file = removed_files::owners_file(&state.pool, &sha256)
        .await?
        .ok_or(AppError::NotFound)?;
    let text = state.files.read(&file.sha256).await?;
    let uses = removed_files::file_uses(&state.pool, &file.sha256).await?;
    Ok(page(
        StatusCode::OK,
        &AdminFilePage {
            file: &file,
            text: text.as_deref(),
            uses: &uses,
        },
    ))
}

async fn remove(state: &AppState, sha256: &str, removal: Removal) -> Result<Response, AppError> {
    if !removed_files::remove(&state.pool, sha256, removal).await? {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to(&format!("/admin/f/{sha256}")).into_response())
}

/// Stops the file being served for any entry; its bytes and record stay.
pub async fn remove_file(
    _: AdminSession,
    State(state): State<AppState>,
    Path(sha256): Path<String>,
) -> Result<Response, AppError> {
    remove(&state, &sha256, Removal::Plain).await
}

/// Removes the file and bans its hash, and guards every account that uploaded it.
pub async fn remove_hazard(
    _: AdminSession,
    State(state): State<AppState>,
    Path(sha256): Path<String>,
) -> Result<Response, AppError> {
    remove(&state, &sha256, Removal::Hazard).await
}
