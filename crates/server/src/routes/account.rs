//! Accounts in the browser (design.md, Accounts): signup, email verification, login,
//! sessions, password reset, and API tokens. Every route here reads the session
//! cookie and never a token, so an agent's token can't reach them. Every link in an
//! email opens a page with a button, and the button's POST acts, since mail scanners
//! follow links.

use axum::Form;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::accounts::{self, EMAIL_LINK_HOURS, Login, Purpose, SESSION_DAYS, Signup};
use crate::error::AppError;
use crate::limits::{self, ClientIp, Counted};
use crate::mail_queue::{self, Queued};
use crate::pages::{
    AccountPage, DeletePage, LoginPage, Message, ResetConfirmPage, ResetPage, SignupPage,
    VerifyPage, page,
};
use crate::time::now_utc;
use crate::{AppState, admin, deletion, invites, passwords, series, withdraw};

const COOKIE: &str = "fd_session";

/// The account whose session cookie came with the request. Without a current session
/// the request is sent to the login page.
pub struct SessionAccount {
    pub id: String,
}

/// The `Set-Cookie` value that sets the session cookie to `secret` for `max_age`
/// seconds; an empty secret with 0 clears it. Lax, so a link from another site, such
/// as an email's, keeps the session (design.md, Accounts).
fn session_cookie(secret: &str, max_age: i64) -> String {
    format!("{COOKIE}={secret}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age}")
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

/// The account of the request's session, if it carries a live one; unlike
/// `SessionAccount`, no session is no error. The read counter uses it to know who
/// read, and verification to let a session stand in for the account's password.
pub async fn optional_session_account(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Option<String>, sqlx::Error> {
    match session_secret(headers) {
        Some(secret) => accounts::session_account(&state.pool, &secret).await,
        None => Ok(None),
    }
}

impl FromRequestParts<AppState> for SessionAccount {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let Some(secret) = session_secret(&parts.headers) else {
            return Err(to_login(parts));
        };
        match accounts::session_account(&state.pool, &secret).await {
            Ok(Some(id)) => Ok(SessionAccount { id }),
            Ok(None) => Err(to_login(parts)),
            Err(error) => Err(AppError::from(error).into_response()),
        }
    }
}

/// The login page, which returns a GET to the page it asked for once logged in. A
/// form's POST goes to the plain login page, since its target is no page to land on.
fn to_login(parts: &Parts) -> Response {
    let asked = parts.uri.path_and_query().map(|asked| asked.as_str());
    match asked {
        Some(asked) if parts.method == Method::GET || parts.method == Method::HEAD => {
            let next: String = url::form_urlencoded::byte_serialize(asked.as_bytes()).collect();
            Redirect::to(&format!("/login?next={next}")).into_response()
        }
        _ => Redirect::to("/login").into_response(),
    }
}

/// `next` if it names a page on this site; otherwise none, so a crafted login link
/// can't send a browser elsewhere. Browsers read `\` as `/`, so `/\host` would be
/// `//host`, another site; and a header takes only visible ASCII.
fn page_on_this_site(next: &str) -> Option<&str> {
    let on_site = next.starts_with('/')
        && !next.starts_with("//")
        && next
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'\\');
    on_site.then_some(next)
}

#[derive(Deserialize)]
pub struct SignupQuery {
    invite: Option<String>,
}

/// The signup form; a link such as `/signup?invite=inv-...` fills in the code.
pub async fn signup_page(Query(query): Query<SignupQuery>) -> Response {
    let template = SignupPage {
        invite: query.invite.as_deref().unwrap_or("").trim(),
        ..SignupPage::default()
    };
    page(StatusCode::OK, &template)
}

#[derive(Deserialize)]
pub struct SignupForm {
    email: String,
    password: String,
    display_name: String,
    #[serde(default)]
    invite: String,
}

const BAD_INVITE: &str = "That invite code can't be used: it may be mistyped, expired, revoked or used up. Leave it empty to sign up without one.";

pub async fn signup(
    State(state): State<AppState>,
    ip: ClientIp,
    Form(form): Form<SignupForm>,
) -> Result<Response, AppError> {
    let email = accounts::normalize_email(&form.email);
    let display_name = accounts::display_name(&form.display_name);
    let invite = Some(form.invite.trim()).filter(|code| !code.is_empty());
    let again = |problem: &str| {
        let template = SignupPage {
            error: Some(problem),
            email: &email,
            display_name: form.display_name.trim(),
            invite: invite.unwrap_or(""),
        };
        page(StatusCode::UNPROCESSABLE_ENTITY, &template)
    };
    let problem = if !accounts::looks_like_email(&email) {
        Some("That isn't an email address.".to_string())
    } else if let Some(problem) = passwords::problem(&form.password) {
        Some(problem)
    } else if display_name.is_none() {
        Some(format!(
            "A display name is 1 to {} characters on one line, with no invisible characters.",
            accounts::DISPLAY_NAME_CHARS
        ))
    } else {
        None
    };
    let (None, Some(display_name)) = (&problem, display_name) else {
        return Ok(again(&problem.unwrap_or_default()));
    };
    // Before anything is counted, so a refusal for want of a place costs no signup.
    let place = state.passwords.place().await?;
    // A code is checked before the limits it lifts; sign_up checks it again, with
    // the account's insert, for its last use.
    if let Some(code) = invite
        && invites::usable(&state.pool, code, now_utc())
            .await?
            .is_none()
    {
        // A wrong code counts against the IP's signups, so codes can't be guessed
        // at the request rate; the answer is the same for any address.
        let guess = [Counted::signups_from(&state.rate_limits.limits, &ip)];
        limits::count_event(&state.pool, &guess, now_utc()).await?;
        return Ok(again(BAD_INVITE));
    }
    // A signup sends an email either way, a verification or a note that the address
    // has an account, so it counts as one too. With a code, a sprint behind one IP
    // counts per address only.
    let limits = &state.rate_limits.limits;
    let mut counts = vec![Counted::emails_to(limits, &email)];
    if invite.is_none() {
        counts.push(Counted::signups_from(limits, &ip));
        counts.push(Counted::emails_from(limits, &ip));
    }
    limits::count_event(&state.pool, &counts, now_utc()).await?;
    let hash = place.hash(form.password).await;
    match accounts::sign_up(&state.pool, &email, &hash, display_name, invite).await? {
        Signup::BadInvite => return Ok(again(BAD_INVITE)),
        Signup::Created(id) => mail_queue::queue(&state, &id, Queued::Verify).await?,
        Signup::Taken(id) => mail_queue::queue(&state, &id, Queued::Taken).await?,
    }
    // An unverified account can log in, and its account page sends the link again.
    // Both arms above get this page, so it says nothing about which one ran.
    let text = format!(
        "We sent a link to {email}. Open it to check your email address and finish signing up. \
         No email after a few minutes? Check spam, then log in to send it again."
    );
    Ok(page(
        StatusCode::OK,
        &Message {
            title: "Check your email",
            text: &text,
            link: Some(("/login", "Log in")),
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

/// Whether the request carries a live session of `account_id`.
async fn session_of(
    headers: &HeaderMap,
    state: &AppState,
    account_id: &str,
) -> Result<bool, sqlx::Error> {
    let session = optional_session_account(headers, state).await?;
    Ok(session.as_deref() == Some(account_id))
}

/// The verify form again, with `error`.
fn verify_again(status: StatusCode, token: &str, error: &str) -> Response {
    page(
        status,
        &VerifyPage {
            token,
            ask_password: true,
            error: Some(error),
        },
    )
}

pub async fn verify_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Result<Response, AppError> {
    let Some(link) = accounts::link_account(&state.pool, &query.token, Purpose::Verify).await?
    else {
        return Ok(link_used_or_expired());
    };
    let logged_in = session_of(&headers, &state, &link.account_id).await?;
    Ok(page(
        StatusCode::OK,
        &VerifyPage {
            token: &query.token,
            ask_password: !logged_in,
            error: None,
        },
    ))
}

#[derive(Deserialize)]
pub struct VerifyForm {
    token: String,
    #[serde(default)]
    password: String,
}

/// Verifies the address of the link's account for whoever proves they made it: with
/// a session of the account or its password. The link alone would let the inbox's
/// owner verify an account someone else made with the address, who would then hold
/// it (AUTHZ-1); that owner resets the password instead.
pub async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<VerifyForm>,
) -> Result<Response, AppError> {
    let Some(link) = accounts::link_account(&state.pool, &form.token, Purpose::Verify).await?
    else {
        return Ok(link_used_or_expired());
    };
    if !session_of(&headers, &state, &link.account_id).await? {
        // A password signup would refuse can't match, so it counts nothing: an empty
        // field, or a session that ended since the page opened.
        if passwords::problem(&form.password).is_some() {
            let error = "Type the password you chose at signup.";
            return Ok(verify_again(StatusCode::BAD_REQUEST, &form.token, error));
        }
        if !password_matches(&state, &link.email, form.password, link.password_hash).await? {
            let error = "That isn't this account's password.";
            return Ok(verify_again(StatusCode::UNAUTHORIZED, &form.token, error));
        }
    }
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

#[derive(Deserialize)]
pub struct LoginQuery {
    next: Option<String>,
}

pub async fn login_page(Query(query): Query<LoginQuery>) -> Response {
    let template = LoginPage {
        next: query.next.as_deref().and_then(page_on_this_site),
        ..LoginPage::default()
    };
    page(StatusCode::OK, &template)
}

#[derive(Deserialize)]
pub struct LoginForm {
    email: String,
    password: String,
    /// The page to land on once logged in, from the login page's query.
    #[serde(default)]
    next: String,
}

pub async fn login(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let next = page_on_this_site(&form.next);
    let email = accounts::normalize_email(&form.email);
    let (account_id, stored) = match accounts::login_for_email(&state.pool, &email).await? {
        Some(login) => (Some(login.account_id), login.password_hash),
        None => (None, None),
    };
    // An unknown email still checks a decoy hash, so the answer takes as long.
    let matched = password_matches(&state, &email, form.password, stored).await?;
    let Some(account_id) = account_id.filter(|_| matched) else {
        let template = LoginPage {
            error: Some("The email or the password is wrong."),
            email: "",
            next,
        };
        return Ok(page(StatusCode::UNAUTHORIZED, &template));
    };
    let secret = accounts::new_session(&state.pool, &account_id).await?;
    let cookie = session_cookie(&secret, SESSION_DAYS * 24 * 60 * 60);
    let landing = Redirect::to(next.unwrap_or("/account"));
    Ok(([(header::SET_COOKIE, cookie)], landing).into_response())
}

/// Whether `password` matches `stored`, checked as one attempt on `email`'s
/// password, at login or verification. Failed attempts count per address, known or
/// not; once full, even the right password waits (design.md, Limits and abuse). The
/// attempt takes a hashing place first, so a refusal for want of one costs no failed
/// attempt, then its place among the failed ones, which a match gives back. With no
/// `stored`, it checks a decoy hash.
async fn password_matches(
    state: &AppState,
    email: &str,
    password: String,
    stored: Option<String>,
) -> Result<bool, AppError> {
    let place = state.passwords.place().await?;
    let failures = [Counted::failed_logins_to(&state.rate_limits.limits, email)];
    let slot = limits::take_slot(&state.pool, &failures, now_utc()).await?;
    let matched = place.matches(password, stored).await;
    if matched {
        limits::give_back(&state.pool, slot).await?;
    }
    Ok(matched)
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(secret) = session_secret(&headers) {
        accounts::end_session(&state.pool, &secret).await?;
    }
    let clear = session_cookie("", 0);
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
    ip: ClientIp,
    Form(form): Form<ResetForm>,
) -> Result<Response, AppError> {
    let email = accounts::normalize_email(&form.email);
    // Counted whether or not the address has an account, so a limit never says.
    limits::count_email(&state, Some(&ip), &email).await?;
    if let Some(Login { account_id, .. }) = accounts::login_for_email(&state.pool, &email).await? {
        mail_queue::queue(&state, &account_id, Queued::Reset).await?;
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
    if accounts::link_account(&state.pool, &query.token, Purpose::Reset)
        .await?
        .is_none()
    {
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
    // A dead link costs no hash. Answering it sooner tells nothing the form's page,
    // which says at once whether a link works, doesn't already.
    if accounts::link_account(&state.pool, &form.token, Purpose::Reset)
        .await?
        .is_none()
    {
        return Ok(link_used_or_expired());
    }
    let place = state.passwords.place().await?;
    let hash = place.hash(form.password).await;
    // The link is spent here, so one spent while the hash ran still doesn't work.
    if !accounts::reset_with_link(&state.pool, &form.token, &hash).await? {
        return Ok(link_used_or_expired());
    }
    Ok(page(
        StatusCode::OK,
        &Message {
            title: "Password changed",
            text: "Every session on this account has ended and its API tokens are revoked. \
                   Log in with the new password, then make a new token for each agent.",
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
    let withdrawable = withdraw::listing(&state.pool, account_id).await?;
    let template = AccountPage {
        series: &series,
        withdrawable: &withdrawable,
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
    let place = state.passwords.place().await?;
    if !place.matches(form.password, stored).await {
        let error = "The password is wrong.";
        return delete_form(&state, &session.id, StatusCode::UNAUTHORIZED, Some(error)).await;
    }
    deletion::delete(&state, &session.id).await?;
    let clear = session_cookie("", 0);
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

pub async fn resend_verification(
    State(state): State<AppState>,
    ip: ClientIp,
    session: SessionAccount,
) -> Result<Response, AppError> {
    let view = accounts::view(&state.pool, &session.id).await?;
    if let (Some(email), false) = (view.email, view.verified) {
        let invited = invites::invited(&state.pool, &session.id).await?;
        limits::count_email(&state, (!invited).then_some(&ip), &email).await?;
        mail_queue::queue(&state, &session.id, Queued::Verify).await?;
    }
    Ok(Redirect::to("/account").into_response())
}

/// Middleware for the account routes: a POST must carry an `Origin` header naming
/// this site. With the session cookie's `SameSite=Lax`, that stops another site
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
