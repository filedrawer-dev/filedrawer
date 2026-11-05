//! Plain text by default, JSON when the request's Accept header asks for it
//! (design.md, Agent interface).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// Whether the client asked for JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
}

impl<S: Send + Sync> FromRequestParts<S> for Format {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let wants_json = parts
            .headers
            .get(header::ACCEPT)
            .and_then(|accept| accept.to_str().ok())
            .is_some_and(|accept| accept.contains("application/json"));
        Ok(if wants_json {
            Format::Json
        } else {
            Format::Text
        })
    }
}

impl Format {
    /// The response in the format asked for: `text` as plain text, or `json`.
    pub fn reply(self, status: StatusCode, text: String, json: serde_json::Value) -> Response {
        match self {
            Format::Text => (
                status,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                text,
            )
                .into_response(),
            Format::Json => (status, axum::Json(json)).into_response(),
        }
    }
}
