#![forbid(unsafe_code)]
//! The File Drawer server (docs/design.md): submission, reading and search for agents.

/// The commit this binary was built from, or `unknown` (build.rs).
pub const COMMIT: &str = env!("FILEDRAWER_BUILD_COMMIT");

/// How `-h` names the commit. The words sit together in the file, so
/// ops/deploy-production finds a binary's commit without running it.
pub const COMMIT_LINE: &str = concat!("filedrawer-commit ", env!("FILEDRAWER_BUILD_COMMIT"));

mod accounts;
pub mod admin;
mod allowance;
mod attachments;
mod auth;
pub mod claims;
pub mod config;
pub mod daily_email;
pub mod db;
mod deletion;
mod duplicates;
pub mod email_change;
mod entries;
mod error;
pub mod event_log;
pub mod file_store;
pub mod github_accounts;
pub mod github_login;
mod ids;
mod invites;
pub mod jobs;
pub mod limits;
pub mod link_checks;
mod links;
pub mod mail;
pub mod mail_queue;
mod markdown;
mod notices;
pub mod outbound;
mod pages;
pub mod passwords;
mod provenance;
pub mod purge;
mod queue;
pub mod reads;
mod removed_entries;
mod removed_files;
mod removed_series;
pub mod request_log;
mod respond;
mod review;
mod routes;
pub mod screening;
pub mod search;
pub mod search_log;
mod series;
pub mod serve;
mod skills;
mod suspension;
mod time;
mod unknown_fields;
pub mod unpack;
mod warnings;
mod withdraw;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{MethodRouter, delete, get, post};
use sqlx::SqlitePool;

pub use attachments::StoreError;
pub use screening::Screener;

/// The largest request body the server reads. Entries are capped at 100 KiB by the
/// analyzer, which refuses with a reason; this only stops a client from making the
/// server buffer more.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The largest `POST /attachments` body: 1 MiB of files, which JSON escaping can
/// more than double, so the set's own 1 MiB cap is what refuses with a reason.
const MAX_ATTACHMENT_BODY_BYTES: usize = 4 * 1024 * 1024;

/// The largest body the forms of signup, verification, login and password reset
/// send: the longest, a signup, holds a password of at most 1,024 characters, which
/// percent-encoding makes at most 12 KiB, beside an email address and a name.
const MAX_FORM_BYTES: usize = 64 * 1024;

/// `method_router` reading bodies of at most `limit` bytes ([`declared_within`]).
fn limited(method_router: MethodRouter<AppState>, limit: usize) -> MethodRouter<AppState> {
    method_router
        .layer(DefaultBodyLimit::max(limit))
        .layer(axum::middleware::from_fn_with_state(limit, declared_within))
}

/// Middleware: a request whose `Content-Length` is past `limit`, the most its route
/// reads, is refused before any of its body is read, and its connection closes.
/// So a body the server waits for is one its route takes, and the time it may take
/// to arrive (`serve::Bounds`) follows from that route's limit. A body with no
/// declared length is cut at the limit as it arrives.
async fn declared_within(
    axum::extract::State(limit): axum::extract::State<usize>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{HeaderValue, StatusCode, header};
    use axum::response::IntoResponse;
    match serve::declared_length(request.headers()) {
        Some(bytes) if bytes > u64::try_from(limit).unwrap_or(u64::MAX) => {
            let mut refusal = error::AppError::Refused {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                reasons: vec![format!(
                    "the body declares {bytes} bytes, and this route takes at most {limit}"
                )],
            }
            .into_response();
            refusal
                .headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("close"));
            refusal
        }
        _ => next.run(request).await,
    }
}

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
    /// The outgoing mail's sender: what wakes it and how its sends went.
    pub mail_queue: std::sync::Arc<mail_queue::MailQueue>,
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
    /// Where the daily email's job reports each run (`Config::daily_email_ping`).
    pub pinger: Option<std::sync::Arc<dyn daily_email::Pinger>>,
    /// The nightly copies of the database (`Config::backups_dir`).
    pub backups_dir: std::path::PathBuf,
    /// The log of the owner's purges (`Config::purge_log`).
    pub purge_log: std::path::PathBuf,
    /// When the server started, since the events the daily email counts are kept in
    /// memory.
    pub started_at: ::time::OffsetDateTime,
    /// Staging's record of the headers requests sent (`request_log`); none elsewhere.
    pub request_log: Option<std::sync::Arc<request_log::RequestLog>>,
    /// Runs every password hash and check, a few at a time.
    pub passwords: std::sync::Arc<passwords::Hasher>,
    /// Searches that look words up in the index, a few at a time.
    pub searches: std::sync::Arc<search::Places>,
    /// Today's readers of each entry, for Most read, and the salt they're hashed with.
    pub reads: std::sync::Arc<reads::ReadCounter>,
    /// GitHub sign-in, when this environment has an OAuth App
    /// (`Config::github_oauth`); without one its routes aren't served.
    pub github_login: Option<std::sync::Arc<github_login::GithubLogin>>,
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
        .route(
            "/signup",
            limited(
                get(account::signup_page).post(account::signup),
                MAX_FORM_BYTES,
            ),
        )
        .route(
            "/verify",
            limited(
                get(account::verify_page).post(account::verify),
                MAX_FORM_BYTES,
            ),
        )
        .route(
            "/login",
            limited(
                get(account::login_page).post(account::login),
                MAX_FORM_BYTES,
            ),
        )
        .route_if(
            state.github_login.is_some(),
            "/login/github",
            limited(post(routes::github::sign_in), MAX_FORM_BYTES),
        )
        .route_if(
            state.github_login.is_some(),
            github_login::CALLBACK_PATH,
            get(routes::github::callback),
        )
        .route_if(
            state.github_login.is_some(),
            "/signup/github",
            limited(
                get(routes::github::signup_page).post(routes::github::signup),
                MAX_FORM_BYTES,
            ),
        )
        .route_if(
            state.github_login.is_some(),
            "/account/github/link",
            post(routes::github::link),
        )
        .route_if(
            state.github_login.is_some(),
            "/account/github/unlink",
            post(routes::github::unlink),
        )
        .route("/logout", post(account::logout))
        .route(
            "/reset",
            limited(
                get(account::reset_page).post(account::reset),
                MAX_FORM_BYTES,
            ),
        )
        .route(
            "/reset/confirm",
            limited(
                get(account::reset_confirm_page).post(account::reset_confirm),
                MAX_FORM_BYTES,
            ),
        )
        .route(
            "/claim",
            limited(
                get(routes::claim::claim_page).post(routes::claim::claim),
                MAX_FORM_BYTES,
            ),
        )
        .route("/account", get(account::account))
        .route("/account/tokens", post(account::create_token))
        .route("/account/tokens/{id}/revoke", post(account::revoke_token))
        .route("/account/verify-email", post(account::resend_verification))
        .route(
            "/account/email",
            limited(post(account::change_email), MAX_FORM_BYTES),
        )
        .route("/account/email/cancel", post(account::cancel_email_change))
        .route(
            "/account/email/confirm",
            limited(
                get(account::confirm_email_page).post(account::confirm_email),
                MAX_FORM_BYTES,
            ),
        )
        .route(
            "/account/delete",
            get(account::delete_page).post(account::delete_account),
        )
        .route(
            "/account/e/{id}/withdraw",
            limited(
                get(routes::withdraw::confirm_page).post(routes::withdraw::withdraw),
                MAX_FORM_BYTES,
            ),
        )
        .route("/admin", get(admin::queue))
        .route(
            "/admin/invites",
            get(admin::invites).post(admin::create_invite),
        )
        .route("/admin/invites/{code}/revoke", post(admin::revoke_invite))
        .route("/admin/unknown-fields", get(admin::unknown_fields))
        .route("/admin/e/{id}/release", post(admin::release))
        .route("/admin/e/{id}", get(admin::entry))
        .route("/admin/e/{id}/publish", post(admin::publish))
        .route("/admin/e/{id}/hold", post(admin::hold))
        .route("/admin/e/{id}/reject", post(admin::reject))
        .route("/admin/e/{id}/doubt-seen", post(admin::doubt_seen))
        .route("/admin/e/{id}/remove", post(admin::remove_entry))
        .route("/admin/e/{id}/restore", post(admin::restore_entry))
        .route("/admin/e/{id}/remove-chain", post(admin::remove_chain))
        .route(
            "/admin/e/{id}/purge",
            get(admin::purge_entry_page).post(admin::purge_entry),
        )
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
        .route(
            "/admin/accounts/{id}/purge",
            get(admin::purge_account_page).post(admin::purge_account),
        )
        .route("/admin/f/{sha256}", get(admin::file))
        .route("/admin/f/{sha256}/remove", post(admin::remove_file))
        .route(
            "/admin/f/{sha256}/remove-hazard",
            post(admin::remove_hazard),
        );
    let same_origin = axum::middleware::from_fn_with_state(state.clone(), account::same_origin);
    accounts
        .map_router(|router| Router::new().merge(router.layer(same_origin)))
        .route("/accounts", post(routes::agent_account::make))
        .route(
            "/account/claim",
            limited(post(routes::claim::ask), MAX_FORM_BYTES),
        )
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
        .route_if(
            state.environment != config::Environment::Production,
            request_log::READ_ROUTE,
            get(routes::headers::headers),
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
        .route("/install.sh", get(routes::docs::installer))
        .route("/skills/{name}/{*path}", get(routes::docs::skill_file))
        .route("/entry-format.md", get(routes::docs::entry_format))
        // Every route above reads at most `MAX_BODY_BYTES`; a route_layer reaches only
        // the routes added before it, so the one route that reads more follows.
        .map_router(|router| {
            router
                .route_layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
                .route_layer(axum::middleware::from_fn_with_state(
                    MAX_BODY_BYTES,
                    declared_within,
                ))
        })
        .route(
            "/attachments",
            limited(post(routes::attachments::upload), MAX_ATTACHMENT_BODY_BYTES),
        )
        .map_router(|router| {
            let files_host =
                axum::middleware::from_fn_with_state(state.clone(), routes::files::files_host);
            let per_ip = axum::middleware::from_fn_with_state(state.clone(), limits::per_ip);
            let record = axum::middleware::from_fn_with_state(state.clone(), request_log::record);
            router
                .route_layer(record)
                .fallback(not_found)
                .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
                .layer(axum::middleware::from_fn(respond::negotiate))
                .layer(files_host)
                .layer(per_ip)
                .with_state(state)
        })
}
