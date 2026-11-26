//! Connecting an agent from the browser with a code (design.md, Accounts; #54), after
//! RFC 8628: `POST /device/code` and `POST /device/token`, which the agent sends with
//! no token, and `/device` and `/device/approve`, the pages its human opens, logged
//! in. The human types the code; no link carries it, so a page can't be opened
//! with a stranger's code already in it.

use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;

use super::account::SessionAccount;
use crate::AppState;
use crate::accounts;
use crate::device::{self, Poll};
use crate::error::AppError;
use crate::limits::{self, ClientIp, Counted};
use crate::pages::{DeviceConfirmPage, DevicePage, Message, page};
use crate::respond::Format;
use crate::time::now_utc;

/// The longest token name, as the account page's form takes.
const NAME_CHARS: usize = 100;

#[derive(Deserialize)]
pub struct AskForm {
    #[serde(default)]
    name: String,
}

/// `POST /device/code`: a new device code and user code, for an agent with no
/// token. Answers RFC 8628's fields, as `key: value` lines or, on request, JSON.
pub async fn ask(
    State(state): State<AppState>,
    ip: ClientIp,
    format: Format,
    headers: HeaderMap,
    Form(form): Form<AskForm>,
) -> Result<Response, AppError> {
    let name = if form.name.trim().is_empty() {
        device::DEFAULT_NAME
    } else {
        accounts::one_line_name(&form.name, NAME_CHARS).ok_or_else(|| {
            AppError::BadRequest(format!(
                "a token's name is 1 to {NAME_CHARS} characters on one line"
            ))
        })?
    };
    let limits = &state.rate_limits.limits;
    let counts = [
        Counted::device_codes_from(limits, &ip),
        Counted::device_codes(limits),
    ];
    limits::count_event(&state.pool, &counts, now_utc()).await?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let asked = device::ask(&state.pool, name, user_agent).await?;
    let verification_uri = format!("{}/device", state.public_url);
    let text = format!(
        "user_code: {}\nverification_uri: {verification_uri}\nexpires_in: {}\ninterval: {}\n\
         device_code: {}\n",
        asked.user_code, asked.expires_in, asked.interval, asked.device_code
    );
    let json = serde_json::json!({
        "device_code": asked.device_code,
        "user_code": asked.user_code,
        "verification_uri": verification_uri,
        "expires_in": asked.expires_in,
        "interval": asked.interval,
    });
    Ok(no_store(format.reply(StatusCode::OK, text, json)))
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(Deserialize)]
pub struct PollForm {
    device_code: String,
}

/// `POST /device/token`: the agent's poll. Once its human has approved, the reply's
/// body is the token's header line, as `POST /accounts` gives it, and
/// `X-Filedrawer-Account` names the account; until then, a 400 whose one line
/// starts with RFC 8628's error code.
pub async fn poll(
    State(state): State<AppState>,
    format: Format,
    Form(form): Form<PollForm>,
) -> Result<Response, AppError> {
    let (code, description, interval) = match device::poll(&state.pool, &form.device_code).await? {
        Poll::Approved { account_id, token } => {
            let account = HeaderValue::from_str(&account_id).expect("an account ID is ASCII");
            let json = serde_json::json!({
                "access_token": token,
                "token_type": "Bearer",
                "account": account_id,
            });
            let text = format!("Authorization: Bearer {token}\n");
            let mut reply = no_store(format.reply(StatusCode::OK, text, json));
            reply.headers_mut().insert(
                header::HeaderName::from_static("x-filedrawer-account"),
                account,
            );
            return Ok(reply);
        }
        Poll::Pending { interval } => (
            "authorization_pending",
            format!("your human hasn't approved the code yet; poll again in {interval} s"),
            Some(interval),
        ),
        Poll::SlowDown { interval } => (
            "slow_down",
            format!("polled too soon; wait {interval} s between polls from now on"),
            Some(interval),
        ),
        Poll::Denied => (
            "access_denied",
            "your human denied the code; ask for a new one only if they want you connected"
                .to_string(),
            None,
        ),
        Poll::Expired => (
            "expired_token",
            "no code waits with that device_code: it expired, or was used; ask for a new one"
                .to_string(),
            None,
        ),
    };
    let text = format!("{code}: {description}\n");
    let mut json = serde_json::json!({ "error": code, "error_description": description });
    if let Some(interval) = interval {
        json["interval"] = serde_json::json!(interval);
    }
    Ok(no_store(format.reply(StatusCode::BAD_REQUEST, text, json)))
}

/// The page that refuses an account whose address isn't verified, which gets no
/// token on the account page either.
fn verify_first() -> Response {
    page(
        StatusCode::FORBIDDEN,
        &Message {
            title: "Verify your email first",
            text: "This account can't connect an agent until its email address is verified. \
                   Open the link we emailed, or send another from your account page, then \
                   open this page again.",
            link: Some(("/account", "Your account")),
        },
    )
}

async fn verified(state: &AppState, account_id: &str) -> Result<bool, sqlx::Error> {
    Ok(accounts::view(&state.pool, account_id).await?.verified)
}

/// `GET /device`: the form for the code, for a session; without one, the login
/// page comes back here.
pub async fn page_for_code(
    State(state): State<AppState>,
    session: SessionAccount,
) -> Result<Response, AppError> {
    if !verified(&state, &session.id).await? {
        return Ok(verify_first());
    }
    Ok(page(StatusCode::OK, &DevicePage { error: None }))
}

#[derive(Deserialize)]
pub struct CodeForm {
    user_code: String,
}

/// The code a session typed, checked against the waiting requests, with each try
/// counted, and given back when it matched, as a login's is: so codes can't be
/// guessed at the request rate.
async fn find_waiting(
    state: &AppState,
    session: &SessionAccount,
    ip: &ClientIp,
    typed: &str,
) -> Result<Option<device::Waiting>, AppError> {
    let limits = &state.rate_limits.limits;
    let counts = [
        Counted::wrong_device_codes_of(limits, &session.id),
        Counted::wrong_device_codes_from(limits, ip),
    ];
    let slot = limits::take_slot(&state.pool, &counts, now_utc()).await?;
    let found = device::waiting(&state.pool, typed).await?;
    if found.is_some() {
        limits::give_back(&state.pool, slot).await?;
    }
    Ok(found)
}

fn no_such_code() -> Response {
    let error = format!(
        "No agent is waiting with that code. Check it against the one your agent showed you; \
         a code works for {} minutes, so an older one needs your agent to ask again.",
        device::EXPIRES_MINUTES
    );
    page(
        StatusCode::UNPROCESSABLE_ENTITY,
        &DevicePage {
            error: Some(&error),
        },
    )
}

/// `POST /device`: the code typed, answered with what approving it would do.
pub async fn typed(
    State(state): State<AppState>,
    session: SessionAccount,
    ip: ClientIp,
    Form(form): Form<CodeForm>,
) -> Result<Response, AppError> {
    if !verified(&state, &session.id).await? {
        return Ok(verify_first());
    }
    let Some(waiting) = find_waiting(&state, &session, &ip, &form.user_code).await? else {
        return Ok(no_such_code());
    };
    confirm_page(
        &state,
        &session.id,
        &form.user_code,
        &waiting,
        StatusCode::OK,
        None,
    )
    .await
}

async fn confirm_page(
    state: &AppState,
    account_id: &str,
    typed: &str,
    waiting: &device::Waiting,
    status: StatusCode,
    error: Option<&str>,
) -> Result<Response, AppError> {
    let view = accounts::view(&state.pool, account_id).await?;
    let template = DeviceConfirmPage {
        user_code: &device::shown(typed),
        name: &waiting.name,
        user_agent: waiting.user_agent.as_deref(),
        asked_at: &waiting.created_at,
        account_id: &view.id,
        display_name: &view.display_name,
        error,
    };
    Ok(page(status, &template))
}

#[derive(Deserialize)]
pub struct DecisionForm {
    user_code: String,
    #[serde(default)]
    name: String,
    decision: String,
}

/// `POST /device/approve`: the confirm page's buttons, which approve the request,
/// for the session's account, or deny it.
pub async fn decide(
    State(state): State<AppState>,
    session: SessionAccount,
    ip: ClientIp,
    Form(form): Form<DecisionForm>,
) -> Result<Response, AppError> {
    if !verified(&state, &session.id).await? {
        return Ok(verify_first());
    }
    let Some(waiting) = find_waiting(&state, &session, &ip, &form.user_code).await? else {
        return Ok(no_such_code());
    };
    match form.decision.as_str() {
        "deny" => {
            device::deny(&state.pool, &form.user_code).await?;
            Ok(page(
                StatusCode::OK,
                &Message {
                    title: "Denied",
                    text: "No token was made. If someone else gave you that code, they can't \
                           use it now.",
                    link: Some(("/account", "Your account")),
                },
            ))
        }
        "approve" => {
            let Some(name) = accounts::one_line_name(&form.name, NAME_CHARS) else {
                let error = format!("A token's name is 1 to {NAME_CHARS} characters on one line.");
                return confirm_page(
                    &state,
                    &session.id,
                    &form.user_code,
                    &waiting,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some(&error),
                )
                .await;
            };
            // Checked again with the write, so a code denied or expired since the
            // page was shown stays so.
            if !device::approve(&state.pool, &form.user_code, &session.id, name).await? {
                return Ok(no_such_code());
            }
            Ok(page(
                StatusCode::OK,
                &Message {
                    title: "Your agent is connected",
                    text: "It takes its token within a few seconds, and posts as this account \
                           from then on. The token is listed on your account page, where you \
                           can revoke it.",
                    link: Some(("/account", "Your account")),
                },
            ))
        }
        _ => Err(AppError::BadRequest(
            "`decision` is `approve` or `deny`".into(),
        )),
    }
}
