//! `GET /staging/status`: what staging's screen spent today, what waits for it, and how
//! its last calls went, and what the link checks know of GitHub's calls and of hosts
//! that asked the server to wait, and how many emails wait to be sent and how the
//! last sends went, for agents that can't read the server's log (AGENTS.md). It names
//! no account, entry, address or secret. Production doesn't route it (`lib::build`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::config::Environment;
use crate::error::AppError;
use crate::link_checks::{GITHUB_HOURLY_WITHOUT_TOKEN, LinkHealth};
use crate::respond::{Format, one_line};
use crate::screening::{Model, Screener};
use crate::time::{now_utc, stamp};
use crate::{AppState, entries, mail_queue, queue};

const NONE_YET: &str = "none since the server started";

pub async fn status(State(state): State<AppState>, format: Format) -> Result<Response, AppError> {
    let environment = match state.environment {
        Environment::Production => "production",
        Environment::Staging => "staging",
        Environment::Dev => "dev",
    };
    let (screener, models, screener_text) = match state.screener {
        Screener::PassAll => ("pass-all", vec![], "pass-all".to_string()),
        Screener::Scripted => ("scripted", vec![], "scripted".to_string()),
        Screener::Models(_) => (
            "models",
            vec![Model::Haiku.id(), Model::Sonnet.id()],
            format!(
                "models ({}, then {} on a hazard doubt)",
                Model::Haiku.id(),
                Model::Sonnet.id()
            ),
        ),
    };
    let spent_usd = dollars(queue::spent_today(&state.pool, now_utc()).await?);
    let cap_usd = dollars(state.screening_daily_nano_usd);
    let (waiting, oldest) = entries::waiting_to_screen(&state.pool).await?;
    let health = state.screening_health.snapshot();
    let success = health.last_success.map(stamp);
    let failure = health.last_failure.map(|failure| {
        (
            stamp(failure.at),
            failure.subject.name(),
            one_line(&failure.error),
        )
    });
    let links = state.link_checker.health();
    let (mail_waiting, mail_oldest) = mail_queue::waiting(&state.pool).await?;
    let mail = state.mail_queue.health();
    let mail_sent = mail.last_sent.map(stamp);
    let mail_failure = mail.last_failure.map(|(at, failure)| (stamp(at), failure));
    let text = format!(
        "environment: {environment}\nscreener: {screener_text}\n\
         spent today: ${spent_usd:.4} of the day's ${cap_usd:.2}\nwaiting: {}\n\
         last success: {}\nlast failure: {}\n{}\
         emails waiting: {}\nemail last sent: {}\nemail last failure: {}\n",
        match &oldest {
            Some(oldest) => format!("{waiting}, the oldest submitted {oldest}"),
            None => waiting.to_string(),
        },
        success.as_deref().unwrap_or(NONE_YET),
        match &failure {
            Some((at, subject, error)) => format!("{at} ({subject}): {error}"),
            None => NONE_YET.to_string(),
        },
        link_lines(&links),
        match &mail_oldest {
            Some(oldest) => format!("{mail_waiting}, the oldest queued {oldest}"),
            None => mail_waiting.to_string(),
        },
        mail_sent.as_deref().unwrap_or(NONE_YET),
        match &mail_failure {
            Some((at, failure)) => format!("{at}: {failure}"),
            None => NONE_YET.to_string(),
        },
    );
    let json = json!({
        "environment": environment,
        "screener": screener,
        "models": models,
        "spent_today_usd": spent_usd,
        "cap_usd": cap_usd,
        "waiting": waiting,
        "oldest_waiting_since": oldest,
        "last_success": success,
        "last_failure": failure.map(|(at, subject, error)| {
            json!({"at": at, "subject": subject, "error": error})
        }),
        "github": {
            "hourly": links.github_hourly,
            "calls_left": links.calls_left.map(|calls| calls.left),
            "reset": links.calls_left.and_then(|calls| calls.reset).map(stamp),
            "half_spent": links.half_spent,
        },
        "paused_hosts": links.paused.iter().map(|paused| {
            json!({"host": paused.host, "until": stamp(paused.until)})
        }).collect::<Vec<_>>(),
        "commit_page_overruled": links.overruled,
        "email": {
            "waiting": mail_waiting,
            "oldest_waiting_since": mail_oldest,
            "last_sent": mail_sent,
            "last_failure": mail_failure.map(|(at, failure)| {
                json!({"at": at, "error": failure})
            }),
        },
    });
    Ok(format.reply(StatusCode::OK, text, json))
}

/// The text lines on what the link checks know of GitHub and the hosts.
fn link_lines(links: &LinkHealth) -> String {
    let allowance = if links.github_hourly == GITHUB_HOURLY_WITHOUT_TOKEN {
        format!(
            "{} calls an hour (no FILEDRAWER_GITHUB_TOKEN)",
            links.github_hourly
        )
    } else {
        format!("{} calls an hour", links.github_hourly)
    };
    let calls_left = match links.calls_left {
        Some(calls) => {
            let until = calls
                .reset
                .map_or(String::new(), |reset| format!(" until {}", stamp(reset)));
            let half = if links.half_spent {
                "; the server has used its half"
            } else {
                ""
            };
            format!("{}{until}{half}", calls.left)
        }
        None => "no answer from GitHub's API this hour".to_string(),
    };
    let paused = if links.paused.is_empty() {
        "none".to_string()
    } else {
        links
            .paused
            .iter()
            .map(|paused| format!("{} until {}", paused.host, stamp(paused.until)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "github allowance: {allowance}\ngithub calls left: {calls_left}\n\
         paused hosts: {paused}\ncommit page overruled: {}\n",
        links.overruled
    )
}

/// Billionths of a dollar, in dollars.
fn dollars(nano_usd: i64) -> f64 {
    nano_usd as f64 / 1e9
}
