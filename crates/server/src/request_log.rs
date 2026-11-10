//! Staging's record of the headers each request sent, to check the HTML rule against
//! what agents' fetch tools send (design.md, Agent interface; #28). Each routed
//! request adds one JSON line to a file beside the database: its method, its route's
//! pattern, four headers and the rule's verdict; no IP, token, staging key or cookie.
//! `GET /staging/headers` (`routes::headers`) reads it back.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{MatchedPath, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::config::Environment;
use crate::respond::{one_line, wants_page};
use crate::time::{now_utc, stamp};

/// The log's file, beside the database.
const FILE_NAME: &str = "request-headers.jsonl";

/// Staging's cap on the file before it moves aside: the disk holds at most twice this.
const STAGING_CAP_BYTES: u64 = 4_000_000;

/// The most characters of a header the log keeps.
const HEADER_CHARS: usize = 300;

/// The route that reads the log, which a read doesn't add to.
pub const READ_ROUTE: &str = "/staging/headers";

/// The record of requests' headers: a file that, once it holds the cap, moves to
/// `<file>.1`, replacing the one before.
#[derive(Debug)]
pub struct RequestLog {
    path: PathBuf,
    cap_bytes: u64,
    /// Held while a line is written or the files are read, so no read meets a half
    /// line and no two writers move the file aside at once.
    lock: Mutex<()>,
}

/// The headers the log keeps from a request, each `None` when it wasn't sent. Two
/// requests that sent the same ones come from one client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SentHeaders {
    pub user_agent: Option<String>,
    pub accept: Option<String>,
    pub sec_fetch_dest: Option<String>,
    pub sec_fetch_mode: Option<String>,
}

impl SentHeaders {
    fn of(headers: &HeaderMap) -> SentHeaders {
        SentHeaders {
            user_agent: header(headers, "user-agent"),
            accept: header(headers, "accept"),
            sec_fetch_dest: header(headers, "sec-fetch-dest"),
            sec_fetch_mode: header(headers, "sec-fetch-mode"),
        }
    }
}

/// One request, as a line of the file.
#[derive(Debug, Serialize, Deserialize)]
pub struct Record {
    pub at: String,
    pub method: String,
    pub route: String,
    #[serde(flatten)]
    pub headers: SentHeaders,
    /// Whether the HTML rule (`respond::wants_page`) gives this request a page. A
    /// route can still answer plain text, as `/e/<id>.txt` does.
    pub rule_page: bool,
}

impl RequestLog {
    /// Staging's log, beside its database; none in production or dev (#28).
    pub fn for_environment(environment: Environment, database: &str) -> Option<RequestLog> {
        let dir = Path::new(database).parent().unwrap_or(Path::new(""));
        (environment == Environment::Staging).then(|| RequestLog::in_dir(dir, STAGING_CAP_BYTES))
    }

    /// A log in `dir` whose file moves aside once it holds `cap_bytes`.
    pub fn in_dir(dir: &Path, cap_bytes: u64) -> RequestLog {
        RequestLog {
            path: dir.join(FILE_NAME),
            cap_bytes,
            lock: Mutex::new(()),
        }
    }

    fn aside(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".1");
        PathBuf::from(name)
    }

    fn append(&self, record: &Record) -> std::io::Result<()> {
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        let _held = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let size = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        if size + line.len() as u64 > self.cap_bytes {
            std::fs::rename(&self.path, self.aside())?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(line.as_bytes())
    }

    /// Every record both files hold, oldest first. A line that doesn't parse, as one
    /// cut short by a full disk, is skipped.
    pub fn records(&self) -> std::io::Result<Vec<Record>> {
        let _held = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut records = Vec::new();
        for path in [self.aside(), self.path.clone()] {
            let file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for line in std::io::BufReader::new(file).lines() {
                if let Ok(record) = serde_json::from_str(&line?) {
                    records.push(record);
                }
            }
        }
        Ok(records)
    }
}

/// A header's value as the log keeps it: one line, cut to `HEADER_CHARS`; `None` when
/// the request didn't send it. Several values of one header join with `, `.
fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    let values: Vec<String> = headers
        .get_all(name)
        .iter()
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(
        one_line(&values.join(", "))
            .chars()
            .take(HEADER_CHARS)
            .collect(),
    )
}

/// Middleware: records each routed request's headers, when the server keeps a log.
/// A write that fails is logged and never fails the request.
pub async fn record(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_string());
    if let (Some(log), Some(route)) = (&state.request_log, route)
        && route != READ_ROUTE
    {
        let record = Record {
            at: stamp(now_utc()),
            method: request.method().to_string(),
            route,
            headers: SentHeaders::of(request.headers()),
            rule_page: wants_page(request.headers()),
        };
        let log = Arc::clone(log);
        let written = tokio::task::spawn_blocking(move || log.append(&record)).await;
        match written {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "can't write the request log"),
            Err(error) => tracing::warn!(%error, "the request log's writer stopped"),
        }
    }
    next.run(request).await
}

/// The headers one client sent: the requests that sent them, grouped.
#[derive(Debug, Serialize)]
pub struct Client {
    #[serde(flatten)]
    pub headers: SentHeaders,
    pub requests: usize,
    /// How many the HTML rule gives a page (`Record::rule_page`).
    pub rule_pages: usize,
    pub first: String,
    pub last: String,
    pub routes: Vec<RouteCount>,
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouteCount {
    pub method: String,
    pub route: String,
    pub requests: usize,
}

/// A client being counted: what it sent, and its requests by method and route.
struct Tally {
    client: Client,
    routes: BTreeMap<(String, String), usize>,
}

/// `records` grouped by the headers they sent, the most recently seen client first.
pub fn by_client(records: Vec<Record>) -> Vec<Client> {
    let mut groups: BTreeMap<SentHeaders, Tally> = BTreeMap::new();
    for record in records {
        let Tally { client, routes } =
            groups
                .entry(record.headers.clone())
                .or_insert_with(|| Tally {
                    client: Client {
                        headers: record.headers,
                        requests: 0,
                        rule_pages: 0,
                        first: record.at.clone(),
                        last: String::new(),
                        routes: Vec::new(),
                    },
                    routes: BTreeMap::new(),
                });
        client.requests += 1;
        client.rule_pages += usize::from(record.rule_page);
        client.last = record.at;
        *routes.entry((record.method, record.route)).or_default() += 1;
    }
    let mut clients: Vec<Client> = groups
        .into_values()
        .map(|Tally { mut client, routes }| {
            client.routes = routes
                .into_iter()
                .map(|((method, route), requests)| RouteCount {
                    method,
                    route,
                    requests,
                })
                .collect();
            client
        })
        .collect();
    clients.sort_by(|a, b| b.last.cmp(&a.last));
    clients
}
