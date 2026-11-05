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
}

impl Config {
    /// Reads `FILEDRAWER_DATABASE`, `FILEDRAWER_LISTEN`, `FILEDRAWER_ENVIRONMENT` and
    /// `FILEDRAWER_SCREENER` through `var`.
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
        if environment == Environment::Production && screener == Screener::PassAll {
            return Err(ConfigError::UnscreenedProduction);
        }
        Ok(Config {
            database,
            listen,
            environment,
            screener,
        })
    }
}
