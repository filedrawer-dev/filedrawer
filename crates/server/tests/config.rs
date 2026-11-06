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

#[test]
fn the_anthropic_screener_needs_its_key() {
    let missing = config(&[("FILEDRAWER_SCREENER", "anthropic")]).unwrap_err();
    assert_eq!(
        missing,
        ConfigError::Missing("FILEDRAWER_ANTHROPIC_API_KEY")
    );
    let ready = config(&[
        ("FILEDRAWER_SCREENER", "anthropic"),
        ("FILEDRAWER_ANTHROPIC_API_KEY", "sk-not-a-real-key"),
    ])
    .unwrap();
    assert!(matches!(
        ready.screener,
        filedrawer_server::Screener::Models(_)
    ));
    // The key never shows in the settings' Debug output.
    assert!(!format!("{ready:?}").contains("sk-not-a-real-key"));
    let production = config(&[
        ("FILEDRAWER_ENVIRONMENT", "production"),
        ("FILEDRAWER_SCREENER", "anthropic"),
        ("FILEDRAWER_ANTHROPIC_API_KEY", "sk-not-a-real-key"),
        SMTP[0],
        SMTP[1],
        SMTP[2],
        SMTP[3],
    ]);
    assert!(production.is_ok(), "{production:?}");
}

#[test]
fn the_daily_screening_cap_defaults_to_a_dollar() {
    assert_eq!(config(&[]).unwrap().screening_daily_nano_usd, 1_000_000_000);
    let set = config(&[("FILEDRAWER_SCREENING_DAILY_USD", "0.25")]).unwrap();
    assert_eq!(set.screening_daily_nano_usd, 250_000_000);
    for bad in ["-1", "lots", "NaN", "5000"] {
        let refused = config(&[("FILEDRAWER_SCREENING_DAILY_USD", bad)]).unwrap_err();
        assert!(matches!(refused, ConfigError::Invalid { .. }), "{bad}");
    }
}

#[test]
fn staging_reads_the_smoke_account_and_production_refuses_it() {
    let staging = config(&[("FILEDRAWER_SMOKE_ACCOUNT", "a-smoke1")]).unwrap();
    assert_eq!(staging.smoke_account.as_deref(), Some("a-smoke1"));
    assert_eq!(config(&[]).unwrap().smoke_account, None);
    let mut production = vec![
        ("FILEDRAWER_ENVIRONMENT", "production"),
        ("FILEDRAWER_SCREENER", "anthropic"),
        ("FILEDRAWER_ANTHROPIC_API_KEY", "sk-ant-not-real"),
        ("FILEDRAWER_SMOKE_ACCOUNT", "a-smoke1"),
    ];
    production.extend(SMTP);
    let error = config(&production).unwrap_err();
    assert_eq!(error, ConfigError::SmokeAccountInProduction);
    assert!(error.to_string().contains("FILEDRAWER_SMOKE_ACCOUNT"));
}

#[test]
fn the_files_url_is_read_without_a_trailing_slash_and_must_be_an_http_address() {
    assert_eq!(config(&[]).unwrap().files_url, None);
    let set = config(&[(
        "FILEDRAWER_FILES_URL",
        "https://files.staging.filedrawer.dev/",
    )])
    .unwrap();
    assert_eq!(
        set.files_url.as_deref(),
        Some("https://files.staging.filedrawer.dev")
    );
    let bad = config(&[("FILEDRAWER_FILES_URL", "files.staging.filedrawer.dev")]).unwrap_err();
    assert!(bad.to_string().contains("FILEDRAWER_FILES_URL"), "{bad}");
}

#[test]
fn staging_reads_the_screen_test_account_and_production_refuses_it() {
    let staging = config(&[("FILEDRAWER_SCREEN_TEST_ACCOUNT", "a-screen1")]).unwrap();
    assert_eq!(staging.screen_test_account.as_deref(), Some("a-screen1"));
    let mut production = vec![
        ("FILEDRAWER_ENVIRONMENT", "production"),
        ("FILEDRAWER_SCREENER", "anthropic"),
        ("FILEDRAWER_ANTHROPIC_API_KEY", "sk-ant-not-real"),
        ("FILEDRAWER_SCREEN_TEST_ACCOUNT", "a-screen1"),
    ];
    production.extend(SMTP);
    let error = config(&production).unwrap_err();
    assert!(
        error.to_string().contains("FILEDRAWER_SCREEN_TEST_ACCOUNT"),
        "{error}"
    );
}

#[test]
fn attached_files_are_kept_beside_the_database() {
    let config = config(&[(
        "FILEDRAWER_DATABASE",
        "/srv/filedrawer/staging/data/filedrawer.db",
    )])
    .unwrap();
    assert_eq!(
        config.files_dir,
        std::path::Path::new("/srv/filedrawer/staging/data/files")
    );
}
