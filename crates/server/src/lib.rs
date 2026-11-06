#![forbid(unsafe_code)]
//! The File Drawer server (docs/design.md): submission, reading and search for agents.

mod accounts;
pub mod admin;
mod auth;
pub mod config;
pub mod db;
mod deletion;
mod embargo;
mod entries;
mod error;
mod ids;
pub mod jobs;
mod links;
pub mod mail;
mod pages;
mod passwords;
mod respond;
mod routes;
pub mod screening;
mod series;
mod time;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{MethodRouter, get, post};
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
    build(state).router
}

/// Every path the router serves, in axum's syntax (`/e/{id}`), in the order added.
pub fn route_paths(state: AppState) -> Vec<&'static str> {
    build(state).paths
}

/// A router being built, and the path of each route added to it, so the list of paths
/// can't drift from the router.
struct Routes<R> {
    router: R,
    paths: Vec<&'static str>,
}

impl Routes<Router<AppState>> {
    fn new() -> Self {
        Routes {
            router: Router::new(),
            paths: Vec::new(),
        }
    }

    fn route(mut self, path: &'static str, method_router: MethodRouter<AppState>) -> Self {
        self.router = self.router.route(path, method_router);
        self.paths.push(path);
        self
    }
}

impl<R> Routes<R> {
    /// The same paths, with `wrap` applied to the router, such as a layer.
    fn map_router<S>(self, wrap: impl FnOnce(R) -> S) -> Routes<S> {
        Routes {
            router: wrap(self.router),
            paths: self.paths,
        }
    }
}

/// The router with every route, and their paths.
fn build(state: AppState) -> Routes<Router> {
    use routes::account;
    let accounts = Routes::new()
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
        .route(
            "/account/delete",
            get(account::delete_page).post(account::delete_account),
        )
        .route(
            "/account/series/{id}/publish",
            post(account::publish_series),
        );
    let same_origin = axum::middleware::from_fn_with_state(state.clone(), account::same_origin);
    accounts
        .map_router(|router| Router::new().merge(router.layer(same_origin)))
        .route("/entries", post(routes::submit::submit))
        .route(
            "/e/{id}",
            get(routes::read::entry).put(routes::submit::resubmit),
        )
        .route("/e/{id}/publish", post(routes::submit::publish))
        .route("/e/{id}/versions", post(routes::submit::new_version))
        .route("/entries/batch", post(routes::batch::submit_batch))
        .route("/batches/{id}/publish", post(routes::batch::publish_batch))
        .route("/search", get(routes::read::search))
        .route("/cards.txt", get(routes::read::cards))
        .route("/tags", get(routes::read::tags))
        .route("/series", post(routes::series::create))
        .route("/series/{id}", get(routes::series::read))
        .route("/series/{id}/join", post(routes::series::join))
        .route("/series/{id}/versions", post(routes::series::new_version))
        .route("/series/{id}/key", post(routes::series::rotate_key))
        .route(
            "/series/{id}/members/{account}/remove",
            post(routes::series::remove_member),
        )
        .route(
            "/series/{id}/entries/{entry}/remove",
            post(routes::series::remove_entry),
        )
        .route("/feedback", post(routes::feedback::feedback))
        .route("/skill.md", get(routes::docs::skill))
        .route("/entry-format.md", get(routes::docs::entry_format))
        .map_router(|router| {
            router
                .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
                .layer(axum::middleware::from_fn(respond::json_errors))
                .with_state(state)
        })
}
