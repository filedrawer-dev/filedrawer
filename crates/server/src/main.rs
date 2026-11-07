#![forbid(unsafe_code)]
//! The `filedrawer` binary: the server, and the owner's admin commands. See `-h`.

use std::process::ExitCode;

use filedrawer_server::config::Config;
use filedrawer_server::file_store::FileStore;
use filedrawer_server::link_checks::LinkChecker;
use std::sync::Arc;

use filedrawer_server::mail::{LogMailer, Mailer, SmtpMailer};
use filedrawer_server::{AppState, admin, app, db};

const USAGE: &str = "\
usage:
  filedrawer serve
  filedrawer admin create-account --display-name NAME
  filedrawer admin create-token --account ID --name NAME
  filedrawer admin grant --account ID

serve runs the server. It reads FILEDRAWER_DATABASE (the SQLite file),
FILEDRAWER_LISTEN (such as 127.0.0.1:8080), FILEDRAWER_ENVIRONMENT (production,
staging or dev), FILEDRAWER_SCREENER (pass-all; production refuses it) and
FILEDRAWER_PUBLIC_URL (such as https://staging.filedrawer.dev), which emails link to
and forms must come from. Attached files' bytes are kept in files/ beside the
database file.

FILEDRAWER_SCREEN_TEST_ACCOUNT names staging's screen-test account, which
`ops/smoke-staging eval` submits as: it isn't held to an account's share of the
day's screening cap, only to the cap. Production refuses it.

FILEDRAWER_CI_SCREEN_ACCOUNT names staging's CI screen account, which CI's
`ops/smoke-staging screen` submits as after each deploy: it has no daily allowance.
Production refuses it.

FILEDRAWER_SMOKE_ACCOUNT names staging's smoke-test account: its entries aren't
screened, and their cards say so, so CI's smoke test calls no model after each
deploy. Production refuses it.

Email goes through SMTP over TLS when FILEDRAWER_SMTP_HOST, FILEDRAWER_SMTP_USER,
FILEDRAWER_SMTP_PASSWORD and FILEDRAWER_MAIL_FROM are all set (FILEDRAWER_SMTP_PORT
defaults to 465), and to the log otherwise. Production refuses to start without them.

Submissions fetch their code and logs links on github.com, huggingface.co and
zenodo.org, and ask GitHub's API whether a code commit is on one of the repository's
own branches or tags, spending at most half of the token's hourly allowance.
FILEDRAWER_GITHUB_TOKEN, a token with no repository access, raises GitHub's
allowance from 60 calls an hour to 5,000; production refuses to start without it.
FILEDRAWER_LINK_CHECKS=off fetches nothing, for tests that run this binary;
production refuses it.

The admin commands need only FILEDRAWER_DATABASE. create-account prints the new
account's ID. create-token prints a new API token for the account; only its hash
is stored, so this is the only time it is shown. grant makes the account the admin:
its browser sessions reach the review queue at /admin. It prints the account's ID.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => return fail(&format!("can't start the async runtime: {error}")),
    };
    match args.as_slice() {
        ["-h" | "--help"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        ["serve"] => runtime.block_on(serve()),
        ["admin", "create-account", "--display-name", name] => {
            runtime.block_on(admin_command(async |pool| {
                admin::create_account(&pool, name).await
            }))
        }
        [
            "admin",
            "create-token",
            "--account",
            account,
            "--name",
            name,
        ] => runtime.block_on(admin_command(async |pool| {
            admin::create_token(&pool, account, name).await
        })),
        ["admin", "grant", "--account", account] => runtime.block_on(admin_command(async |pool| {
            admin::grant(&pool, account).await
        })),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Opens the database, runs `command`, and prints what it returns: an ID or a token.
async fn admin_command(
    command: impl AsyncFnOnce(sqlx::SqlitePool) -> Result<String, admin::AdminError>,
) -> ExitCode {
    let pool = match open_database().await {
        Ok(pool) => pool,
        Err(code) => return code,
    };
    match command(pool).await {
        Ok(printed) => {
            println!("{printed}");
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error.to_string()),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("filedrawer: {message}");
    ExitCode::FAILURE
}

async fn open_database() -> Result<sqlx::SqlitePool, ExitCode> {
    let Ok(path) = std::env::var("FILEDRAWER_DATABASE") else {
        return Err(fail("FILEDRAWER_DATABASE is not set"));
    };
    db::open(&path)
        .await
        .map_err(|error| fail(&format!("can't open the database {path}: {error}")))
}

async fn serve() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = match Config::from_env(|name| std::env::var(name).ok()) {
        Ok(config) => config,
        Err(error) => return fail(&error.to_string()),
    };
    // Before the database, whose migrations an older binary can't undo: a rollback
    // after a directory the store can't make would then still start.
    let files = match FileStore::open(config.files_dir.clone()) {
        Ok(files) => Arc::new(files),
        Err(error) => {
            return fail(&format!(
                "can't open the attached files' directory {}: {error}",
                config.files_dir.display()
            ));
        }
    };
    let pool = match db::open(&config.database).await {
        Ok(pool) => pool,
        Err(error) => {
            return fail(&format!(
                "can't open the database {}: {error}",
                config.database
            ));
        }
    };
    let listener = match tokio::net::TcpListener::bind(config.listen).await {
        Ok(listener) => listener,
        Err(error) => return fail(&format!("can't listen on {}: {error}", config.listen)),
    };
    tracing::info!(listen = %config.listen, environment = ?config.environment, "serving");
    let mailer: Arc<dyn Mailer> = match &config.smtp {
        Some(settings) => match SmtpMailer::new(settings) {
            Ok(mailer) => Arc::new(mailer),
            Err(error) => return fail(&format!("can't set up SMTP: {error}")),
        },
        None => {
            tracing::warn!("no SMTP settings: emails go to the log");
            Arc::new(LogMailer)
        }
    };
    if config.github_token.is_none() {
        tracing::warn!(
            "no FILEDRAWER_GITHUB_TOKEN: the link checks may call GitHub's API 60 times an hour"
        );
    }
    let state = AppState {
        pool,
        screener: config.screener,
        screening_daily_nano_usd: config.screening_daily_nano_usd,
        screening_lock: Default::default(),
        mailer,
        public_url: config.public_url.clone(),
        smoke_account: config.smoke_account.clone(),
        screen_test_account: config.screen_test_account.clone(),
        ci_screen_account: config.ci_screen_account.clone(),
        files_url: config.files_url.clone(),
        files,
        attachment_store_cap_bytes: config.attachment_store_cap_bytes,
        attachment_disk_floor_bytes: filedrawer_server::ATTACHMENT_DISK_FLOOR_BYTES,
        rate_limits: std::sync::Arc::new(filedrawer_server::limits::RateLimits::new(
            filedrawer_server::limits::Limits::site(),
        )),
        environment: config.environment,
        screening_health: Default::default(),
        link_checker: Arc::new(if config.link_checks {
            LinkChecker::over_https(
                config
                    .github_token
                    .as_ref()
                    .map(|token| token.expose().to_string()),
            )
        } else {
            LinkChecker::off()
        }),
    };
    tokio::spawn(filedrawer_server::jobs::run_publisher(state.clone()));
    tokio::spawn(filedrawer_server::jobs::run_tidier(state.clone()));
    tokio::spawn(filedrawer_server::jobs::run_link_rechecker(state.clone()));
    let router = app(state);
    let shutdown = async {
        // systemd stops a service with SIGTERM; Ctrl-C is for a terminal. A failure
        // to install a handler leaves the other, or SIGKILL.
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut signal) => {
                    signal.recv().await;
                }
                Err(_) => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = terminate => {}
        }
        tracing::info!("shutting down");
    };
    // The peer's address reaches the rate limits, which key on the client's IP.
    let router = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
    match axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&format!("the server stopped: {error}")),
    }
}
