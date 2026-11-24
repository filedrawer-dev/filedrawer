//! Claiming an account an agent made (design.md, Accounts; #130): `POST
//! /account/claim`, which the agent sends with its token, and `/claim`, the page
//! behind the link the named address gets, whose button's POST acts, since mail
//! scanners follow links.

use axum::Form;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use super::account::session_cookie;
use crate::accounts::{self, EMAIL_LINK_HOURS, SESSION_DAYS};
use crate::auth::TokenAccount;
use crate::claims::{self, Refused};
use crate::error::AppError;
use crate::limits::{self, ClientIp, Counted};
use crate::pages::{ClaimPage, Message, page};
use crate::time::now_utc;
use crate::{AppState, passwords};

#[derive(Deserialize)]
pub struct AskForm {
    email: String,
}

/// `POST /account/claim`: the one account setting a token may change, and only
/// while the account has no owner. Answers the same whether or not the address has
/// an account.
pub async fn ask(
    State(state): State<AppState>,
    account: TokenAccount,
    ip: ClientIp,
    Form(form): Form<AskForm>,
) -> Result<Response, AppError> {
    if !accounts::is_unclaimed(&state.pool, &account.id).await? {
        return Err(has_owner());
    }
    let email = accounts::normalize_email(&form.email);
    if !accounts::looks_like_email(&email) {
        return Err(AppError::Refused {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            reasons: vec!["`email` isn't an email address".into()],
        });
    }
    let limits = &state.rate_limits.limits;
    let counts = [
        Counted::claims_of(limits, &account.id),
        Counted::claims(limits),
        Counted::emails_to(limits, &email),
        Counted::emails_from(limits, &ip),
    ];
    limits::count_event(&state.pool, &counts, now_utc()).await?;
    let Some(asked) = claims::request(&state, &account.id, &email).await? else {
        return Err(has_owner());
    };
    let no_store = (header::CACHE_CONTROL, "no-store");
    let body = format!(
        "{}code: {}\nexpires_at: {}\n\
         A link is on its way to that address. Whoever opens it is asked for the code, \
         so show the code to your human, and to nobody else.\n",
        account.line(),
        asked.code,
        asked.expires_at
    );
    Ok(([no_store], body).into_response())
}

/// The refusal for a claim asked of an account that has an owner.
fn has_owner() -> AppError {
    AppError::Conflict(
        "this account has an owner already, who manages it from the account page".into(),
    )
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: String,
}

fn dead_link() -> Response {
    let text = format!(
        "It was used, replaced or expired, or its code was mistyped too often. A claim's \
         link works once, for {EMAIL_LINK_HOURS} hours. Ask your agent to claim the account \
         again for a new link and code."
    );
    page(
        StatusCode::BAD_REQUEST,
        &Message {
            title: "This link doesn't work",
            text: &text,
            link: None,
        },
    )
}

fn taken() -> Response {
    page(
        StatusCode::CONFLICT,
        &Message {
            title: "This address has an account",
            text: "An account an agent made can't be claimed with an address that already \
                   has a File Drawer account. Log in to that account and make a token for \
                   your agent on the account page.",
            link: Some(("/login", "Log in")),
        },
    )
}

pub async fn claim_page(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, AppError> {
    let Some(pending) = claims::by_link(&state.pool, &query.token).await? else {
        return Ok(dead_link());
    };
    let template = ClaimPage {
        token: &query.token,
        account_id: &pending.account_id,
        entries: pending.entries,
        display_name: "",
        error: None,
    };
    Ok(page(StatusCode::OK, &template))
}

#[derive(Deserialize)]
pub struct ClaimForm {
    token: String,
    code: String,
    display_name: String,
    password: String,
}

pub async fn claim(
    State(state): State<AppState>,
    Form(form): Form<ClaimForm>,
) -> Result<Response, AppError> {
    let Some(pending) = claims::by_link(&state.pool, &form.token).await? else {
        return Ok(dead_link());
    };
    let again = |problem: &str| {
        let template = ClaimPage {
            token: &form.token,
            account_id: &pending.account_id,
            entries: pending.entries,
            display_name: form.display_name.trim(),
            error: Some(problem),
        };
        page(StatusCode::UNPROCESSABLE_ENTITY, &template)
    };
    let display_name = match accounts::display_name_refusal(&form.display_name) {
        Ok(name) => name,
        Err(problem) => return Ok(again(&problem)),
    };
    if let Some(problem) = passwords::problem(&form.password) {
        return Ok(again(&problem));
    }
    // The code before the hash, so a guess costs none.
    match claims::check(&state.pool, &form.token, &form.code).await? {
        Ok(()) => {}
        Err(Refused::Dead) => return Ok(dead_link()),
        Err(Refused::Taken) => return Ok(taken()),
        Err(Refused::Wrong) => {
            return Ok(again(
                "That isn't the code your agent was given. Check it and try again; after \
                 a few wrong codes the link stops working.",
            ));
        }
    }
    let place = state.passwords.place().await?;
    let hash = place.hash(form.password.clone()).await;
    // Checked again with the write, so a claim that ended while the hash ran stays
    // ended.
    let today = now_utc().date();
    let claimed = claims::confirm(
        &state.pool,
        &form.token,
        &form.code,
        display_name,
        &hash,
        today,
    )
    .await?;
    let account_id = match claimed {
        Ok(account_id) => account_id,
        Err(Refused::Taken) => return Ok(taken()),
        // The code was right a moment ago, so a wrong one now means the claim was
        // replaced while the hash ran.
        Err(Refused::Dead | Refused::Wrong) => return Ok(dead_link()),
    };
    let secret = accounts::new_session(&state.pool, &account_id).await?;
    let cookie = session_cookie(&secret, SESSION_DAYS * 24 * 60 * 60);
    Ok(([(header::SET_COOKIE, cookie)], Redirect::to("/account")).into_response())
}
