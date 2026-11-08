//! The account behind an API token (design.md, Accounts).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};

use crate::error::AppError;
use crate::respond::one_line;
use crate::{AppState, ids};

/// The account a request's `Authorization: Bearer` token belongs to. Only an account
/// with a verified email gets this far (design.md, Accounts: "An account can post
/// only after its email is verified").
#[derive(Debug, Clone)]
pub struct TokenAccount {
    pub id: String,
    pub display_name: String,
    /// The token's own ID, which each entry it submits records.
    pub token_id: String,
}

impl TokenAccount {
    /// The reply line that names the account, above the delimiter: its ID and its
    /// display name on one line.
    pub fn line(&self) -> String {
        format!("account: {} ({})\n", self.id, one_line(&self.display_name))
    }

    /// The account, as a JSON reply names it.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"id": self.id, "display_name": self.display_name})
    }
}

struct TokenRow {
    id: String,
    display_name: String,
    verified: bool,
    token_id: String,
}

impl FromRequestParts<AppState> for TokenAccount {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let account = token_account(&parts.headers, state).await?;
        // A write counts against the account's rate, refused or not (#14).
        if !matches!(parts.method, Method::GET | Method::HEAD) {
            state.rate_limits.write_by(&account.id)?;
        }
        Ok(account)
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

/// The API token in the request's `Authorization: Bearer` header, if it has one. A
/// header of another scheme, such as the `Basic` one a browser sends through
/// staging's password, holds no token.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token)
}

async fn token_account(headers: &HeaderMap, state: &AppState) -> Result<TokenAccount, AppError> {
    let token = bearer_token(headers).ok_or(AppError::Unauthorized)?;
    let hash = ids::token_hash(token.trim());
    let row = sqlx::query_as!(
        TokenRow,
        r#"SELECT a.id, a.display_name, a.email_verified_at IS NOT NULL AS "verified!: bool",
                  t.id AS token_id
               FROM tokens t JOIN accounts a ON a.id = t.account_id
               WHERE t.hash = ? AND t.revoked_at IS NULL AND a.deleted_at IS NULL"#,
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
        token_id: row.token_id,
    })
}
