//! How the link checks reach the web: the `Fetcher` seam, and `HttpFetcher`, its
//! real client, which never follows a redirect by itself.

use std::future::Future;
use std::time::Duration;

/// A fetch under way.
pub type Fetching<'a, T> = std::pin::Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// What a GET answered, its body unread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub location: Option<String>,
}

/// What GitHub's REST API answered.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiAnswer {
    pub status: u16,
    pub body: Option<serde_json::Value>,
    /// `x-ratelimit-remaining`: the calls left this hour.
    pub remaining: Option<u32>,
    /// `x-ratelimit-reset`: when the hour's calls come back, in Unix seconds.
    pub reset: Option<i64>,
}

/// How the server reaches the web, so tests can stand in for it.
pub trait Fetcher: Send + Sync {
    /// GETs `url` without following a redirect and without reading the body.
    fn get<'a>(&'a self, url: &'a str) -> Fetching<'a, Answer>;
    /// GETs `path`, such as `/repos/o/r/branches`, from GitHub's REST API, with the
    /// server's token if it has one.
    fn github<'a>(&'a self, path: &'a str) -> Fetching<'a, ApiAnswer>;
}

/// GitHub's hourly allowance of API calls with a token, and without one, per IP.
pub const GITHUB_HOURLY_WITH_TOKEN: u32 = 5000;
pub const GITHUB_HOURLY_WITHOUT_TOKEN: u32 = 60;

/// How long one fetch may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The most of an API answer the server reads. A compare lists at most 300 files.
const MAX_API_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Who the hosts see fetching.
const USER_AGENT: &str = "filedrawer.dev link check (https://filedrawer.dev/skill.md)";

/// Fetches from the real hosts. Neither client follows a redirect by itself: the
/// checks follow each hop only when it stays on a listed host.
pub struct HttpFetcher {
    http: reqwest::Client,
    api_base: String,
    token: Option<String>,
}

impl HttpFetcher {
    /// With `token`, GitHub's API allows 5,000 calls an hour instead of 60.
    pub fn new(token: Option<String>) -> HttpFetcher {
        HttpFetcher::with_api_base(token, "https://api.github.com", FETCH_TIMEOUT)
    }

    /// With `api_base` standing in for `https://api.github.com`, and each fetch
    /// given up after `timeout`, for tests.
    pub fn with_api_base(token: Option<String>, api_base: &str, timeout: Duration) -> HttpFetcher {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .user_agent(USER_AGENT)
            .build()
            .expect("a client with only a redirect policy, a timeout and an agent builds");
        HttpFetcher {
            http,
            api_base: api_base.trim_end_matches('/').to_string(),
            token,
        }
    }
}

/// A fetch's failure, for the author: "timed out", or the error without its URL.
fn fetch_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".into()
    } else {
        error.without_url().to_string()
    }
}

fn header_number<T: std::str::FromStr>(
    headers: &reqwest::header::HeaderMap,
    name: &str,
) -> Option<T> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

impl Fetcher for HttpFetcher {
    fn get<'a>(&'a self, url: &'a str) -> Fetching<'a, Answer> {
        Box::pin(async move {
            let response = self.http.get(url).send().await.map_err(fetch_error)?;
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            // Dropped unread: only the status matters.
            Ok(Answer {
                status: response.status().as_u16(),
                location,
            })
        })
    }

    fn github<'a>(&'a self, path: &'a str) -> Fetching<'a, ApiAnswer> {
        Box::pin(async move {
            let mut request = self
                .http
                .get(format!("{}{path}", self.api_base))
                .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                .header("x-github-api-version", "2022-11-28");
            if let Some(token) = &self.token {
                request = request.bearer_auth(token);
            }
            let mut response = request.send().await.map_err(fetch_error)?;
            let status = response.status().as_u16();
            let remaining = header_number(response.headers(), "x-ratelimit-remaining");
            let reset = header_number(response.headers(), "x-ratelimit-reset");
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(fetch_error)? {
                if bytes.len() + chunk.len() > MAX_API_BODY_BYTES {
                    return Err("GitHub's answer was too long to read".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(ApiAnswer {
                status,
                body: serde_json::from_slice(&bytes).ok(),
                remaining,
                reset,
            })
        })
    }
}
