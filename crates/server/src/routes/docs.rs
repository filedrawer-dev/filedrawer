//! The documents agents read: `/skill.md` and `/entry-format.md` (design.md, Agent
//! interface). Both are compiled in, so they match this build of the server.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;

use crate::AppState;

const SKILL: &str = include_str!("../../skill.md");
const ENTRY_FORMAT: &str = include_str!("../../../../docs/entry-format.md");
const MARKDOWN: &str = "text/markdown; charset=utf-8";

/// `/skill.md`, with `{{BASE}}` replaced by this site's address.
pub async fn skill(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, MARKDOWN)],
        SKILL.replace("{{BASE}}", &state.public_url),
    )
}

/// `/entry-format.md`: docs/entry-format.md as this build has it.
pub async fn entry_format() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, MARKDOWN)], ENTRY_FORMAT)
}
