#![forbid(unsafe_code)]
//! The File Drawer server (docs/design.md): submission, reading and search for agents.

mod accounts;
pub mod admin;
mod allowance;
mod attachments;
mod auth;
pub mod config;
pub mod db;
mod deletion;
mod embargo;
mod entries;
mod error;
pub mod file_store;
mod ids;
mod invites;
pub mod jobs;
pub mod limits;
pub mod link_checks;
mod links;
pub mod mail;
mod markdown;
mod pages;
mod passwords;
mod queue;
mod removed_entries;
mod removed_files;
mod removed_series;
mod respond;
mod review;
mod routes;
pub mod screening;
mod series;
mod skills;
mod suspension;
mod time;
pub mod unpack;
mod warnings;
mod withdraw;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{MethodRouter, delete, get, post};
use sqlx::SqlitePool;

pub use attachments::StoreError;
pub use embargo::Embargoes;
pub use screening::Screener;

/// The largest request body the server reads. Entries are capped at 100 KiB by the
/// analyzer, which refuses with a reason; this only stops a client from making the
/// server buffer more.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The largest `POST /attachments` body: 1 MiB of files, which JSON escaping can
/// more than double, so the set's own 1 MiB cap is what refuses with a reason.
const MAX_ATTACHMENT_BODY_BYTES: usize = 4 * 1024 * 1024;

/// What every request handler shares.
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub screener: Screener,
    /// The most screening may spend in a UTC day, in billionths of a dollar.
    pub screening_daily_nano_usd: i64,
    /// Held while one item is screened, so no two screenings run at once.
    pub screening_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub mailer: std::sync::Arc<dyn mail::Mailer>,
    /// The site's address, with no trailing slash (`Config::public_url`).
    pub public_url: String,
    /// Staging's smoke-test account (`Config::smoke_account`).
    pub smoke_account: Option<String>,
    /// Staging's screen-test account (`Config::screen_test_account`).
    pub screen_test_account: Option<String>,
    /// Staging's CI screen account (`Config::ci_screen_account`).
    pub ci_screen_account: Option<String>,
    /// Where attached files are served (`Config::files_url`).
    pub files_url: Option<String>,
    /// Attached files' bytes.
    pub files: std::sync::Arc<file_store::FileStore>,
    /// The most bytes of attached files the site stores in all
    /// (`Config::attachment_store_cap_bytes`).
    pub attachment_store_cap_bytes: i64,
    /// Uploads are refused when they would leave the disk with fewer bytes free.
    pub attachment_disk_floor_bytes: u64,
    /// The rate limits and their per-minute counts.
    pub rate_limits: std::sync::Arc<limits::RateLimits>,
    /// Which deployment this is (`Config::environment`).
    pub environment: config::Environment,
    /// What the last screening calls came to, for `GET /staging/status`.
    pub screening_health: std::sync::Arc<screening::ScreeningHealth>,
    /// Fetches `code` and `logs` links, and asks GitHub about commits.
    pub link_checker: std::sync::Arc<link_checks::LinkChecker>,
    /// Whether embargoes and hidden series are taken: `Paused` everywhere but tests.
    pub embargoes: Embargoes,
}

/// The free disk an upload must leave (design.md, Attached code: 5 GB).
pub const ATTACHMENT_DISK_FLOOR_BYTES: u64 = 5_000_000_000;

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

    /// `route` when `wanted`, so a route production mustn't serve isn't there at all.
    fn route_if(
        self,
        wanted: bool,
        path: &'static str,
        method_router: MethodRouter<AppState>,
    ) -> Self {
        if wanted {
            self.route(path, method_router)
        } else {
            self
        }
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

/// What a path no route serves answers: a 404 with a body, which a browser sees as a
/// page (axum's own has none).
async fn not_found() -> error::AppError {
    error::AppError::NotFound
}

/// The router with every route, and their paths. A new route is named in `/skill.md`
/// or a skill, or listed in `NOT_FOR_AGENTS` in `tests/it/skill.rs`, as
/// CODING_STANDARDS.md (Untrusted text) says which; a test refuses one that is neither.
fn build(state: AppState) -> Routes<Router> {
    use routes::{account, admin};
    // A build whose skills lack a version stops here, at start, not on a request.
    skills::versions(", ");
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
        )
        .route(
            "/account/e/{id}/withdraw",
            get(routes::withdraw::confirm_page).post(routes::withdraw::withdraw),
        )
        .route("/admin", get(admin::queue))
        .route(
            "/admin/invites",
            get(admin::invites).post(admin::create_invite),
        )
        .route("/admin/invites/{code}/revoke", post(admin::revoke_invite))
        .route("/admin/e/{id}/release", post(admin::release))
        .route("/admin/e/{id}", get(admin::entry))
        .route("/admin/e/{id}/publish", post(admin::publish))
        .route("/admin/e/{id}/hold", post(admin::hold))
        .route("/admin/e/{id}/reject", post(admin::reject))
        .route("/admin/e/{id}/reject-chain", post(admin::reject_chain))
        .route("/admin/e/{id}/doubt-seen", post(admin::doubt_seen))
        .route("/admin/e/{id}/remove", post(admin::remove_entry))
        .route("/admin/e/{id}/restore", post(admin::restore_entry))
        .route(
            "/admin/series-refusals/{id}/confirm",
            post(admin::confirm_refusal),
        )
        .route(
            "/admin/series-refusals/{id}/dismiss",
            post(admin::dismiss_refusal),
        )
        .route("/admin/series/{id}", get(admin::series))
        .route("/admin/series/{id}/remove", post(admin::remove_series))
        .route("/admin/series/{id}/restore", post(admin::restore_series))
        .route("/admin/accounts/{id}/clear-guard", post(admin::clear_guard))
        .route("/admin/accounts/{id}", get(admin::account))
        .route("/admin/accounts/{id}/suspend", post(admin::suspend))
        .route("/admin/accounts/{id}/lift", post(admin::lift))
        .route("/admin/f/{sha256}", get(admin::file))
        .route("/admin/f/{sha256}/remove", post(admin::remove_file))
        .route(
            "/admin/f/{sha256}/remove-hazard",
            post(admin::remove_hazard),
        );
    let same_origin = axum::middleware::from_fn_with_state(state.clone(), account::same_origin);
    accounts
        .map_router(|router| Router::new().merge(router.layer(same_origin)))
        .route("/", get(routes::home::home))
        .route("/beta", get(routes::beta::beta))
        .route("/terms", get(routes::legal::terms))
        .route("/privacy", get(routes::legal::privacy))
        .route("/static/site.css", get(routes::assets::stylesheet))
        .route("/static/copy.js", get(routes::assets::copy_script))
        .route("/static/fonts/{name}", get(routes::assets::font))
        .route("/entries", post(routes::submit::submit))
        .route(
            "/e/{id}",
            get(routes::read::entry)
                .put(routes::submit::resubmit)
                .delete(routes::delete::entry),
        )
        .route("/e/{id}/publish", post(routes::submit::publish))
        .route("/e/{id}/versions", post(routes::submit::new_version))
        .route("/entries/batch", post(routes::batch::submit_batch))
        .route("/entries/mine", get(routes::mine::list))
        .route(
            "/attachments",
            post(routes::attachments::upload)
                .layer(DefaultBodyLimit::max(MAX_ATTACHMENT_BODY_BYTES)),
        )
        .route("/batches/{id}", delete(routes::delete::batch))
        .route("/batches/{id}/publish", post(routes::batch::publish_batch))
        .route("/search", get(routes::read::search))
        .route("/cards.txt", get(routes::read::cards))
        .route("/tags", get(routes::read::tags))
        .route_if(
            state.environment != config::Environment::Production,
            "/staging/status",
            get(routes::status::status),
        )
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
        .route("/skills/{name}/{*path}", get(routes::docs::skill_file))
        .route("/entry-format.md", get(routes::docs::entry_format))
        .map_router(|router| {
            let files_host =
                axum::middleware::from_fn_with_state(state.clone(), routes::files::files_host);
            let per_ip = axum::middleware::from_fn_with_state(state.clone(), limits::per_ip);
            router
                .fallback(not_found)
                .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
                .layer(axum::middleware::from_fn(respond::negotiate))
                .layer(files_host)
                .layer(per_ip)
                .with_state(state)
        })
}
