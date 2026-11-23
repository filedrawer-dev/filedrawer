//! GitHub sign-in in the browser (design.md, Accounts; #119): signing in and up with
//! GitHub, linking it to an account and unlinking it. Every start is a POST from
//! this site, so another site or a crawler can't begin one; the browser that
//! started it must finish it, through a cookie only it holds.

use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Redirect, Response};
use serde::Deserialize;

use super::account::{
    SessionAccount, cookie_value, optional_session_account, page_on_this_site, session_cookie,
};
use crate::accounts::{self, SESSION_DAYS};
use crate::error::AppError;
use crate::github_accounts::{self, Finished, Linking, Purpose};
use crate::github_login::{CALLBACK_PATH, Failure, GithubLogin};
use crate::limits::{self, ClientIp, Counted};
use crate::mail_queue::{self, Queued};
use crate::pages::{GithubSignupPage, Message, page};
use crate::time::now_utc;
use crate::{AppState, email_change, invites};

/// The browser's cookie for its sign-ins under way. Its path is the whole site, so
/// a browser sends it to the next start too, which keeps it, and a second tab
/// doesn't strand the first's sign-in.
const START_COOKIE: &str = "fd_github";
/// The browser's cookie for its signup waiting for the form.
const SIGNUP_COOKIE: &str = "fd_github_signup";

/// A cookie scoped to `path`, lasting `max_age` seconds; Lax, since GitHub's
/// redirect back is a top-level GET from another site.
fn scoped_cookie(name: &str, value: &str, path: &str, max_age: i64) -> String {
    format!("{name}={value}; Path={path}; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age}")
}

/// How long the start and signup cookies last: as long as what they point to.
const START_SECONDS: i64 = github_accounts::START_MINUTES * 60;

/// The exact callback URL the environment's OAuth App names.
fn callback_url(state: &AppState) -> String {
    format!("{}{CALLBACK_PATH}", state.public_url)
}

fn github(state: &AppState) -> Result<&GithubLogin, AppError> {
    state.github_login.as_deref().ok_or(AppError::NotFound)
}

fn message(status: StatusCode, title: &str, text: &str, link: Option<(&str, &str)>) -> Response {
    page(status, &Message { title, text, link })
}

/// What a GitHub-only account can do when GitHub can't sign it in.
const WHEN_GITHUB_FAILS: &str = "If you signed up with GitHub, you can choose a password \
     at /reset instead; that ends every session, revokes every API token and unlinks GitHub, \
     which you can link again from your account page.";

/// Sends the browser to GitHub for `purpose`, keeping what the callback needs. The
/// account page's form starts a `Purpose::ChangeEmail` here, for an account with no
/// password to confirm a change of address with: nothing waits and nothing is
/// counted until GitHub's answer comes back.
pub(super) async fn start(
    state: &AppState,
    headers: &HeaderMap,
    ip: &ClientIp,
    purpose: Purpose,
    next: Option<&str>,
) -> Result<Response, AppError> {
    let github = github(state)?;
    // A browser keeps its secret across starts, so a second tab's start doesn't
    // strand the first's.
    let browser = cookie_value(headers, START_COOKIE).unwrap_or_else(crate::ids::new_secret);
    let Some(started) =
        github_accounts::start(&state.pool, &browser, &ip.0, &purpose, next).await?
    else {
        return Ok(message(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many sign-ins under way",
            "Too many GitHub sign-ins were started from here in the last few minutes. Wait ten minutes, then try again.",
            None,
        ));
    };
    let to = github.authorize_url(&callback_url(state), &started.state, &started.verifier);
    let cookie = scoped_cookie(START_COOKIE, &browser, "/", START_SECONDS);
    Ok(([(header::SET_COOKIE, cookie)], Redirect::to(&to)).into_response())
}

#[derive(Deserialize)]
pub struct StartForm {
    #[serde(default)]
    next: String,
}

/// `POST /login/github`: signs in, or up, with GitHub.
pub async fn sign_in(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    Form(form): Form<StartForm>,
) -> Result<Response, AppError> {
    let next = page_on_this_site(&form.next);
    start(&state, &headers, &ip, Purpose::SignIn, next).await
}

/// `POST /account/github/link`: links GitHub to the session's account.
pub async fn link(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    session: SessionAccount,
) -> Result<Response, AppError> {
    start(&state, &headers, &ip, Purpose::Link(session.id), None).await
}

/// Whether the request's session is `account_id`'s: a link or a change of address
/// finishes only under the session of the account that started it.
async fn in_session_of(
    state: &AppState,
    headers: &HeaderMap,
    account_id: &str,
) -> Result<bool, sqlx::Error> {
    let session = optional_session_account(headers, state).await?;
    Ok(session.as_deref() == Some(account_id))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /login/github/callback`: where GitHub sends the browser back.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, AppError> {
    let github = github(&state)?;
    let expired = || {
        message(
            StatusCode::BAD_REQUEST,
            "This sign-in didn't finish",
            "It expired, was used already, or was started in another browser. Start it again from this browser.",
            Some(("/login", "Log in")),
        )
    };
    match query.error.as_deref() {
        None => {}
        Some("access_denied") => {
            return Ok(message(
                StatusCode::OK,
                "GitHub sign-in cancelled",
                "Nothing changed here.",
                Some(("/login", "Log in")),
            ));
        }
        // Any other error is the OAuth App's setup. GitHub's codes are lower-case
        // words; anything else in the link is someone's, and isn't shown.
        Some(error) => {
            let code = if error
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            {
                error
            } else {
                "unknown"
            };
            tracing::warn!(code, "GitHub refused a sign-in");
            return Ok(message(
                StatusCode::BAD_GATEWAY,
                "GitHub refused the sign-in",
                &format!(
                    "GitHub answered {code}, which means File Drawer's GitHub app is set up wrongly; please tell {}. {WHEN_GITHUB_FAILS}",
                    crate::respond::CONTACT
                ),
                Some(("/login", "Log in")),
            ));
        }
    }
    let (Some(code), Some(github_state), Some(browser)) = (
        query.code,
        query.state,
        cookie_value(&headers, START_COOKIE),
    ) else {
        return Ok(expired());
    };
    let Some(started) = github_accounts::take_start(&state.pool, &github_state, &browser).await?
    else {
        return Ok(expired());
    };
    let identity = match github
        .oauth
        .identify(&code, &started.verifier, &callback_url(&state))
        .await
    {
        Ok(identity) => identity,
        Err(Failure::CodeRefused(_)) => return Ok(expired()),
        Err(Failure::NoEmailAccess) => {
            return Ok(message(
                StatusCode::FORBIDDEN,
                "GitHub didn't share your email addresses",
                "File Drawer asks GitHub only for your email addresses, to know which are verified. Sign in again and allow it.",
                Some(("/login", "Log in")),
            ));
        }
        Err(Failure::Unavailable(error)) => {
            tracing::warn!(%error, "GitHub sign-in failed");
            return Ok(message(
                StatusCode::BAD_GATEWAY,
                "GitHub didn't answer",
                &format!("Try again in a few minutes. {WHEN_GITHUB_FAILS}"),
                Some(("/login", "Log in")),
            ));
        }
    };
    match started.purpose {
        Purpose::Link(account_id) => {
            if !in_session_of(&state, &headers, &account_id).await? {
                return Ok(message(
                    StatusCode::FORBIDDEN,
                    "Log in to link GitHub",
                    "The session that started linking GitHub has ended. Log in, then link it again from your account page.",
                    Some(("/login", "Log in")),
                ));
            }
            match github_accounts::link(&state.pool, &account_id, &identity).await? {
                Linking::Done => Ok(Redirect::to("/account").into_response()),
                Linking::OtherAccount => Ok(message(
                    StatusCode::CONFLICT,
                    "That GitHub account is linked elsewhere",
                    "Another File Drawer account has it linked. Unlink it there first, or sign in with GitHub to reach that account.",
                    Some(("/account", "Your account")),
                )),
                Linking::HasAnother => Ok(message(
                    StatusCode::CONFLICT,
                    "This account has GitHub linked already",
                    "Unlink that GitHub account from your account page first.",
                    Some(("/account", "Your account")),
                )),
            }
        }
        Purpose::ChangeEmail {
            account_id,
            new_email,
        } => {
            if !in_session_of(&state, &headers, &account_id).await? {
                return Ok(message(
                    StatusCode::FORBIDDEN,
                    "Log in to change your email",
                    "The session that asked for the change has ended. Log in, then ask again from your account page.",
                    Some(("/login", "Log in")),
                ));
            }
            let linked = github_accounts::account_of(&state.pool, identity.github_id).await?;
            if linked.as_deref() != Some(account_id.as_str()) {
                return Ok(message(
                    StatusCode::FORBIDDEN,
                    "That isn't this account's GitHub account",
                    "Changing the email address takes the GitHub account linked to this File Drawer account. Nothing changed.",
                    Some(("/account", "Your account")),
                ));
            }
            // The form refused the account's own address; a change confirmed while
            // GitHub's page was open may have made this one its own since.
            let current = accounts::view(&state.pool, &account_id).await?.email;
            if current.as_deref() == Some(new_email.as_str()) {
                return Ok(Redirect::to("/account").into_response());
            }
            // Counted now that the request is known to be the account's: it sends
            // an email to the new address either way.
            limits::count_email(&state, Some(&ip), &new_email).await?;
            email_change::request(&state, &account_id, &new_email).await?;
            Ok(Redirect::to("/account").into_response())
        }
        Purpose::SignIn => {
            if let Some(account_id) =
                github_accounts::account_of(&state.pool, identity.github_id).await?
            {
                github_accounts::note_login(&state.pool, &identity).await?;
                let secret = accounts::new_session(&state.pool, &account_id).await?;
                let cookie = session_cookie(&secret, SESSION_DAYS * 24 * 60 * 60);
                let landing = Redirect::to(started.next.as_deref().unwrap_or("/account"));
                return Ok(([(header::SET_COOKIE, cookie)], landing).into_response());
            }
            if identity.emails.is_empty() {
                return Ok(message(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "No verified email address on GitHub",
                    "File Drawer needs an address it can write to, and GitHub lists none that is verified and gets mail (its noreply addresses don't). Verify one at github.com/settings/emails, or sign up with email and a password.",
                    Some(("/signup", "Sign up")),
                ));
            }
            let key = github_accounts::hold_signup(&state.pool, &identity, started.next.as_deref())
                .await?;
            let cookie = scoped_cookie(SIGNUP_COOKIE, &key, SIGNUP_PATH, START_SECONDS);
            Ok(([(header::SET_COOKIE, cookie)], Redirect::to(SIGNUP_PATH)).into_response())
        }
    }
}

/// The finish-signup form's path.
const SIGNUP_PATH: &str = "/signup/github";

fn signup_gone() -> Response {
    message(
        StatusCode::BAD_REQUEST,
        "This signup expired",
        "Signing up with GitHub must finish within ten minutes, in the browser that started it. Start again.",
        Some(("/signup", "Sign up")),
    )
}

/// `GET /signup/github`: the finish-signup form.
pub async fn signup_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    github(&state)?;
    let Some(key) = cookie_value(&headers, SIGNUP_COOKIE) else {
        return Ok(signup_gone());
    };
    let Some(pending) = github_accounts::pending(&state.pool, &key).await? else {
        return Ok(signup_gone());
    };
    let suggested = suggested_name(&pending.identity);
    Ok(page(
        StatusCode::OK,
        &GithubSignupPage {
            error: None,
            login: &pending.identity.login,
            display_name: &suggested,
            emails: &pending.identity.emails,
            chosen: 0,
            invite: "",
        },
    ))
}

/// A display name to suggest: GitHub's name cut to the display-name rules' length,
/// or the login when the name has none or the rules refuse it, such as for an
/// invisible character; the form shows it, and any refusal says why.
fn suggested_name(identity: &crate::github_login::Identity) -> String {
    let cut = |text: &str| -> String {
        text.trim()
            .chars()
            .take(accounts::DISPLAY_NAME_CHARS)
            .collect()
    };
    identity
        .name
        .as_deref()
        .map(cut)
        .filter(|name| accounts::display_name_refusal(name).is_ok())
        .unwrap_or_else(|| cut(&identity.login))
}

#[derive(Deserialize)]
pub struct SignupForm {
    display_name: String,
    /// Which of the listed addresses, by its place in the list.
    email: usize,
    #[serde(default)]
    invite: String,
}

const BAD_INVITE: &str = "That invite code can't be used: it may be mistyped, expired, revoked or used up. Leave it empty to sign up without one.";

/// `POST /signup/github`: makes the account, verified and with no password.
pub async fn signup(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    Form(form): Form<SignupForm>,
) -> Result<Response, AppError> {
    github(&state)?;
    let Some(key) = cookie_value(&headers, SIGNUP_COOKIE) else {
        return Ok(signup_gone());
    };
    let Some(pending) = github_accounts::pending(&state.pool, &key).await? else {
        return Ok(signup_gone());
    };
    let identity = &pending.identity;
    let invite = Some(form.invite.trim()).filter(|code| !code.is_empty());
    let chosen = form.email.min(identity.emails.len().saturating_sub(1));
    let again = |status: StatusCode, problem: &str| {
        page(
            status,
            &GithubSignupPage {
                error: Some(problem),
                login: &identity.login,
                display_name: form.display_name.trim(),
                emails: &identity.emails,
                chosen,
                invite: invite.unwrap_or(""),
            },
        )
    };
    let Some(email) = identity.emails.get(form.email) else {
        return Ok(again(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Choose one of the addresses listed.",
        ));
    };
    let display_name = match accounts::display_name_refusal(&form.display_name) {
        Ok(name) => name,
        Err(problem) => return Ok(again(StatusCode::UNPROCESSABLE_ENTITY, &problem)),
    };
    let limits = &state.rate_limits.limits;
    if let Some(code) = invite
        && invites::usable(&state.pool, code, now_utc())
            .await?
            .is_none()
    {
        // As at email signup, a wrong code counts against the IP's signups, so
        // codes can't be guessed at the request rate.
        let guess = [Counted::signups_from(limits, &ip)];
        limits::count_event(&state.pool, &guess, now_utc()).await?;
        return Ok(again(StatusCode::UNPROCESSABLE_ENTITY, BAD_INVITE));
    }
    // Every finish counts against the address's emails, taken or not, so the
    // inbox's holder can't be flooded with notes that it has an account. A young
    // GitHub account counts against the IP's signups too (design.md, Limits and
    // abuse), unless a code covers it.
    let mut counts = vec![Counted::emails_to(limits, email)];
    if invite.is_none() && github_accounts::young(identity, now_utc()) {
        counts.push(Counted::signups_from(limits, &ip));
    }
    limits::count_event(&state.pool, &counts, now_utc()).await?;
    let clear = scoped_cookie(SIGNUP_COOKIE, "", SIGNUP_PATH, 0);
    match github_accounts::finish_signup(&state.pool, &key, email, display_name, invite).await? {
        Finished::Created(account_id) => {
            let secret = accounts::new_session(&state.pool, &account_id).await?;
            let session = session_cookie(&secret, SESSION_DAYS * 24 * 60 * 60);
            let landing = Redirect::to(pending.next.as_deref().unwrap_or("/account"));
            // Appended, since an array of headers inserts, and the second cookie
            // would replace the first.
            let cookies =
                AppendHeaders([(header::SET_COOKIE, session), (header::SET_COOKIE, clear)]);
            Ok((cookies, landing).into_response())
        }
        Finished::Taken(account_id) => {
            // What email signup does: the address's holder learns of the attempt.
            mail_queue::queue(&state, &account_id, Queued::Taken).await?;
            let text = format!(
                "We sent a note to {email}. An account already uses that address: log in to it, then link GitHub from its account page."
            );
            let done = message(
                StatusCode::OK,
                "Check your email",
                &text,
                Some(("/login", "Log in")),
            );
            Ok(([(header::SET_COOKIE, clear)], done).into_response())
        }
        Finished::GithubTaken => Ok(message(
            StatusCode::CONFLICT,
            "That GitHub account has an account here",
            "It was linked while this form was open. Sign in with GitHub to reach it.",
            Some(("/login", "Log in")),
        )),
        Finished::BadInvite => Ok(again(StatusCode::UNPROCESSABLE_ENTITY, BAD_INVITE)),
        Finished::Gone => Ok(signup_gone()),
    }
}

/// `POST /account/github/unlink`: only while the account has a password, so the
/// person keeps a way in.
pub async fn unlink(
    State(state): State<AppState>,
    session: SessionAccount,
) -> Result<Response, AppError> {
    if !accounts::has_password(&state.pool, &session.id).await? {
        return Ok(message(
            StatusCode::CONFLICT,
            "GitHub is this account's only way in",
            "Choosing a password at /reset unlinks GitHub too, and ends every session and revokes every API token of the account; you can link GitHub again from your account page.",
            Some(("/reset", "Choose a password")),
        ));
    }
    github_accounts::unlink(&state.pool, &session.id).await?;
    Ok(Redirect::to("/account").into_response())
}
