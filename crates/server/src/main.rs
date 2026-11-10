#![forbid(unsafe_code)]
//! The `filedrawer` binary: the server, and the owner's admin commands. See `-h`.

use std::process::ExitCode;

use filedrawer_server::config::Config;
use filedrawer_server::daily_email::HttpPinger;
use filedrawer_server::file_store::FileStore;
use filedrawer_server::link_checks::LinkChecker;
use std::sync::Arc;

use filedrawer_server::mail::{LogMailer, Mailer, SmtpMailer};
use filedrawer_server::{AppState, admin, app, db, unpack};

const USAGE: &str = "\
usage:
  filedrawer serve
  filedrawer admin create-account --display-name NAME
  filedrawer admin create-token --account ID --name NAME
  filedrawer admin grant --account ID
  filedrawer unpack FILE...

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
zenodo.org, and ask GitHub whether a code commit is on one of the repository's own
branches or in one of its tags: its commit page first, then its API, spending at
most half of the token's hourly allowance.
FILEDRAWER_GITHUB_TOKEN, a token with no repository access, raises GitHub's
allowance from 60 calls an hour to 5,000; production refuses to start without it.
FILEDRAWER_LINK_CHECKS=off fetches nothing, for tests that run this binary;
production refuses it. FILEDRAWER_PUBLISHER_SECONDS sets how often the screening
queue runs, which is when an entry with attached code is screened: 30 seconds unless
set, and only dev takes it, for the same tests.

Once a day, after midnight UTC, the server emails the admins what screening
published, what waits for them, what failed, and new feedback, when there is
anything to report; outside production, only problems. After each run it pings
FILEDRAWER_DAILY_EMAIL_PING, a healthchecks.io ping URL (https://hc-ping.com/...),
or its /fail URL when the run failed; production refuses to start without it. The
email reports the newest nightly copy of the database in backups/ beside the
database file, named filedrawer-YYYY-MM-DD.db.

The admin commands need only FILEDRAWER_DATABASE. create-account prints the new
account's ID. create-token prints a new API token for the account; only its hash
is stored, so this is the only time it is shown. grant makes the account the admin:
its browser sessions reach the review queue at /admin. It prints the account's ID.

unpack runs the server's decoding on files read as one entry's attached files, as
the screen reads them, and prints for each how many parts it hides and how many
bytes, that it hides nothing, or the limit it passes, which sends an entry to the
owner's review. It never prints what a file hides. `ops/smoke-staging eval` runs it
on the field-test entries' attachment sets.";

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
        ["unpack", files @ ..] if !files.is_empty() => unpack_files(files),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// `filedrawer unpack FILE...`: what the server's decoding finds in the files, read as
/// one entry's attached files sharing one text budget, as the screen reads them. One
/// line a file, giving counts only: the hidden text, and the labels that quote the
/// start of each encoded run, stay unprinted, since field-test files mustn't be shown.
fn unpack_files(paths: &[&str]) -> ExitCode {
    let mut texts = Vec::new();
    for path in paths {
        match std::fs::read(path).map(String::from_utf8) {
            Ok(Ok(text)) => texts.push(text),
            Ok(Err(_)) => {
                eprintln!("filedrawer: {path} isn't UTF-8 text, as attached files must be");
                return ExitCode::from(2);
            }
            Err(error) => {
                eprintln!("filedrawer: can't read {path}: {error}");
                return ExitCode::from(2);
            }
        }
    }
    let mut budget = unpack::TextBudget::for_files(texts.iter().map(String::len).sum());
    for (path, text) in paths.iter().zip(&texts) {
        match unpack::unpack(text, &mut budget) {
            Ok(found) if found.is_empty() => println!("{path}: nothing hidden"),
            Ok(found) => {
                let bytes: usize = found.iter().map(|part| part.text.len()).sum();
                let parts = if found.len() == 1 { "part" } else { "parts" };
                println!("{path}: {} {parts}, {bytes} bytes hidden", found.len());
            }
            // Such a limit names an archive member, which is the file's hidden content.
            Err(unpack::Limit::Unopened(_)) => {
                println!("{path}: past a limit: it holds something the server doesn't open");
            }
            Err(limit) => println!("{path}: past a limit: {limit}"),
        }
    }
    ExitCode::SUCCESS
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
        // Paused (design.md, Embargo); no setting opens them.
        embargoes: filedrawer_server::Embargoes::Paused,
        pinger: config.daily_email_ping.as_ref().map(|url| {
            Arc::new(HttpPinger::new(url.expose()))
                as Arc<dyn filedrawer_server::daily_email::Pinger>
        }),
        backups_dir: config.backups_dir.clone(),
        started_at: time::OffsetDateTime::now_utc(),
    };
    tokio::spawn(filedrawer_server::jobs::run_publisher(
        state.clone(),
        config.publisher_tick,
    ));
    tokio::spawn(filedrawer_server::jobs::run_tidier(state.clone()));
    tokio::spawn(filedrawer_server::jobs::run_link_rechecker(state.clone()));
    tokio::spawn(filedrawer_server::jobs::run_releaser(state.clone()));
    tokio::spawn(filedrawer_server::jobs::run_daily_email(state.clone()));
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
