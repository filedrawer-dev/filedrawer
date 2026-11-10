//! `GET /staging/headers`: the headers clients sent staging, grouped by client, for
//! agents that can't read the server's log (`request_log`, #28). The headers are the
//! clients' own text, so in plain text they come below the delimiter. Production
//! doesn't route it (`lib::build`).

use std::fmt::Write;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::AppState;
use crate::error::AppError;
use crate::request_log::{Client, by_client};
use crate::respond::{DELIMITER, Format};

pub async fn headers(State(state): State<AppState>, format: Format) -> Result<Response, AppError> {
    let Some(log) = state.request_log.clone() else {
        return Ok(format.reply(
            StatusCode::OK,
            "recording: no, only staging keeps a request log\n".to_string(),
            json!({"recording": false, "requests": 0, "clients": []}),
        ));
    };
    let records = tokio::task::spawn_blocking(move || log.records())
        .await
        .map_err(std::io::Error::other)??;
    let requests = records.len();
    let clients = by_client(records);
    let mut text = format!(
        "recording: yes\nrequests: {requests}\nclients: {}\n",
        clients.len()
    );
    if !clients.is_empty() {
        text.push_str(DELIMITER);
        text.push('\n');
        for client in &clients {
            client_lines(&mut text, client);
        }
    }
    let json = json!({"recording": true, "requests": requests, "clients": clients});
    Ok(format.reply(StatusCode::OK, text, json))
}

/// A blank line, then `client`'s lines.
fn client_lines(text: &mut String, client: &Client) {
    let sent = |value: &Option<String>| value.clone().unwrap_or_else(|| "(not sent)".into());
    let _ = write!(
        text,
        "\nclient: {} requests, {} given a page by the HTML rule, {} to {}\n\
         user-agent: {}\naccept: {}\nsec-fetch-dest: {}\nsec-fetch-mode: {}\n",
        client.requests,
        client.rule_pages,
        client.first,
        client.last,
        sent(&client.headers.user_agent),
        sent(&client.headers.accept),
        sent(&client.headers.sec_fetch_dest),
        sent(&client.headers.sec_fetch_mode),
    );
    for route in &client.routes {
        let _ = writeln!(
            text,
            "route: {} {} ({})",
            route.method, route.route, route.requests
        );
    }
}
