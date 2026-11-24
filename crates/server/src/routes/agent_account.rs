//! `POST /accounts`: an agent makes an account with no email address and no signup
//! (design.md, Accounts and Limits and abuse; #130). It takes no session and no
//! token, so it sits outside the browser's account routes and their origin check.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};

use super::account::is_site;
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::accounts::{self, AgentAccount};
use crate::error::AppError;
use crate::limits::{self, ClientIp, Counted};
use crate::time::now_utc;

/// The reply header that names the new account, since the body is only the token's
/// line, which the skill saves unread.
const ACCOUNT_HEADER: &str = "x-filedrawer-account";

/// Makes the account and answers 201 with its token as the header line the skills
/// keep, `Authorization: Bearer fdt_...`, this once.
pub async fn make(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // An agent sends no Origin; a page on another site that posts here does.
    if let Some(origin) = headers.get(header::ORIGIN)
        && !is_site(origin, &state)
    {
        return Err(AppError::Forbidden(
            "an account is made by an agent, not from a page on another site".into(),
        ));
    }
    let limits = &state.rate_limits.limits;
    let counts = [
        Counted::agent_accounts_from(limits, &ip),
        Counted::agent_accounts(limits),
    ];
    limits::count_event(&state.pool, &counts, now_utc())
        .await
        .map_err(|error| match error {
            // A person can still sign up, which this limit doesn't count.
            AppError::TooManyRequests {
                what,
                retry_after_secs,
                ..
            } => AppError::TooManyRequests {
                what,
                retry_after_secs,
                instead: Some(format!(
                    "your human can make an account at {}/signup instead",
                    state.public_url
                )),
            },
            other => other,
        })?;
    let AgentAccount { id, token } = accounts::make_by_agent(&state.pool).await?;
    let account = HeaderValue::from_str(&id).expect("an account ID is ASCII");
    Ok((
        StatusCode::CREATED,
        [
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::HeaderName::from_static(ACCOUNT_HEADER), account),
        ],
        format!("Authorization: Bearer {token}\n"),
    )
        .into_response())
}
