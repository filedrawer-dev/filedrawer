//! The one error type handlers return, and the status code each case maps to. A
//! message or reason can quote text from outside (an entry's field, a path, a URL
//! segment), so each goes through `one_line` as the body is written, and none can
//! pass for one of the server's fields (CODING_STANDARDS.md, Untrusted text).

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::respond::one_line;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("send an API token as `Authorization: Bearer <token>`")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    /// Nothing the token's account may see, saying which account that is, so an
    /// agent holding another account's token can tell.
    #[error("not found: {}", one_line(.0))]
    NotFoundFor(String),
    /// The account deleted it; the message says when.
    #[error("{}", one_line(.0))]
    Gone(String),
    #[error("{}", one_line(.0))]
    Forbidden(String),
    #[error("{}", one_line(.0))]
    Conflict(String),
    #[error("{}", one_line(.0))]
    BadRequest(String),
    /// The entry or request is refused, with every reason.
    #[error("refused:\n{}", .reasons.iter().map(|reason| format!("- {}", one_line(reason))).collect::<Vec<_>>().join("\n"))]
    Refused {
        status: StatusCode,
        reasons: Vec<String>,
    },
    /// Screening failed what was sent. The reasons, which a model wrote from the
    /// sender's text, follow the author-text delimiter, one line each.
    #[error(
        "error: the screen failed it\n{}\n{}",
        crate::respond::DELIMITER,
        .0.iter().map(|reason| format!("reason: {}", one_line(reason))).collect::<Vec<_>>().join("\n")
    )]
    ScreenRejected(Vec<String>),
    /// A rate limit reached (design.md, Limits and abuse).
    #[error("too many requests: {what} reached its limit; try again in {retry_after_secs} s")]
    TooManyRequests { what: String, retry_after_secs: u64 },
    #[error("internal error")]
    Database(#[from] sqlx::Error),
    /// Reading or writing a file failed, such as an attached file's bytes.
    #[error("internal error")]
    Io(#[from] std::io::Error),
}

impl From<crate::attachments::StoreError> for AppError {
    fn from(error: crate::attachments::StoreError) -> Self {
        match error {
            crate::attachments::StoreError::Database(error) => AppError::Database(error),
            crate::attachments::StoreError::Files(error) => AppError::Io(error),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::NotFound | AppError::NotFoundFor(_) => StatusCode::NOT_FOUND,
            AppError::Gone(_) => StatusCode::GONE,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Refused { status, .. } => *status,
            AppError::ScreenRejected(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            AppError::Database(error) => {
                tracing::error!(%error, "database error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppError::Io(error) => {
                tracing::error!(%error, "I/O error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let mut response = (status, format!("{self}\n")).into_response();
        if let AppError::TooManyRequests {
            retry_after_secs, ..
        } = &self
        {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                header::HeaderValue::from(*retry_after_secs),
            );
        }
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}
