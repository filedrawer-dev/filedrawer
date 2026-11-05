#![forbid(unsafe_code)]
//! The File Drawer server (docs/design.md): submission, reading and search for agents.

pub mod admin;
mod auth;
pub mod config;
pub mod db;
mod embargo;
mod entries;
mod error;
mod ids;
mod respond;
mod routes;
pub mod screening;
mod time;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use sqlx::SqlitePool;

pub use screening::Screener;

/// The largest request body the server reads. Entries are capped at 100 KiB by the
/// analyzer, which refuses with a reason; this only stops a client from making the
/// server buffer more.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// What every request handler shares.
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub screener: Screener,
}

/// The router with every route.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/entries", post(routes::submit::submit))
        .route("/e/{id}", get(routes::read::entry))
        .route("/search", get(routes::read::search))
        .route("/cards.txt", get(routes::read::cards))
        .route("/tags", get(routes::read::tags))
        .route("/feedback", post(routes::feedback::feedback))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(axum::middleware::from_fn(respond::json_errors))
        .with_state(state)
}
