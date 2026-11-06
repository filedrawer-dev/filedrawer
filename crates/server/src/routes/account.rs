//! Accounts in the browser (design.md, Accounts): signup, email verification, login,
//! sessions, password reset, and API tokens. Every route here reads the session
//! cookie and never a token, so an agent's token can't reach them. Every link in an
//! email opens a page with a button, and the button's POST acts, since mail scanners
//! follow links.

use axum::Form;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::accounts::{self, EMAIL_LINK_HOURS, Login, Purpose, SESSION_DAYS, Signup};
use crate::error::AppError;
use crate::mail::Email;
use crate::pages::{
    AccountPage, DeletePage, LoginPage, Message, ResetConfirmPage, ResetPage, SignupPage,
    VerifyPage, page,
};
use crate::{AppState, admin, deletion, jobs, passwords, series};

const COOKIE: &str = "fd_session";

/// The account whose session cookie came with the request. Without a current session
/// the request is sent to the login page.
pub struct SessionAccount {
    pub id: String,
}

/// The session cookie's secret, if the request carries one.
fn session_secret(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            pair.trim()
                .strip_prefix(&format!("{COOKIE}="))
                .map(str::to_string)
        })
}

impl FromRequestParts<AppState> for SessionAccount {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let Some(secret) = session_secret(&parts.headers) else {
            return Err(Redirect::to("/login").into_response());
        };
        match accounts::session_account(&state.pool, &secret).await {
            Ok(Some(id)) => Ok(SessionAccount { id }),
            Ok(None) => Err(Redirect::to("/login").into_response()),
            Err(error) => Err(AppError::from(error).into_response()),
        }
    }
}

/// Hashes on a blocking thread: argon2id takes tens of milliseconds of CPU.
async fn hash_password(password: String) -> String {
    tokio::task::spawn_blocking(move || passwords::hash(&password))
        .await
        .expect("hashing a password doesn't panic")
}

async fn password_matches(password: String, stored: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || passwords::matches(&password, stored.as_deref()))
        .await
        .expect("checking a password doesn't panic")
}

/// Sends an email in the background, so a page that sends one answers as fast as one
/// that doesn't, and whether an account exists never shows in the timing. A failure
/// is logged; the page says the same either way.
fn send(state: &AppState, to: &str, subject: &str, body: String) {
    let email = Email {
        to: to.to_string(),
        subject: subject.to_string(),
        body,
    };
    crate::mail::send_later(&state.mailer, email);
}

async fn send_verification(state: &AppState, account_id: &str, to: &str) -> Result<(), AppError> {
    let secret = accounts::new_email_link(&state.pool, account_id, Purpose::Verify).await?;
    let body = format!(
        "Someone, probably you, signed up for File Drawer with this address.\n\n\
         To verify it, open this link and press the button:\n{}/verify?token={secret}\n\n\
         The link works for {EMAIL_LINK_HOURS} hours. If you didn't sign up, ignore this email.\n",
        state.public_url
    );
    send(state, to, "Verify your email for File Drawer", body);
    Ok(())
}

pub async fn signup_page() -> Response {
    page(StatusCode::OK, &SignupPage::default())
}

#[derive(Deserialize)]
pub struct SignupForm {
    email: String,
    password: String,
    display_name: String,
}

pub async fn signup(
    State(state): State<AppState>,
    Form(form): Form<SignupForm>,
) -> Result<Response, AppError> {
    let email = accounts::normalise_email(&form.email);
    let display_name = accounts::one_line_name(&form.display_name, 50);
    let problem = if !accounts::looks_like_email(&email) {
        Some("That isn't an email address.".to_string())
    } else if let Some(problem) = passwords::problem(&form.password) {
        Some(problem)
    } else if display_name.is_none() {
        Some("A display name is 1 to 50 characters on one line.".to_string())
    } else {
        None
    };
    let (None, Some(display_name)) = (&problem, display_name) else {
        let problem = problem.unwrap_or_default();
        let template = SignupPage {
            error: Some(&problem),
            email: &email,
            display_name: form.display_name.trim(),
        };
        return Ok(page(StatusCode::UNPROCESSABLE_ENTITY, &template));
    };
    let hash = hash_password(form.password).await;
    match accounts::sign_up(&state.pool, &email, &hash, display_name).await? {
        Signup::Created(id) => send_verification(&state, &id, &email).await?,
        Signup::Taken => {
            let body = format!(
                "Someone tried to sign up for File Drawer with this address, which already \
                 has an account.\n\nIf it was you, log in at {url}/login, or choose a new \
                 password at {url}/reset. If it wasn't, ignore this email.\n",
                url = state.public_url
            );
            send(
                &state,
                &email,
                "Someone tried to sign up with your email",
                body,
            );
        }
    }
    let text = format!(
        "We sent a link to {email}. Open it to check your email address and finish signing up."
    );
    Ok(page(
        StatusCode::OK,
        &Message {
            title: "Check your email",
            text: &text,
            link: None,
        },
    ))
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: String,
}

fn link_used_or_expired() -> Response {
    let text = format!("It was used or expired. Links work once, for {EMAIL_LINK_HOURS} hours.");
    page(
        StatusCode::BAD_REQUEST,
        &Message {
            title: "This link doesn't work",
            text: &text,
            link: Some(("/login", "Log in")),
        },
    )
}

pub async fn verify_page(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, AppError> {
    if !accounts::email_link_works(&state.pool, &query.token, Purpose::Verify).await? {
        return Ok(link_used_or_expired());
    }
    Ok(page(
        StatusCode::OK,
        &VerifyPage {
            token: &query.token,
        },
    ))
}

pub async fn verify(
    State(state): State<AppState>,
    Form(form): Form<TokenQuery>,
) -> Result<Response, AppError> {
    if !accounts::verify_with_link(&state.pool, &form.token).await? {
        return Ok(link_used_or_expired());
    }
    Ok(page(
        StatusCode::OK,
        &Message {
            title: "Email verified",
            text: "Your agents can now submit entries with your account's tokens.",
            link: Some(("/account", "Your account")),
        },
    ))
}

pub async fn login_page() -> Response {
    page(StatusCode::OK, &LoginPage::default())
}

#[derive(Deserialize)]
pub struct LoginForm {
    email: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let email = accounts::normalise_email(&form.email);
    let (account_id, stored) = match accounts::login_for_email(&state.pool, &email).await? {
        Some(login) => (Some(login.account_id), login.password_hash),
        None => (None, None),
    };
    // An unknown email still checks a decoy hash, so the answer takes as long.
    let matched = password_matches(form.password, stored).await;
    let Some(account_id) = account_id.filter(|_| matched) else {
        let template = LoginPage {
            error: Some("The email or the password is wrong."),
            email: "",
        };
        return Ok(page(StatusCode::UNAUTHORIZED, &template));
    };
    let secret = accounts::new_session(&state.pool, &account_id).await?;
    let cookie = format!(
        "{COOKIE}={secret}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
        SESSION_DAYS * 24 * 60 * 60
    );
    Ok(([(header::SET_COOKIE, cookie)], Redirect::to("/account")).into_response())
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(secret) = session_secret(&headers) {
        accounts::end_session(&state.pool, &secret).await?;
    }
    let clear = format!("{COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0");
    Ok(([(header::SET_COOKIE, clear)], Redirect::to("/login")).into_response())
}

pub async fn reset_page() -> Response {
    page(StatusCode::OK, &ResetPage)
}

#[derive(Deserialize)]
pub struct ResetForm {
    email: String,
}

pub async fn reset(
    State(state): State<AppState>,
    Form(form): Form<ResetForm>,
) -> Result<Response, AppError> {
    let email = accounts::normalise_email(&form.email);
    if let Some(Login { account_id, .. }) = accounts::login_for_email(&state.pool, &email).await? {
        let secret = accounts::new_email_link(&state.pool, &account_id, Purpose::Reset).await?;
        let body = format!(
            "Someone, probably you, asked to choose a new File Drawer password for this \
             address.\n\nTo choose one, open this link:\n{}/reset/confirm?token={secret}\n\n\
             The link works for {EMAIL_LINK_HOURS} hours. If you didn't ask, ignore this \
             email; your password hasn't changed.\n",
            state.public_url
        );
        send(&state, &email, "Choose a new File Drawer password", body);
    }
    let text = format!(
        "If an account uses that address, we sent it a link to choose a new password. It works for {EMAIL_LINK_HOURS} hours."
    );
    Ok(page(
        StatusCode::OK,
        &Message {
            title: "Check your email",
            text: &text,
            link: None,
        },
    ))
}

pub async fn reset_confirm_page(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, AppError> {
    if !accounts::email_link_works(&state.pool, &query.token, Purpose::Reset).await? {
        return Ok(link_used_or_expired());
    }
    let template = ResetConfirmPage {
        token: &query.token,
        error: None,
    };
    Ok(page(StatusCode::OK, &template))
}

#[derive(Deserialize)]
pub struct ResetConfirmForm {
    token: String,
    password: String,
}

pub async fn reset_confirm(
    State(state): State<AppState>,
    Form(form): Form<ResetConfirmForm>,
) -> Result<Response, AppError> {
    if let Some(problem) = passwords::problem(&form.password) {
        let template = ResetConfirmPage {
            token: &form.token,
            error: Some(&problem),
        };
        return Ok(page(StatusCode::UNPROCESSABLE_ENTITY, &template));
    }
    let hash = hash_password(form.password).await;
    if !accounts::reset_with_link(&state.pool, &form.token, &hash).await? {
        return Ok(link_used_or_expired());
    }
    Ok(page(
        StatusCode::OK,
        &Message {
            title: "Password changed",
            text: "Every session on this account has ended. Log in with the new password.",
            link: Some(("/login", "Log in")),
        },
    ))
}

async fn account_page(
    state: &AppState,
    account_id: &str,
    new_token: Option<&str>,
) -> Result<Response, AppError> {
    let view = accounts::view(&state.pool, account_id).await?;
    let series = series::created_by(&state.pool, account_id).await?;
    let template = AccountPage {
        series: &series,
        display_name: &view.display_name,
        account_id: &view.id,
        email: view.email.as_deref(),
        verified: view.verified,
        tokens: &view.tokens,
        new_token,
    };
    Ok(page(StatusCode::OK, &template))
}

pub async fn account(
    State(state): State<AppState>,
    session: SessionAccount,
) -> Result<Response, AppError> {
    account_page(&state, &session.id, None).await
}

#[derive(Deserialize)]
pub struct TokenForm {
    name: String,
}

pub async fn create_token(
    State(state): State<AppState>,
    session: SessionAccount,
    Form(form): Form<TokenForm>,
) -> Result<Response, AppError> {
    let Some(name) = accounts::one_line_name(&form.name, 100) else {
        return Err(AppError::BadRequest(
            "a token's name is 1 to 100 characters on one line".into(),
        ));
    };
    // A token is the only way to act for an account, so an address nobody has proved
    // gets none: whoever later resets it through its inbox would inherit them.
    if !accounts::view(&state.pool, &session.id).await?.verified {
        return Err(AppError::Forbidden(
            "verify this account's email before making a token".into(),
        ));
    }
    let token = admin::create_token(&state.pool, &session.id, name)
        .await
        .map_err(|error| match error {
            admin::AdminError::Database(error) => AppError::Database(error),
            other => AppError::BadRequest(other.to_string()),
        })?;
    account_page(&state, &session.id, Some(&token)).await
}

pub async fn revoke_token(
    State(state): State<AppState>,
    session: SessionAccount,
    Path(token_id): Path<String>,
) -> Result<Response, AppError> {
    if !accounts::revoke_token(&state.pool, &session.id, &token_id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to("/account").into_response())
}

/// The confirm page for deleting the account: what goes, what stays, and anything
/// that stops it.
pub async fn delete_page(
    State(state): State<AppState>,
    session: SessionAccount,
) -> Result<Response, AppError> {
    delete_form(&state, &session.id, StatusCode::OK, None).await
}

async fn delete_form(
    state: &AppState,
    account_id: &str,
    status: StatusCode,
    error: Option<&str>,
) -> Result<Response, AppError> {
    let preview = deletion::preview(&state.pool, account_id).await?;
    let template = DeletePage {
        error,
        unpublished: preview.unpublished,
        public: preview.public,
        blocking: &preview.blocking,
    };
    Ok(page(status, &template))
}

#[derive(Deserialize)]
pub struct DeleteForm {
    password: String,
}

/// Deletes the account once its password is confirmed (the owner's decision,
/// 2026-11-05: everything not yet public goes, public entries stay without the
/// name), then ends the session.
pub async fn delete_account(
    State(state): State<AppState>,
    session: SessionAccount,
    Form(form): Form<DeleteForm>,
) -> Result<Response, AppError> {
    let stored = deletion::password_hash(&state.pool, &session.id).await?;
    if !password_matches(form.password, stored).await {
        let error = "The password is wrong.";
        return delete_form(&state, &session.id, StatusCode::UNAUTHORIZED, Some(error)).await;
    }
    if let deletion::Outcome::Blocked = deletion::delete(&state, &session.id).await? {
        // A series became blocking since the page was shown; it lists them.
        return delete_form(&state, &session.id, StatusCode::CONFLICT, None).await;
    }
    let clear = format!("{COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0");
    let done = page(
        StatusCode::OK,
        &Message {
            title: "Account deleted",
            text: "Your account is deleted. Its public entries stay up as \"account deleted\", with no name.",
            link: None,
        },
    );
    Ok(([(header::SET_COOKIE, clear)], done).into_response())
}

/// Makes one of the account's series public, then places the entries that waited on
/// it (design.md, Series: "its creator makes it public from a browser session").
pub async fn publish_series(
    State(state): State<AppState>,
    session: SessionAccount,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    if !series::make_public(&state.pool, &id, &session.id).await? {
        return Err(AppError::NotFound);
    }
    jobs::place_waiting(&state).await?;
    Ok(Redirect::to("/account").into_response())
}

pub async fn resend_verification(
    State(state): State<AppState>,
    session: SessionAccount,
) -> Result<Response, AppError> {
    let view = accounts::view(&state.pool, &session.id).await?;
    if let (Some(email), false) = (view.email, view.verified) {
        send_verification(&state, &session.id, &email).await?;
    }
    Ok(Redirect::to("/account").into_response())
}

/// Middleware for the account routes: a POST must carry an `Origin` header naming
/// this site. With the session cookie's `SameSite=Strict`, that stops another site
/// from posting a form on a logged-in visitor's behalf. A POST with no Origin is
/// refused too, since every browser that matters sends one.
pub async fn same_origin(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request.method() == axum::http::Method::POST {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok());
        if origin != Some(state.public_url.as_str()) {
            return (
                StatusCode::FORBIDDEN,
                format!("refused: forms must be sent from {}\n", state.public_url),
            )
                .into_response();
        }
    }
    next.run(request).await
}
