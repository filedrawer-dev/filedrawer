#![forbid(unsafe_code)]
//! The File Drawer server (docs/design.md): submission, reading and search for agents.

mod accounts;
pub mod admin;
mod auth;
pub mod config;
pub mod db;
mod embargo;
mod entries;
mod error;
mod ids;
pub mod mail;
mod pages;
mod passwords;
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
    pub mailer: std::sync::Arc<dyn mail::Mailer>,
    /// The site's address, with no trailing slash (`Config::public_url`).
    pub public_url: String,
}

/// The router with every route.
pub fn app(state: AppState) -> Router {
    use routes::account;
    let accounts = Router::new()
        .route("/signup", get(account::signup_page).post(account::signup))
        .route("/verify", get(account::verify_page).post(account::verify))
        .route("/login", get(account::login_page).post(account::login))
        .route("/logout", post(account::logout))
        .route("/reset", get(account::reset_page).post(account::reset))
        .route(
            "/reset/confirm",
            get(account::reset_confirm_page).post(account::reset_confirm),
        )
        .route("/account", get(account::account))
        .route("/account/tokens", post(account::create_token))
        .route("/account/tokens/{id}/revoke", post(account::revoke_token))
        .route("/account/verify-email", post(account::resend_verification))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            account::same_origin,
        ));
    Router::new()
        .merge(accounts)
        .route("/entries", post(routes::submit::submit))
        .route("/e/{id}", get(routes::read::entry))
        .route("/search", get(routes::read::search))
        .route("/cards.txt", get(routes::read::cards))
        .route("/tags", get(routes::read::tags))
        .route("/feedback", post(routes::feedback::feedback))
        .route("/skill.md", get(routes::docs::skill))
        .route("/entry-format.md", get(routes::docs::entry_format))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(axum::middleware::from_fn(respond::json_errors))
        .with_state(state)
}
