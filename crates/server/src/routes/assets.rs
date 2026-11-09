//! The pages' stylesheet and fonts, and the home page's script, compiled in, so no
//! page loads anything from another host (design.md, Pages for people). Courier Prime
//! is under the SIL Open Font License 1.1, served beside the fonts as
//! `/static/fonts/OFL.txt`.

use std::sync::LazyLock;

use axum::extract::Path;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::error::AppError;

const STYLESHEET: &str = include_str!("../../static/site.css");

/// The home page's copy button, the one script any page runs.
const COPY_SCRIPT: &str = include_str!("../../static/copy.js");

/// Each file under `/static/fonts/`, by the name it is served at. A font's name
/// carries its `@fontsource/courier-prime` version, so a new version gets a new URL.
const FONTS: &[(&str, &[u8])] = &[
    (
        "courier-prime-5.2.8-latin-400-normal.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-400-normal.woff2"),
    ),
    (
        "courier-prime-5.2.8-latin-400-italic.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-400-italic.woff2"),
    ),
    (
        "courier-prime-5.2.8-latin-700-normal.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-700-normal.woff2"),
    ),
    (
        "courier-prime-5.2.8-latin-700-italic.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-700-italic.woff2"),
    ),
    (
        "courier-prime-5.2.8-latin-ext-400-normal.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-ext-400-normal.woff2"),
    ),
    (
        "courier-prime-5.2.8-latin-ext-400-italic.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-ext-400-italic.woff2"),
    ),
    (
        "courier-prime-5.2.8-latin-ext-700-normal.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-ext-700-normal.woff2"),
    ),
    (
        "courier-prime-5.2.8-latin-ext-700-italic.woff2",
        include_bytes!("../../static/fonts/courier-prime-latin-ext-700-italic.woff2"),
    ),
    ("OFL.txt", include_bytes!("../../static/fonts/OFL.txt")),
];

/// A year: the files never change at a URL, since the stylesheet's and the script's
/// carry their hash and the fonts' names their version. The licence's never changes.
const FOREVER: &str = "public, max-age=31536000, immutable";

/// `path` with a hash of `text`, the file served there, so a new build's pages never
/// get an old copy from a browser's cache.
fn hashed_href(path: &str, text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let hash: String = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{path}?v={hash}")
}

static STYLESHEET_HREF: LazyLock<String> =
    LazyLock::new(|| hashed_href("/static/site.css", STYLESHEET));

static COPY_SCRIPT_HREF: LazyLock<String> =
    LazyLock::new(|| hashed_href("/static/copy.js", COPY_SCRIPT));

/// Where pages link the stylesheet.
pub fn stylesheet_href() -> &'static str {
    &STYLESHEET_HREF
}

/// Where the home page links its copy button's script.
pub fn copy_script_href() -> &'static str {
    &COPY_SCRIPT_HREF
}

pub async fn stylesheet() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, FOREVER),
        ],
        STYLESHEET,
    )
        .into_response()
}

pub async fn copy_script() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, FOREVER),
        ],
        COPY_SCRIPT,
    )
        .into_response()
}

pub async fn font(Path(name): Path<String>) -> Result<Response, AppError> {
    let (_, bytes) = FONTS
        .iter()
        .find(|(font, _)| *font == name)
        .ok_or(AppError::NotFound)?;
    let kind = if name.ends_with(".woff2") {
        "font/woff2"
    } else {
        "text/plain; charset=utf-8"
    };
    Ok((
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, FOREVER),
        ],
        *bytes,
    )
        .into_response())
}
