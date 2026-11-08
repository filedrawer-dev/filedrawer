//! The server's settings, from environment variables so systemd units can set them.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::screening::{AnthropicClient, Screener};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Staging,
    Dev,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database: String,
    pub listen: SocketAddr,
    pub environment: Environment,
    pub screener: Screener,
    /// The most screening may spend in a UTC day, in billionths of a dollar
    /// (design.md: $1, with at most 10% for one account).
    pub screening_daily_nano_usd: i64,
    /// The site's address as browsers reach it, with no trailing slash. Emails link
    /// to it, and form posts must come from it.
    pub public_url: String,
    /// `None` sends email to the log.
    pub smtp: Option<SmtpSettings>,
    /// Staging's smoke-test account, whose entries aren't screened, so CI's smoke
    /// test calls no model after each deploy. Production refuses it.
    pub smoke_account: Option<String>,
    /// Staging's screen-test account, which `ops/smoke-staging eval` submits as: not
    /// held to an account's share of the day's cap. Production refuses it.
    pub screen_test_account: Option<String>,
    /// Staging's CI screen account, which CI's `ops/smoke-staging screen` submits as
    /// after each deploy: outside the daily allowance. Production refuses it.
    pub ci_screen_account: Option<String>,
    /// Where attached files are served, on a host of their own with no cookies
    /// (design.md, Attached code), with no trailing slash. `None` turns attachments off.
    pub files_url: Option<String>,
    /// Attached files' bytes: `files/` beside the database file.
    pub files_dir: PathBuf,
    /// The most bytes of attached files the site stores in all (design.md,
    /// Attached code: 20 GB in production, 2 GB elsewhere).
    pub attachment_store_cap_bytes: i64,
    /// What the link checks call GitHub's API with: 5,000 calls an hour instead of
    /// 60 (design.md, Submission and screening, step 2). Production refuses to run
    /// without one.
    pub github_token: Option<GithubToken>,
    /// Whether submissions fetch their `code` and `logs` links. Off only for the
    /// tests that run the server binary, so they reach no real host; production
    /// refuses off.
    pub link_checks: bool,
    /// How often the publisher runs the screening queue, which is when an entry with
    /// attached code is screened: 30 seconds, or `FILEDRAWER_PUBLISHER_SECONDS` in dev,
    /// for tests that run the server binary.
    pub publisher_tick: std::time::Duration,
}

/// A GitHub token, which Debug output leaves out.
#[derive(Clone)]
pub struct GithubToken(String);

impl GithubToken {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for GithubToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GithubToken(..)")
    }
}

/// How to reach the SMTP server. Its Debug output leaves the password out.
#[derive(Clone)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// The From address, such as `File Drawer <noreply@filedrawer.dev>`.
    pub from: String,
}

impl std::fmt::Debug for SmtpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{0} is not set")]
    Missing(&'static str),
    #[error("{name} is `{value}`; expected {expected}")]
    Invalid {
        name: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error(
        "FILEDRAWER_SCREENER=pass-all publishes every entry unscreened, so production refuses it"
    )]
    UnscreenedProduction,
    #[error(
        "FILEDRAWER_SMOKE_ACCOUNT publishes an account's entries unscreened, so production refuses it"
    )]
    SmokeAccountInProduction,
    #[error(
        "FILEDRAWER_SCREEN_TEST_ACCOUNT lifts an account's share of the screening cap, so production refuses it"
    )]
    ScreenTestAccountInProduction,
    #[error(
        "FILEDRAWER_CI_SCREEN_ACCOUNT lifts an account's daily allowance, so production refuses it"
    )]
    CiScreenAccountInProduction,
    #[error(
        "production sends verification and reset email, so it needs FILEDRAWER_SMTP_HOST, FILEDRAWER_SMTP_USER, FILEDRAWER_SMTP_PASSWORD and FILEDRAWER_MAIL_FROM"
    )]
    NoMailInProduction,
    #[error(
        "set all of FILEDRAWER_SMTP_HOST, FILEDRAWER_SMTP_USER, FILEDRAWER_SMTP_PASSWORD and FILEDRAWER_MAIL_FROM, or none; {0} is missing"
    )]
    PartialSmtp(&'static str),
    #[error(
        "production checks code links with GitHub's API, which allows 60 calls an hour without a token, so it needs FILEDRAWER_GITHUB_TOKEN"
    )]
    NoGithubTokenInProduction,
    #[error("FILEDRAWER_LINK_CHECKS=off leaves every link unchecked, so production refuses it")]
    LinkChecksOffInProduction,
    #[error(
        "FILEDRAWER_PUBLISHER_SECONDS speeds up the screening queue for tests, so only dev takes it"
    )]
    PublisherTickOutsideDev,
}

/// The SMTP settings, if any are set. All four must be set together.
fn smtp_settings(
    var: &impl Fn(&str) -> Option<String>,
) -> Result<Option<SmtpSettings>, ConfigError> {
    let get = |name| var(name).filter(|value| !value.is_empty());
    let host = get("FILEDRAWER_SMTP_HOST");
    let user = get("FILEDRAWER_SMTP_USER");
    let password = get("FILEDRAWER_SMTP_PASSWORD");
    let from = get("FILEDRAWER_MAIL_FROM");
    let (host, user, password, from) = match (host, user, password, from) {
        (None, None, None, None) => return Ok(None),
        (Some(host), Some(user), Some(password), Some(from)) => (host, user, password, from),
        (None, ..) => return Err(ConfigError::PartialSmtp("FILEDRAWER_SMTP_HOST")),
        (_, None, ..) => return Err(ConfigError::PartialSmtp("FILEDRAWER_SMTP_USER")),
        (_, _, None, _) => return Err(ConfigError::PartialSmtp("FILEDRAWER_SMTP_PASSWORD")),
        (.., None) => return Err(ConfigError::PartialSmtp("FILEDRAWER_MAIL_FROM")),
    };
    let port = match var("FILEDRAWER_SMTP_PORT") {
        None => 465,
        Some(text) => text.parse().map_err(|_| ConfigError::Invalid {
            name: "FILEDRAWER_SMTP_PORT",
            value: text,
            expected: "a port number such as 465",
        })?,
    };
    Ok(Some(SmtpSettings {
        host,
        port,
        user,
        password,
        from,
    }))
}

impl Config {
    /// Reads the `FILEDRAWER_*` settings through `var`: `DATABASE`, `LISTEN`,
    /// `ENVIRONMENT`, `SCREENER` and `PUBLIC_URL`, and the optional SMTP ones.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let get = |name: &'static str| var(name).ok_or(ConfigError::Missing(name));
        let database = get("FILEDRAWER_DATABASE")?;
        let listen_text = get("FILEDRAWER_LISTEN")?;
        let listen = listen_text.parse().map_err(|_| ConfigError::Invalid {
            name: "FILEDRAWER_LISTEN",
            value: listen_text.clone(),
            expected: "an address such as 127.0.0.1:8080",
        })?;
        let environment = match get("FILEDRAWER_ENVIRONMENT")?.as_str() {
            "production" => Environment::Production,
            "staging" => Environment::Staging,
            "dev" => Environment::Dev,
            other => {
                return Err(ConfigError::Invalid {
                    name: "FILEDRAWER_ENVIRONMENT",
                    value: other.to_string(),
                    expected: "production, staging or dev",
                });
            }
        };
        let screener = match get("FILEDRAWER_SCREENER")?.as_str() {
            "pass-all" => Screener::PassAll,
            "anthropic" => {
                let key = var("FILEDRAWER_ANTHROPIC_API_KEY")
                    .filter(|key| !key.trim().is_empty())
                    .ok_or(ConfigError::Missing("FILEDRAWER_ANTHROPIC_API_KEY"))?;
                Screener::Models(Arc::new(AnthropicClient::new(key.trim().to_string())))
            }
            other => {
                return Err(ConfigError::Invalid {
                    name: "FILEDRAWER_SCREENER",
                    value: other.to_string(),
                    expected: "anthropic or pass-all",
                });
            }
        };
        let screening_daily_nano_usd = match var("FILEDRAWER_SCREENING_DAILY_USD") {
            None => 1_000_000_000,
            Some(text) => text
                .parse::<f64>()
                .ok()
                .filter(|usd| usd.is_finite() && *usd >= 0.0 && *usd <= 1000.0)
                // In range by the filter, so the cast can't truncate.
                .map(|usd| (usd * 1e9).round() as i64)
                .ok_or(ConfigError::Invalid {
                    name: "FILEDRAWER_SCREENING_DAILY_USD",
                    value: text,
                    expected: "dollars a day, such as 1.00",
                })?,
        };
        let public_url_text = get("FILEDRAWER_PUBLIC_URL")?;
        let public_url = public_url_text.trim_end_matches('/').to_string();
        if !(public_url.starts_with("https://") || public_url.starts_with("http://")) {
            return Err(ConfigError::Invalid {
                name: "FILEDRAWER_PUBLIC_URL",
                value: public_url_text,
                expected: "the site's address, such as https://staging.filedrawer.dev",
            });
        }
        let smtp = smtp_settings(&var)?;
        if environment == Environment::Production && smtp.is_none() {
            return Err(ConfigError::NoMailInProduction);
        }
        if environment == Environment::Production && matches!(screener, Screener::PassAll) {
            return Err(ConfigError::UnscreenedProduction);
        }
        let files_url = match var("FILEDRAWER_FILES_URL").filter(|url| !url.is_empty()) {
            None => None,
            Some(text) if text.starts_with("https://") || text.starts_with("http://") => {
                Some(text.trim_end_matches('/').to_string())
            }
            Some(text) => {
                return Err(ConfigError::Invalid {
                    name: "FILEDRAWER_FILES_URL",
                    value: text,
                    expected: "the files host's address, such as https://files.staging.filedrawer.dev",
                });
            }
        };
        // Staging's test accounts each lift a rule for one account, so production
        // refuses every one.
        let staging_account = |name: &str, refused: ConfigError| {
            let id = var(name).filter(|id| !id.is_empty());
            if environment == Environment::Production && id.is_some() {
                return Err(refused);
            }
            Ok(id)
        };
        let smoke_account = staging_account(
            "FILEDRAWER_SMOKE_ACCOUNT",
            ConfigError::SmokeAccountInProduction,
        )?;
        let screen_test_account = staging_account(
            "FILEDRAWER_SCREEN_TEST_ACCOUNT",
            ConfigError::ScreenTestAccountInProduction,
        )?;
        let ci_screen_account = staging_account(
            "FILEDRAWER_CI_SCREEN_ACCOUNT",
            ConfigError::CiScreenAccountInProduction,
        )?;
        let files_dir = Path::new(&database)
            .parent()
            .unwrap_or(Path::new(""))
            .join("files");
        let attachment_store_cap_bytes = match environment {
            Environment::Production => 20_000_000_000,
            Environment::Staging | Environment::Dev => 2_000_000_000,
        };
        let github_token = var("FILEDRAWER_GITHUB_TOKEN")
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
            .map(GithubToken);
        if environment == Environment::Production && github_token.is_none() {
            return Err(ConfigError::NoGithubTokenInProduction);
        }
        let link_checks = match var("FILEDRAWER_LINK_CHECKS").as_deref() {
            None | Some("on") => true,
            Some("off") => false,
            Some(other) => {
                return Err(ConfigError::Invalid {
                    name: "FILEDRAWER_LINK_CHECKS",
                    value: other.to_string(),
                    expected: "on or off",
                });
            }
        };
        if environment == Environment::Production && !link_checks {
            return Err(ConfigError::LinkChecksOffInProduction);
        }
        let publisher_tick = match var("FILEDRAWER_PUBLISHER_SECONDS") {
            None => std::time::Duration::from_secs(30),
            Some(_) if environment != Environment::Dev => {
                return Err(ConfigError::PublisherTickOutsideDev);
            }
            Some(text) => match text.parse::<u64>() {
                Ok(seconds) if seconds > 0 => std::time::Duration::from_secs(seconds),
                _ => {
                    return Err(ConfigError::Invalid {
                        name: "FILEDRAWER_PUBLISHER_SECONDS",
                        value: text,
                        expected: "a whole number of seconds from 1",
                    });
                }
            },
        };
        Ok(Config {
            link_checks,
            publisher_tick,
            github_token,
            files_dir,
            attachment_store_cap_bytes,
            database,
            listen,
            environment,
            screener,
            screening_daily_nano_usd,
            public_url,
            smtp,
            smoke_account,
            screen_test_account,
            ci_screen_account,
            files_url,
        })
    }
}
