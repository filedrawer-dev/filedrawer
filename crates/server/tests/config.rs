//! Seam: `Config::from_env`, the settings `filedrawer serve` reads.

use std::collections::HashMap;

use filedrawer_server::config::{Config, ConfigError};

fn config(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let mut vars: HashMap<String, String> = [
        ("FILEDRAWER_DATABASE", "/tmp/x.db"),
        ("FILEDRAWER_LISTEN", "127.0.0.1:8081"),
        ("FILEDRAWER_ENVIRONMENT", "staging"),
        ("FILEDRAWER_SCREENER", "pass-all"),
        ("FILEDRAWER_PUBLIC_URL", "https://staging.filedrawer.dev/"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    for (key, value) in pairs {
        vars.insert(key.to_string(), value.to_string());
    }
    Config::from_env(|name| vars.get(name).cloned())
}

const SMTP: [(&str, &str); 4] = [
    ("FILEDRAWER_SMTP_HOST", "smtp.purelymail.com"),
    ("FILEDRAWER_SMTP_USER", "noreply@filedrawer.dev"),
    ("FILEDRAWER_SMTP_PASSWORD", "hunter2-not-real"),
    (
        "FILEDRAWER_MAIL_FROM",
        "File Drawer <noreply@filedrawer.dev>",
    ),
];

#[test]
fn staging_without_smtp_logs_email() {
    let config = config(&[]).unwrap();
    assert!(config.smtp.is_none());
    assert_eq!(config.public_url, "https://staging.filedrawer.dev");
}

#[test]
fn smtp_settings_are_read_with_port_465_by_default_and_debug_hides_the_password() {
    let config = config(&SMTP).unwrap();
    let smtp = config.smtp.unwrap();
    assert_eq!(
        (smtp.host.as_str(), smtp.port),
        ("smtp.purelymail.com", 465)
    );
    assert!(!format!("{smtp:?}").contains("hunter2"));
}

#[test]
fn half_the_smtp_settings_is_an_error_naming_the_missing_one() {
    let error = config(&SMTP[..2]).unwrap_err();
    assert_eq!(error, ConfigError::PartialSmtp("FILEDRAWER_SMTP_PASSWORD"));
}

#[test]
fn production_needs_smtp() {
    let error = config(&[("FILEDRAWER_ENVIRONMENT", "production")]).unwrap_err();
    assert_eq!(error, ConfigError::NoMailInProduction);
}

#[test]
fn the_public_url_must_be_an_http_address() {
    assert!(matches!(
        config(&[("FILEDRAWER_PUBLIC_URL", "staging.filedrawer.dev")]),
        Err(ConfigError::Invalid {
            name: "FILEDRAWER_PUBLIC_URL",
            ..
        })
    ));
}
