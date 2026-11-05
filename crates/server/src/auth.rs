//! The account behind an API token (design.md, Accounts).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};

use crate::error::AppError;
use crate::{AppState, ids};

/// The account a request's `Authorization: Bearer` token belongs to. Only an account
/// with a verified email gets this far (design.md, Accounts: "An account can post
/// only after its email is verified").
#[derive(Debug, Clone)]
pub struct TokenAccount {
    pub id: String,
    pub display_name: String,
}

struct TokenRow {
    id: String,
    display_name: String,
    verified: bool,
}

impl FromRequestParts<AppState> for TokenAccount {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        token_account(&parts.headers, state).await
    }
}

/// The account behind the request's token, for a route that also serves requests
/// without one: `None` when there is no token, or it isn't a working one.
pub async fn optional_token_account(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Option<TokenAccount>, AppError> {
    match token_account(headers, state).await {
        Ok(account) => Ok(Some(account)),
        Err(AppError::Database(error)) => Err(AppError::Database(error)),
        Err(_) => Ok(None),
    }
}

async fn token_account(headers: &HeaderMap, state: &AppState) -> Result<TokenAccount, AppError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token)
        .ok_or(AppError::Unauthorized)?;
    let hash = ids::token_hash(token.trim());
    let row = sqlx::query_as!(
        TokenRow,
        r#"SELECT a.id, a.display_name, a.email_verified_at IS NOT NULL AS "verified!: bool"
               FROM tokens t JOIN accounts a ON a.id = t.account_id
               WHERE t.hash = ? AND t.revoked_at IS NULL"#,
        hash
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::Unauthorized)?;
    if !row.verified {
        return Err(AppError::Forbidden(
                "verify this account's email before submitting: open the link we emailed, or send another from your account page".into(),
            ));
    }
    Ok(TokenAccount {
        id: row.id,
        display_name: row.display_name,
    })
}
