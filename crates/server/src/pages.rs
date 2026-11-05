//! The HTML pages for people (design.md, Pages for people), from askama templates
//! with escaping on.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::accounts::TokenView;

/// A rendered page with its status code.
pub fn page(status: StatusCode, template: &impl Template) -> Response {
    match template.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(error) => {
            tracing::error!(%error, "a template failed to render");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response()
        }
    }
}

/// A short page: a heading, a sentence and maybe a link.
#[derive(Template)]
#[template(path = "message.html")]
pub struct Message<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub link: Option<(&'a str, &'a str)>,
}

#[derive(Template, Default)]
#[template(path = "signup.html")]
pub struct SignupPage<'a> {
    pub error: Option<&'a str>,
    pub email: &'a str,
    pub display_name: &'a str,
}

#[derive(Template, Default)]
#[template(path = "login.html")]
pub struct LoginPage<'a> {
    pub error: Option<&'a str>,
    pub email: &'a str,
}

#[derive(Template)]
#[template(path = "verify.html")]
pub struct VerifyPage<'a> {
    pub token: &'a str,
}

#[derive(Template)]
#[template(path = "reset.html")]
pub struct ResetPage;

#[derive(Template)]
#[template(path = "reset_confirm.html")]
pub struct ResetConfirmPage<'a> {
    pub token: &'a str,
    pub error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "account.html")]
pub struct AccountPage<'a> {
    pub display_name: &'a str,
    pub account_id: &'a str,
    pub email: Option<&'a str>,
    pub verified: bool,
    pub tokens: &'a [TokenView],
    /// A token just made, shown this once.
    pub new_token: Option<&'a str>,
}
