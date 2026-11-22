//! `filedrawer-server check-outbound`: whether the server can reach the hosts it
//! calls, run by the deploys under the service's own sandbox (#102).
//!
//! Its calls are real, not through the traits the server's other callers of these
//! hosts take, since the way out of the sandbox is what it checks; its tests call
//! local listeners and closed ports.

use std::time::Duration;

use crate::config::{Config, SmtpSettings};
use crate::error::with_causes;
use crate::screening::Screener;

/// How long each host gets.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// A host the probe calls, and how.
#[derive(Debug, Clone)]
pub enum Target {
    /// An unauthenticated GET of this URL: any HTTP answer shows the way out.
    Http(String),
    /// An SMTP login and a NOOP, sending nothing.
    Smtp(SmtpSettings),
}

/// Whether the probe calls a host, and how, or why not.
#[derive(Debug, Clone)]
pub enum Step {
    Call(Target),
    Skip(&'static str),
}

/// One host of the probe.
#[derive(Debug, Clone)]
pub struct Planned {
    pub host: String,
    pub step: Step,
}

/// What one host did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Reached,
    Skipped(&'static str),
    Failed(String),
}

/// The hosts `config` has the server call, at the URLs the server calls.
pub fn plan(config: &Config) -> Vec<Planned> {
    let http = |host: &str, url: &str, called: bool, why: &'static str| Planned {
        host: host.to_string(),
        step: if called {
            Step::Call(Target::Http(url.to_string()))
        } else {
            Step::Skip(why)
        },
    };
    let smtp = match &config.smtp {
        Some(settings) => Planned {
            host: format!("{}:{}", settings.host, settings.port),
            step: Step::Call(Target::Smtp(settings.clone())),
        },
        None => Planned {
            host: "SMTP".to_string(),
            step: Step::Skip("no SMTP settings"),
        },
    };
    vec![
        smtp,
        http(
            "api.anthropic.com",
            crate::screening::API_URL,
            matches!(config.screener, Screener::Models(_)),
            "FILEDRAWER_SCREENER isn't anthropic",
        ),
        // GitHub sign-in reads the user from the API too, link checks or not.
        http(
            "api.github.com",
            crate::link_checks::GITHUB_API,
            config.link_checks || config.github_oauth.is_some(),
            "FILEDRAWER_LINK_CHECKS=off and no GitHub OAuth App",
        ),
        http(
            "github.com",
            crate::github_login::GITHUB_WEB,
            config.github_oauth.is_some(),
            "no FILEDRAWER_GITHUB_CLIENT_ID",
        ),
        // The host's root, never the ping URL, which would report the daily job;
        // the settings take no other host.
        http(
            "hc-ping.com",
            "https://hc-ping.com/",
            config.daily_email_ping.is_some(),
            "no FILEDRAWER_DAILY_EMAIL_PING",
        ),
    ]
}

/// Calls each planned host at once, each within `timeout`.
pub async fn run(plan: Vec<Planned>, timeout: Duration) -> Vec<(String, Outcome)> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("a client with only a timeout and a redirect policy builds");
    let client = &client;
    futures::future::join_all(plan.into_iter().map(|planned| async move {
        let outcome = match planned.step {
            Step::Skip(why) => Outcome::Skipped(why),
            Step::Call(target) => {
                match tokio::time::timeout(timeout, call(client, target, timeout)).await {
                    Ok(Ok(())) => Outcome::Reached,
                    Ok(Err(why)) => Outcome::Failed(why),
                    Err(_) => {
                        Outcome::Failed(format!("no answer within {}s", timeout.as_secs_f32()))
                    }
                }
            }
        };
        (planned.host, outcome)
    }))
    .await
}

/// One call, and why it failed, with the error's causes.
async fn call(client: &reqwest::Client, target: Target, timeout: Duration) -> Result<(), String> {
    match target {
        // Any answer: a 4xx from an unauthenticated call shows the way out as well as
        // a 200, and rustls has checked the host's certificate before it.
        Target::Http(url) => client
            .get(&url)
            .send()
            .await
            .map(drop)
            .map_err(|error| with_causes(&error)),
        Target::Smtp(settings) => crate::mail::check_login(&settings, timeout)
            .await
            .map_err(|error| with_causes(&error)),
    }
}
/// One line of the probe's output for a host, in printable ASCII alone: the install
/// passes on only such lines.
pub fn line(host: &str, outcome: &Outcome) -> String {
    let text = match outcome {
        Outcome::Reached => format!("reached {host}"),
        Outcome::Skipped(why) => format!("skipped {host} ({why})"),
        Outcome::Failed(why) => format!("can't reach {host}: {why}"),
    };
    text.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            ' '..='~' => c,
            _ => '?',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(extra: &[(&str, &str)]) -> Config {
        let mut vars = vec![
            ("FILEDRAWER_DATABASE", "/nowhere/filedrawer.db"),
            ("FILEDRAWER_LISTEN", "127.0.0.1:0"),
            ("FILEDRAWER_ENVIRONMENT", "staging"),
            ("FILEDRAWER_SCREENER", "pass-all"),
            ("FILEDRAWER_PUBLIC_URL", "https://staging.example"),
        ];
        vars.extend_from_slice(extra);
        Config::from_env(|name| {
            vars.iter()
                .rev()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
        .unwrap()
    }

    fn hosts(plan: &[Planned]) -> Vec<(String, bool)> {
        plan.iter()
            .map(|planned| (planned.host.clone(), matches!(planned.step, Step::Call(_))))
            .collect()
    }

    #[test]
    fn a_bare_config_calls_only_github() {
        let plan = plan(&config(&[]));
        assert_eq!(
            hosts(&plan),
            [
                ("SMTP".to_string(), false),
                ("api.anthropic.com".to_string(), false),
                ("api.github.com".to_string(), true),
                ("github.com".to_string(), false),
                ("hc-ping.com".to_string(), false),
            ]
        );
    }

    #[test]
    fn each_setting_adds_its_host() {
        let plan = plan(&config(&[
            ("FILEDRAWER_SMTP_HOST", "smtp.example"),
            ("FILEDRAWER_SMTP_USER", "u"),
            ("FILEDRAWER_SMTP_PASSWORD", "secret"),
            ("FILEDRAWER_MAIL_FROM", "x@example.com"),
            ("FILEDRAWER_SCREENER", "anthropic"),
            ("FILEDRAWER_ANTHROPIC_API_KEY", "k"),
            ("FILEDRAWER_DAILY_EMAIL_PING", "https://hc-ping.com/abc"),
            ("FILEDRAWER_LINK_CHECKS", "off"),
        ]));
        assert_eq!(
            hosts(&plan),
            [
                ("smtp.example:465".to_string(), true),
                ("api.anthropic.com".to_string(), true),
                ("api.github.com".to_string(), false),
                ("github.com".to_string(), false),
                ("hc-ping.com".to_string(), true),
            ]
        );
        // The ping host's root only: a GET of the ping URL would report the job.
        let ping = plan.iter().find(|p| p.host == "hc-ping.com").unwrap();
        assert!(
            matches!(&ping.step, Step::Call(Target::Http(url)) if url == "https://hc-ping.com/")
        );
    }

    #[tokio::test]
    async fn any_http_answer_is_a_way_out_and_a_closed_port_names_its_cause() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buffer = [0; 1024];
                let _ = socket.read(&mut buffer).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                    .await;
            }
        });
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let results = run(
            vec![
                Planned {
                    host: "open".into(),
                    step: Step::Call(Target::Http(format!("http://{open}/"))),
                },
                Planned {
                    host: "closed".into(),
                    step: Step::Call(Target::Http(format!("http://{closed}/"))),
                },
                Planned {
                    host: "off".into(),
                    step: Step::Skip("not set"),
                },
            ],
            TIMEOUT,
        )
        .await;
        assert_eq!(results[0], ("open".to_string(), Outcome::Reached));
        assert_eq!(results[2], ("off".to_string(), Outcome::Skipped("not set")));
        let Outcome::Failed(why) = &results[1].1 else {
            panic!("{results:?}")
        };
        // with_causes: reqwest's own message, then the connection error beneath it.
        assert!(why.contains(": "), "{why}");
        assert!(why.to_lowercase().contains("connect"), "{why}");
    }

    #[tokio::test]
    async fn a_host_that_never_answers_fails_at_the_timeout() {
        // Accepts, then says nothing.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let silent = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let started = std::time::Instant::now();
        let results = run(
            vec![Planned {
                host: "silent".into(),
                step: Step::Call(Target::Http(format!("http://{silent}/"))),
            }],
            Duration::from_millis(300),
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(matches!(&results[0].1, Outcome::Failed(_)), "{results:?}");
    }

    #[tokio::test]
    async fn an_smtp_host_that_refuses_fails_without_the_password() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let results = run(
            vec![Planned {
                host: "smtp".into(),
                step: Step::Call(Target::Smtp(SmtpSettings {
                    host: "127.0.0.1".into(),
                    port: closed.port(),
                    user: "u".into(),
                    password: "hunter2".into(),
                    from: "x@example.com".into(),
                })),
            }],
            TIMEOUT,
        )
        .await;
        let Outcome::Failed(why) = &results[0].1 else {
            panic!("{results:?}")
        };
        assert!(!why.contains("hunter2"), "{why}");
    }

    #[test]
    fn lines_say_what_each_host_did() {
        assert_eq!(line("a.example", &Outcome::Reached), "reached a.example");
        assert_eq!(
            line("a.example", &Outcome::Skipped("not set")),
            "skipped a.example (not set)"
        );
        assert_eq!(
            line("a.example", &Outcome::Failed("refused".into())),
            "can't reach a.example: refused"
        );
    }

    #[test]
    fn a_line_holds_only_printable_ascii() {
        // The install passes on only such lines, so a cause with a newline or a
        // control character mustn't drop the whole line.
        assert_eq!(
            line("a.example", &Outcome::Failed("one\ntwo\u{1b}[0m é".into())),
            "can't reach a.example: one two?[0m ?"
        );
    }
}
