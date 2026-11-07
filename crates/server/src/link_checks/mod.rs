//! The online half of the link checks (design.md, Submission and screening, step 2):
//! whether each `code` and `logs` link on a listed host answers, whether a GitHub
//! `code` commit is on one of the named repository's own branches or at one of its
//! tags, and whether a Zenodo `code` link names a version record. The offline half, by the URL's shape,
//! is the analyzer's. Only a pin found bad at submission refuses an entry; anything
//! else becomes a warning and a flag on the card (the owner's decisions, 2026-11-07).

mod fetch;
mod github;
mod pause;
mod stored;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use analyzer::{
    CardContext, Fields, LISTED_HOSTS, Pin, PinFlag, code_pin, github_commit, listed_host,
    zenodo_record,
};
use futures::StreamExt;
use reqwest::Url;

pub use fetch::{
    Answer, ApiAnswer, Fetcher, Fetching, GITHUB_HOURLY_WITH_TOKEN, GITHUB_HOURLY_WITHOUT_TOKEN,
    HttpFetcher,
};
pub use stored::{RECHECK_DAYS, pin_flag, recheck_due, store, stored};

use crate::AppState;
use crate::respond::one_line;

/// The fetcher and what the server knows of its GitHub API calls.
pub struct LinkChecker {
    fetcher: Arc<dyn Fetcher>,
    /// GitHub's hourly allowance of API calls: 5,000 with a token, 60 without.
    github_hourly: u32,
    /// The calls left and when they come back, in Unix seconds, as GitHub's last
    /// answer said.
    github_quota: Mutex<Option<(u32, i64)>>,
    /// The hosts that asked the server to wait, `api.github.com` among them.
    pauses: pause::Pauses,
    /// Checks nothing.
    off: bool,
}

impl LinkChecker {
    pub fn new(fetcher: Arc<dyn Fetcher>, github_hourly: u32) -> LinkChecker {
        LinkChecker {
            fetcher,
            github_hourly,
            github_quota: Mutex::new(None),
            pauses: pause::Pauses::default(),
            off: false,
        }
    }

    /// A checker that checks nothing (`Config::link_checks`).
    pub fn off() -> LinkChecker {
        LinkChecker {
            off: true,
            ..LinkChecker::new(
                Arc::new(HttpFetcher::new(None)),
                GITHUB_HOURLY_WITHOUT_TOKEN,
            )
        }
    }

    /// A checker on the real hosts, calling GitHub's API with `token` if given.
    pub fn over_https(token: Option<String>) -> LinkChecker {
        let hourly = if token.is_some() {
            GITHUB_HOURLY_WITH_TOKEN
        } else {
            GITHUB_HOURLY_WITHOUT_TOKEN
        };
        LinkChecker::new(Arc::new(HttpFetcher::new(token)), hourly)
    }
}

/// The most redirects a link may take.
const MAX_REDIRECTS: usize = 5;

/// The most fetches one request runs at once.
const CONCURRENT_FETCHES: usize = 8;

/// How long one request's checks may take in all. A check still running then counts
/// as "retry later", so a batch of 100 entries can't hold a request for minutes.
const CHECK_BUDGET: Duration = Duration::from_secs(20);

/// Hosts a listed host hands downloads to. A redirect to one counts as live without
/// a fetch.
const DOWNLOAD_HOSTS: [&str; 1] = ["codeload.github.com"];

/// GitHub's file and release hosts (`raw.`, `objects.`, `release-assets.` and
/// others), and Hugging Face's CDN, by suffix.
const DOWNLOAD_SUFFIXES: [&str; 3] = [".githubusercontent.com", ".huggingface.co", ".hf.co"];

/// Which field a link is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Code,
    Logs,
}

impl Field {
    /// Its name in an entry, and in the `field` column.
    fn name(self) -> &'static str {
        match self {
            Field::Code => "code",
            Field::Logs => "logs",
        }
    }
}

/// Whether a link answered.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    Live,
    /// With what it answered, such as "answered 404".
    Dead(String),
    /// It answered 408, 429 or 5xx, or not at all, or its host asked the server to
    /// wait so nothing was sent: not dead, and checked again soon.
    RetryLater(String),
}

/// Whether a `code` link's pin was confirmed online.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinCheck {
    Verified,
    /// With why the server couldn't tell.
    NotVerified(String),
    /// With what's wrong: a Zenodo concept record.
    Failed(String),
    /// A GitHub commit on no branch or tag of the repository the link names: a
    /// fork's, or one whose branch was squashed or rebased away. With what the server
    /// checked and how to clear it. Flagged, never refused (the owner's decision,
    /// 2026-11-07).
    OnNoBranch(String),
}

/// One link as the server last checked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedLink {
    pub field: Field,
    pub url: String,
    pub reach: Reach,
    /// For a `code` link on GitHub or Zenodo.
    pub pin: Option<PinCheck>,
}

/// The links of `fields` the server checks, each once: `code` and `logs` links whose
/// host is listed, and a `code` link to a Zenodo DOI, which is checked on Zenodo.
fn checked_links(fields: &Fields) -> Vec<(Field, String)> {
    let mut links = Vec::new();
    let code = fields.code.iter().map(|url| (Field::Code, url));
    let logs = fields.logs.iter().map(|url| (Field::Logs, url));
    for (field, url) in code.chain(logs) {
        let url = url.trim().to_string();
        let checked = listed_host(&url).is_some()
            || (field == Field::Code && code_pin(&url) == Pin::CheckedOnline);
        if checked && !links.contains(&(field, url.clone())) {
            links.push((field, url));
        }
    }
    links
}

/// Whether the server may fetch `url`: plain http(s) on a listed host, with no user
/// or port, which could point a listed name at another service.
fn fetchable(url: &Url) -> bool {
    matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url
            .host_str()
            .is_some_and(|host| LISTED_HOSTS.contains(&host.to_ascii_lowercase().as_str()))
}

fn is_download_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    DOWNLOAD_HOSTS.contains(&host.as_str())
        || DOWNLOAD_SUFFIXES
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

/// Checks the `code` and `logs` links of each entry, in order. `account` is charged
/// for the GitHub API calls; `None` for the server's own rechecks.
pub async fn check(
    state: &AppState,
    entries: &[&Fields],
    account: Option<&str>,
) -> Vec<Vec<CheckedLink>> {
    let asked: Vec<Vec<(Field, String)>> = entries
        .iter()
        .map(|fields| {
            if state.link_checker.off {
                Vec::new()
            } else {
                checked_links(fields)
            }
        })
        .collect();
    let deadline = tokio::time::Instant::now() + CHECK_BUDGET;
    // A Zenodo DOI isn't fetched; its pin check finds whether it answers.
    let urls: BTreeSet<String> = asked
        .iter()
        .flatten()
        .filter(|(_, url)| listed_host(url).is_some())
        .map(|(_, url)| url.clone())
        .collect();
    let reaches: HashMap<String, Reach> = futures::stream::iter(urls)
        .map(|url| reach_by(state, url, deadline))
        .buffer_unordered(CONCURRENT_FETCHES)
        .collect()
        .await;
    let code_urls: BTreeSet<(String, Option<Reach>)> = asked
        .iter()
        .flatten()
        .filter(|(field, _)| *field == Field::Code)
        .map(|(_, url)| (url.clone(), reaches.get(url).cloned()))
        .collect();
    let pins: HashMap<String, PinOutcome> = futures::stream::iter(code_urls)
        .map(|(url, reach)| pin_by(state, url, reach, account, deadline))
        .buffer_unordered(CONCURRENT_FETCHES)
        .collect()
        .await;
    asked
        .into_iter()
        .map(|links| {
            links
                .into_iter()
                .map(|(field, url)| {
                    let outcome = match field {
                        Field::Code => pins
                            .get(&url)
                            .cloned()
                            .expect("every code link had its pin checked"),
                        Field::Logs => PinOutcome::default(),
                    };
                    CheckedLink {
                        field,
                        reach: reaches
                            .get(&url)
                            .cloned()
                            .or(outcome.reach)
                            .unwrap_or_else(|| Reach::RetryLater(stopped_waiting())),
                        pin: outcome.pin,
                        url,
                    }
                })
                .collect()
        })
        .collect()
}

/// A `code` link's pin check, and whether the record it names answers when the link
/// itself isn't fetched, as for a Zenodo DOI.
#[derive(Debug, Clone, Default)]
struct PinOutcome {
    pin: Option<PinCheck>,
    reach: Option<Reach>,
}

/// `url` with its reach, or "retry later" if the request's checks ran out of time.
async fn reach_by(
    state: &AppState,
    url: String,
    deadline: tokio::time::Instant,
) -> (String, Reach) {
    let reach = tokio::time::timeout_at(deadline, reach(state, &url))
        .await
        .unwrap_or_else(|_| Reach::RetryLater(stopped_waiting()));
    (url, reach)
}

/// `url` with its pin check, or "not verified" if the request's checks ran out of
/// time. `reach` is `None` when the link itself isn't fetched.
async fn pin_by(
    state: &AppState,
    url: String,
    reach: Option<Reach>,
    account: Option<&str>,
    deadline: tokio::time::Instant,
) -> (String, PinOutcome) {
    let checked = tokio::time::timeout_at(deadline, pin(state, &url, reach.as_ref(), account))
        .await
        .unwrap_or_else(|_| PinOutcome {
            pin: Some(PinCheck::NotVerified(stopped_waiting())),
            reach: None,
        });
    (url, checked)
}

fn stopped_waiting() -> String {
    format!(
        "the server stopped waiting after {} s",
        CHECK_BUDGET.as_secs()
    )
}

/// Whether `url` answers, following redirects that stay on the listed hosts.
async fn reach(state: &AppState, url: &str) -> Reach {
    let Ok(mut current) = Url::parse(url) else {
        return Reach::Dead("isn't a URL the server can read".into());
    };
    if !fetchable(&current) {
        return Reach::Dead("names a user or a port, which the server doesn't fetch".into());
    }
    for _ in 0..=MAX_REDIRECTS {
        let host = current.host_str().unwrap_or("").to_ascii_lowercase();
        let answer = match state.link_checker.fetch_from(&host, current.as_str()).await {
            Ok(answer) => answer,
            Err(no_answer) => return Reach::RetryLater(no_answer.why(&host)),
        };
        let status = answer.status;
        match status {
            200..=299 => return Reach::Live,
            300..=399 => {
                let Some(next) = answer
                    .location
                    .and_then(|location| current.join(&location).ok())
                else {
                    return Reach::Dead(format!("answered {status} without a Location"));
                };
                let host = next.host_str().unwrap_or("").to_ascii_lowercase();
                if is_download_host(&host) {
                    return Reach::Live;
                }
                if !fetchable(&next) {
                    return Reach::Dead(format!(
                        "redirects to {host}, which the server doesn't fetch"
                    ));
                }
                current = next;
            }
            408 | 429 | 500..=599 => return Reach::RetryLater(format!("it answered {status}")),
            _ => return Reach::Dead(format!("answered {status}")),
        }
    }
    Reach::Dead(format!("redirects more than {MAX_REDIRECTS} times"))
}

/// Why a fetch has no answer.
enum NoAnswer {
    /// The host asked the server to wait, so nothing was sent.
    Waiting,
    /// It didn't answer, with why.
    Silent(String),
}

impl NoAnswer {
    /// Why a link on `host` has no answer, for the author.
    fn why(&self, host: &str) -> String {
        match self {
            NoAnswer::Waiting => {
                format!("{host} asked the server to wait before fetching from it again")
            }
            NoAnswer::Silent(why) => format!("it didn't answer ({why})"),
        }
    }
}

impl LinkChecker {
    /// GETs `url` from `host`, unless `host` asked the server to wait; a 429, or a
    /// 503 with `Retry-After`, makes it wait.
    async fn fetch_from(&self, host: &str, url: &str) -> Result<Answer, NoAnswer> {
        if self.pauses.holds(host) {
            return Err(NoAnswer::Waiting);
        }
        let answer = self.fetcher.get(url).await.map_err(NoAnswer::Silent)?;
        if answer.status == 429 || (answer.status == 503 && answer.retry_after.is_some()) {
            self.pauses.pause(host, answer.retry_after);
        } else {
            self.pauses.clear(host);
        }
        Ok(answer)
    }
}

/// The pin check of `code: url`, for GitHub and Zenodo links.
async fn pin(
    state: &AppState,
    url: &str,
    reach: Option<&Reach>,
    account: Option<&str>,
) -> PinOutcome {
    if code_pin(url) == Pin::CheckedOnline {
        let Some(record) = zenodo_record(url) else {
            return PinOutcome {
                pin: Some(PinCheck::NotVerified(
                    "it names no Zenodo record the server can look up".into(),
                )),
                reach: Some(Reach::Dead("names no Zenodo record".into())),
            };
        };
        let (record_reach, pin) = zenodo_pin(state, record).await;
        return PinOutcome {
            pin: Some(pin),
            reach: Some(record_reach),
        };
    }
    let Some(commit) = github_commit(url) else {
        return PinOutcome::default();
    };
    let pin = if let Some(Reach::Dead(_)) = reach {
        PinCheck::NotVerified(format!(
            "the link is dead, so GitHub's API can't say whether commit {} is on a branch of {}",
            commit.sha, commit.repo
        ))
    } else {
        github::github_pin(state, &commit, account).await
    };
    PinOutcome {
        pin: Some(pin),
        reach: None,
    }
}

/// Whether Zenodo's `record` answers, and whether it is a version record. A concept
/// record redirects to its latest version, which a later upload changes.
async fn zenodo_pin(state: &AppState, record: u64) -> (Reach, PinCheck) {
    let url = format!("https://zenodo.org/api/records/{record}");
    let answer = match state.link_checker.fetch_from("zenodo.org", &url).await {
        Ok(answer) => answer,
        Err(no_answer) => {
            let pin = match &no_answer {
                NoAnswer::Waiting => no_answer.why("zenodo.org"),
                NoAnswer::Silent(why) => format!("Zenodo didn't answer ({why})"),
            };
            return (
                Reach::RetryLater(no_answer.why("zenodo.org")),
                PinCheck::NotVerified(pin),
            );
        }
    };
    let status = answer.status;
    match status {
        200..=299 => (Reach::Live, PinCheck::Verified),
        300..=399 => {
            let latest = answer
                .location
                .as_deref()
                .and_then(zenodo_record)
                .map_or(String::new(), |latest| {
                    format!(", such as https://zenodo.org/records/{latest}")
                });
            (
                Reach::Live,
                PinCheck::Failed(format!(
                    "record {record} is Zenodo's concept record, which moves to each new version; link the version record that ran{latest}"
                )),
            )
        }
        408 | 429 | 500..=599 => (
            Reach::RetryLater(format!("it answered {status}")),
            PinCheck::NotVerified(format!("Zenodo answered {status} for record {record}")),
        ),
        _ => (
            Reach::Dead(format!(
                "names a record Zenodo doesn't have (it answered {status})"
            )),
            PinCheck::NotVerified(format!("Zenodo answered {status} for record {record}")),
        ),
    }
}

/// The reasons a submission is refused: a pin the check found bad.
pub fn refusals(links: &[CheckedLink]) -> Vec<String> {
    links
        .iter()
        .filter_map(|link| match &link.pin {
            Some(PinCheck::Failed(why)) => Some(format!("`code: {}`: {why}", one_line(&link.url))),
            _ => None,
        })
        .collect()
}

/// The warnings for the author.
pub fn warnings(links: &[CheckedLink]) -> Vec<String> {
    let mut warnings = Vec::new();
    for link in links {
        let field = link.field.name();
        let url = one_line(&link.url);
        match &link.reach {
            Reach::Live => {}
            Reach::Dead(why) => warnings.push(format!(
                "`{field}: {url}` {why}, so the card says \"{field} link dead\"; the server checks it again each week"
            )),
            Reach::RetryLater(why) => warnings.push(format!(
                "the server couldn't check `{field}: {url}`: {why}; it tries again an hour after the check, then less often"
            )),
        }
        match &link.pin {
            None | Some(PinCheck::Verified) => {}
            Some(PinCheck::NotVerified(why)) => warnings.push(format!(
                "the server couldn't confirm the pin of `{field}: {url}`: {why}; the card says \"pin not verified\" until a recheck confirms it"
            )),
            Some(PinCheck::Failed(why)) => warnings.push(format!(
                "`{field}: {url}`: {why}; the card says \"code pin failed\""
            )),
            Some(PinCheck::OnNoBranch(why)) => warnings.push(format!(
                "`{field}: {url}`: {why}; the card says \"commit on no branch\""
            )),
        }
    }
    warnings
}

/// The card's flags from the links.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkFlags {
    pub code_link_dead: bool,
    pub logs_link_dead: bool,
    pub pin: Option<PinFlag>,
}

impl PinCheck {
    /// What the card flags for this pin state.
    fn flag(&self) -> Option<PinFlag> {
        match self {
            PinCheck::Verified => None,
            PinCheck::NotVerified(_) => Some(PinFlag::NotVerified),
            PinCheck::Failed(_) => Some(PinFlag::Failed),
            PinCheck::OnNoBranch(_) => Some(PinFlag::OnNoBranch),
        }
    }
}

impl LinkFlags {
    pub fn of(links: &[CheckedLink]) -> LinkFlags {
        let dead = |field| {
            links
                .iter()
                .any(|link| link.field == field && matches!(link.reach, Reach::Dead(_)))
        };
        LinkFlags {
            code_link_dead: dead(Field::Code),
            logs_link_dead: dead(Field::Logs),
            pin: links
                .iter()
                .find_map(|link| link.pin.as_ref().and_then(PinCheck::flag)),
        }
    }

    /// `context` with these flags.
    pub fn on(self, context: CardContext) -> CardContext {
        CardContext {
            code_link_dead: self.code_link_dead,
            logs_link_dead: self.logs_link_dead,
            pin: self.pin,
            ..context
        }
    }
}
