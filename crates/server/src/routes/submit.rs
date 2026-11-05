//! `POST /entries` (design.md, Submission and screening; Entry status; Embargo).

use analyzer::{MAX_ENTRY_BYTES, Refusal, analyze, render_card, stored_text};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::auth::TokenAccount;
use crate::embargo::{Release, release};
use crate::entries;
use crate::error::AppError;
use crate::respond::Format;
use crate::screening::Verdict;
use crate::{AppState, time};

pub async fn submit(
    State(state): State<AppState>,
    account: TokenAccount,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    if body.len() > MAX_ENTRY_BYTES {
        return Err(AppError::Refused {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            reasons: vec![Refusal::TooLarge { bytes: body.len() }.to_string()],
        });
    }
    let text = std::str::from_utf8(&body).map_err(|_| AppError::Refused {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        reasons: vec!["the entry is not UTF-8 text".into()],
    })?;
    let source = stored_text(text);
    let analysis = analyze(&source);
    let mut reasons: Vec<String> = analysis.refusals.iter().map(ToString::to_string).collect();
    let release = release(analysis.fields.embargo_until.as_deref(), time::today())
        .map_err(|why| reasons.push(why))
        .ok();
    let Some(release) = release.filter(|_| reasons.is_empty()) else {
        return Err(AppError::Refused {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            reasons,
        });
    };

    let (seq, id) = entries::insert(&state.pool, &account.id, &source, &analysis).await?;
    let placed = match (state.screener.screen(&source).await, release) {
        (Verdict::Pass, Release::On(_)) => entries::embargo(&state.pool, seq).await?,
        (Verdict::Pass, Release::Now) => entries::publish(&state.pool, seq, &analysis).await?,
    };
    let status = placed.status();

    let context = entries::card_context(&id, &account.id, &account.display_name);
    let card = render_card(&analysis, &context);
    let warnings: Vec<String> = analysis.warnings.iter().map(ToString::to_string).collect();
    let release_line = match release {
        Release::On(date) => format!("release: {date}\n"),
        Release::Now => String::new(),
    };
    let mut text = format!(
        "id: {id}\nstatus: {status}\n{release_line}url: /e/{id}\nprimary row: {}\ncard:\n",
        analysis.primary_row_summary()
    );
    for line in card.lines() {
        text.push_str(&format!("  {line}\n"));
    }
    for warning in &warnings {
        text.push_str(&format!("warning: {warning}\n"));
    }
    let json = json!({
        "id": id,
        "status": status,
        "release": match release { Release::On(date) => Some(date.to_string()), Release::Now => None },
        "primary_row": analysis.primary_row_summary(),
        "card": card,
        "warnings": warnings,
    });
    Ok(format.reply(StatusCode::CREATED, text, json))
}
