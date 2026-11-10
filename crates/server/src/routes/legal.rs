//! `GET /terms` and `GET /privacy` (design.md, Pages for people): each page's text is
//! one markdown file, compiled in, which a person's browser gets rendered and anything
//! else gets as it is, so the two can't drift.

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pulldown_cmark::{Parser, html};

use crate::pages::{LegalPage, page};
use crate::respond::wants_page;

const TERMS: &str = include_str!("../../legal/terms.md");
const PRIVACY: &str = include_str!("../../legal/privacy.md");

pub async fn terms(headers: HeaderMap) -> Response {
    legal(&headers, "Terms", TERMS)
}

pub async fn privacy(headers: HeaderMap) -> Response {
    legal(&headers, "Privacy", PRIVACY)
}

fn legal(headers: &HeaderMap, title: &str, markdown: &str) -> Response {
    if !wants_page(headers) {
        return (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            markdown.to_string(),
        )
            .into_response();
    }
    // The server's own text, so it renders as written: author markdown goes through
    // `markdown::to_html` instead, which demotes headings and drops `mailto:` links.
    let mut rendered = String::new();
    html::push_html(&mut rendered, Parser::new(markdown));
    page(
        StatusCode::OK,
        &LegalPage {
            title,
            html: &rendered,
        },
    )
}
