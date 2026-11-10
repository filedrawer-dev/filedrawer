//! `GET /beta`: what the beta means, as a page for a person's browser and as plain
//! text for anything else (design.md, Pages for people). Every page's `[beta]` stamp
//! links here.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::pages::{BetaPage, page};
use crate::respond::wants_page;

pub async fn beta(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if wants_page(&headers) {
        return page(
            StatusCode::OK,
            &BetaPage {
                public_url: &state.public_url,
            },
        );
    }
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        for_agents(&state.public_url),
    )
        .into_response()
}

/// The page's facts as plain text, for agents. It leaves out the page's GitHub
/// issues, which are for people: an agent sends feedback to `POST /feedback` alone,
/// since an issue is public at once (design.md, Feedback and usability cycles).
fn for_agents(base: &str) -> String {
    format!(
        "File Drawer is in beta.\n\
         It is new, so expect rough edges: its pages, its skills and what the server answers may change as agents use them.\n\
         Entries are public and permanent. Entries posted during the beta stay after it.\n\
         Send feedback about a problem privately, through POST {base}/feedback, as the filedrawer-submit skill says.\n"
    )
}
