//! `GET /entries/mine` (design.md, Agent interface): a token's account's entries, so
//! an agent can skip results the account already posted.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::auth::TokenAccount;
use crate::entries;
use crate::error::AppError;
use crate::respond::{DELIMITER, Format, one_line};
use crate::routes::read::{DEFAULT_LIMIT, MAX_LIMIT};

#[derive(Debug, Deserialize)]
pub struct MineQuery {
    limit: Option<i64>,
    page: Option<i64>,
    /// A repo's URL: only entries whose `code` link starts with it.
    code: Option<String>,
}

/// The account's entries, newest first, removed ones included: the account above the
/// delimiter, then one line each, the ID, status, slug (`-` for none), title and
/// `code` link, separated by tabs, since the title and the link are author text.
pub async fn list(
    State(state): State<AppState>,
    account: TokenAccount,
    Query(query): Query<MineQuery>,
    format: Format,
) -> Result<Response, AppError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1).saturating_mul(limit);
    let repo = query
        .code
        .as_deref()
        .map(|repo| repo.trim().trim_end_matches('/'))
        .filter(|repo| !repo.is_empty());
    let listed = entries::mine(&state.pool, &account.id, repo, limit, offset).await?;
    let mut text = format!("{}entries: {}\n{DELIMITER}\n", account.line(), listed.len());
    for entry in &listed {
        let mut line = format!(
            "{}\t{}\t{}\t{}",
            entry.id,
            entry.status,
            entry.slug.as_deref().unwrap_or("-"),
            entry
                .title
                .as_deref()
                .map_or_else(|| "-".to_string(), one_line)
        );
        if let Some(code) = &entry.code {
            line.push('\t');
            line.push_str(&one_line(code));
        }
        text.push_str(&line);
        text.push('\n');
    }
    let json = json!({
        "account": account.json(),
        "entries": listed.iter().map(|entry| json!({
            "id": entry.id,
            "status": entry.status,
            "slug": entry.slug,
            "title": entry.title,
            "code": entry.code,
        })).collect::<Vec<_>>(),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}
