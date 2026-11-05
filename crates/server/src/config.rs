//! The server's settings, from environment variables so systemd units can set them.

use std::net::SocketAddr;

use crate::screening::Screener;

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
    /// The site's address as browsers reach it, with no trailing slash. Emails link
    /// to it, and form posts must come from it.
    pub public_url: String,
    /// `None` sends email to the log.
    pub smtp: Option<SmtpSettings>,
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
        "production sends verification and reset email, so it needs FILEDRAWER_SMTP_HOST, FILEDRAWER_SMTP_USER, FILEDRAWER_SMTP_PASSWORD and FILEDRAWER_MAIL_FROM"
    )]
    NoMailInProduction,
    #[error(
        "set all of FILEDRAWER_SMTP_HOST, FILEDRAWER_SMTP_USER, FILEDRAWER_SMTP_PASSWORD and FILEDRAWER_MAIL_FROM, or none; {0} is missing"
    )]
    PartialSmtp(&'static str),
}

/// The SMTP settings, if any are set. All four must be set together.
fn smtp_settings(
    var: &impl Fn(&str) -> Option<String>,
) -> Result<Option<SmtpSettings>, ConfigError> {
    const NAMES: [&str; 4] = [
        "FILEDRAWER_SMTP_HOST",
        "FILEDRAWER_SMTP_USER",
        "FILEDRAWER_SMTP_PASSWORD",
        "FILEDRAWER_MAIL_FROM",
    ];
    let values: Vec<Option<String>> = NAMES
        .iter()
        .map(|name| var(name).filter(|value| !value.is_empty()))
        .collect();
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let missing = NAMES.iter().zip(&values).find(|(_, value)| value.is_none());
    if let Some((name, _)) = missing {
        return Err(ConfigError::PartialSmtp(name));
    }
    let [Some(host), Some(user), Some(password), Some(from)] =
        <[Option<String>; 4]>::try_from(values).expect("four names, four values")
    else {
        unreachable!("every value was checked above");
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
            other => {
                return Err(ConfigError::Invalid {
                    name: "FILEDRAWER_SCREENER",
                    value: other.to_string(),
                    expected: "pass-all",
                });
            }
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
        if environment == Environment::Production && screener == Screener::PassAll {
            return Err(ConfigError::UnscreenedProduction);
        }
        Ok(Config {
            database,
            listen,
            environment,
            screener,
            public_url,
            smtp,
        })
    }
}
