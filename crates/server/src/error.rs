//! The one error type handlers return, and the status code each case maps to.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("send an API token as `Authorization: Bearer <token>`")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    /// The entry or request is refused, with every reason.
    #[error("refused:\n{}", .reasons.iter().map(|reason| format!("- {reason}")).collect::<Vec<_>>().join("\n"))]
    Refused {
        status: StatusCode,
        reasons: Vec<String>,
    },
    #[error("internal error")]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Refused { status, .. } => *status,
            AppError::Database(error) => {
                tracing::error!(%error, "database error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let mut response = (status, format!("{self}\n")).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}
