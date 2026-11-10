//! The documents agents read: `/skill.md`, the installer, the files of the skills it
//! installs, and `/entry-format.md` (design.md, Agent interface and Skills). All are
//! compiled in, so they match this build of the server.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;

use crate::AppState;
use crate::error::AppError;
use crate::skills::{self, Skill};

const INSTALL: &str = include_str!("../../install.md");
const ENTRY_FORMAT: &str = include_str!("../../../../docs/entry-format.md");
const MARKDOWN: &str = "text/markdown; charset=utf-8";

/// `/skill.md`: the installer, rendered for this site, with each skill's version in
/// `X-Filedrawer-Skills`, which the installed skills read from a HEAD of this route
/// alone, and the commit the server was built from in `X-Filedrawer-Commit`, which
/// ops/deploy-production reads.
pub async fn skill(State(state): State<AppState>) -> impl IntoResponse {
    let text = skills::render(INSTALL, &state.public_url)
        .replace("{{SKILLS}}", &skills::versions(" "))
        .replace("{{TAG_URLS}}", &skills::tag_urls());
    (
        [
            (header::CONTENT_TYPE, MARKDOWN.to_string()),
            (
                header::HeaderName::from_static(skills::VERSIONS_HEADER),
                skills::versions(", "),
            ),
            (
                header::HeaderName::from_static("x-filedrawer-commit"),
                crate::COMMIT.to_string(),
            ),
        ],
        text,
    )
}

/// `/skills/<name>/<path>`: a file of a skill's folder, rendered for this site, for an
/// agent that reads the skill without installing it.
pub async fn skill_file(
    State(state): State<AppState>,
    Path((name, path)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let text = Skill::named(&name)
        .and_then(|skill| skill.file(&path))
        .ok_or(AppError::NotFound)?;
    Ok((
        [(header::CONTENT_TYPE, MARKDOWN)],
        skills::render(text, &state.public_url),
    ))
}

/// `/entry-format.md`: docs/entry-format.md as this build has it.
pub async fn entry_format() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, MARKDOWN)], ENTRY_FORMAT)
}
