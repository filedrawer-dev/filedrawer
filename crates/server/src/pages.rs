//! The HTML pages for people (design.md, Pages for people), from askama templates
//! with escaping on.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::accounts::{Guarded, TokenView};
use crate::entries::Undecided;
use crate::series::Owned;

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
#[template(path = "delete.html")]
pub struct DeletePage<'a> {
    pub error: Option<&'a str>,
    pub unpublished: i64,
    pub public: i64,
    /// Hidden series that stop the deletion.
    pub blocking: &'a [String],
}

#[derive(Template)]
#[template(path = "account.html")]
pub struct AccountPage<'a> {
    pub display_name: &'a str,
    pub account_id: &'a str,
    pub email: Option<&'a str>,
    pub verified: bool,
    pub tokens: &'a [TokenView],
    /// The series this account made.
    pub series: &'a [Owned],
    /// A token just made, shown this once.
    pub new_token: Option<&'a str>,
}

/// An entry on the admin queue page, with the title read from its text.
pub struct QueueRow<'a> {
    pub entry: &'a Undecided,
    pub title: String,
}

#[derive(Template)]
#[template(path = "admin.html")]
pub struct AdminQueuePage<'a> {
    /// Each heading, with its entries: those in review, then those held.
    pub sections: [(&'a str, Vec<&'a QueueRow<'a>>); 2],
    pub guarded: &'a [Guarded],
}

#[derive(Template)]
#[template(path = "admin_entry.html")]
pub struct AdminEntryPage<'a> {
    pub title: &'a str,
    pub entry: &'a Undecided,
    pub card: &'a str,
}
