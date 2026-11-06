//! The files host (design.md, Attached code): attached files, served as plain text
//! from a host of their own that carries no cookies, so an attached HTML file can't
//! run as a page on the site. The host answers `GET /f/<sha256>` and nothing else.

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::{AppState, attachments, file_store};

/// Middleware in front of every route: a request addressed to the files host is
/// answered here, and never reaches the site's routes.
pub async fn files_host(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(files_host) = state.files_url.as_deref().map(host_of) else {
        return next.run(request).await;
    };
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    // However the client wrote the host (case, a port, a trailing dot), the files
    // host never reaches the site's routes.
    if bare_host(host) != bare_host(files_host) {
        return next.run(request).await;
    }
    let sha = request
        .uri()
        .path()
        .strip_prefix("/f/")
        .map(str::to_ascii_lowercase)
        .filter(|sha| file_store::is_sha256(sha));
    let served = match sha {
        Some(sha) if request.method() == axum::http::Method::GET => public_text(&state, &sha).await,
        _ => Ok(None),
    };
    match served {
        Ok(Some(text)) => {
            let mut response = (StatusCode::OK, text).into_response();
            let headers = response.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            headers.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            response
        }
        Ok(None) => (StatusCode::NOT_FOUND, "not found\n").into_response(),
        Err(error) => {
            tracing::error!(%error, "serving an attached file failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response()
        }
    }
}

/// The text of file `sha256`, if a public entry's set holds it. A file whose record
/// is there but whose bytes aren't is an error, not a missing file.
async fn public_text(state: &AppState, sha256: &str) -> Result<Option<String>, ServeError> {
    if !attachments::is_public_file(&state.pool, sha256).await? {
        return Ok(None);
    }
    match state.files.read(sha256).await? {
        Some(text) => Ok(Some(text)),
        None => Err(ServeError::Missing(sha256.to_string())),
    }
}

#[derive(Debug, thiserror::Error)]
enum ServeError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Files(#[from] std::io::Error),
    #[error("public file {0} has a record but no bytes on disk")]
    Missing(String),
}

/// The host, with any port, of a URL such as `https://files.filedrawer.dev`.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// A host as names compare: lower case, without a port or a trailing dot.
fn bare_host(host: &str) -> String {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    };
    name.trim_end_matches('.').to_ascii_lowercase()
}
