//! A fake `Fetcher`: the web as a test sets it up, recording every fetch, so no test
//! reaches a real host.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use filedrawer_server::link_checks::{
    Answer, ApiAnswer, Fetcher, Fetching, JsonAnswer, LinkChecker,
};
use serde_json::{Value, json};

/// Every page answers 200 and every GitHub commit is on its repository's default
/// branch, unless a test says otherwise.
#[derive(Clone, Default)]
pub struct FakeWeb {
    web: Arc<Mutex<Web>>,
}

#[derive(Default)]
struct Web {
    pages: HashMap<String, Result<Answer, String>>,
    api: HashMap<String, Result<ApiAnswer, String>>,
    fetched: Vec<String>,
    api_calls: Vec<String>,
    /// Pages that answer only once `release` lets them.
    gated: HashMap<String, Arc<tokio::sync::Semaphore>>,
    json: HashMap<String, Result<JsonAnswer, String>>,
    json_fetched: Vec<String>,
}

impl FakeWeb {
    /// A checker on this web, with GitHub's hourly allowance of API calls.
    pub fn checker(&self, github_hourly: u32) -> Arc<LinkChecker> {
        Arc::new(LinkChecker::new(Arc::new(self.clone()), github_hourly))
    }

    fn set_page(&self, url: &str, answer: Result<Answer, String>) {
        self.web.lock().unwrap().pages.insert(url.into(), answer);
    }

    /// `url` answers `status`.
    pub fn answer(&self, url: &str, status: u16) {
        self.set_page(
            url,
            Ok(Answer {
                status,
                location: None,
                retry_after: None,
            }),
        );
    }

    /// `url` answers `status` with `Retry-After: seconds`.
    pub fn answer_after(&self, url: &str, status: u16, seconds: u64) {
        self.set_page(
            url,
            Ok(Answer {
                status,
                location: None,
                retry_after: Some(seconds),
            }),
        );
    }

    /// `url` answers `status` with `location`.
    pub fn redirect(&self, url: &str, status: u16, location: &str) {
        self.set_page(
            url,
            Ok(Answer {
                status,
                location: Some(location.into()),
                retry_after: None,
            }),
        );
    }

    /// `url` answers only after `release(url)`, so a test can hold a fetch in
    /// flight.
    pub fn hold(&self, url: &str) {
        self.web
            .lock()
            .unwrap()
            .gated
            .insert(url.into(), Arc::new(tokio::sync::Semaphore::new(0)));
    }

    /// Lets a held `url` answer.
    pub fn release(&self, url: &str) {
        if let Some(gate) = self.web.lock().unwrap().gated.get(url) {
            gate.add_permits(1);
        }
    }

    /// `url` doesn't answer.
    pub fn fail(&self, url: &str) {
        self.set_page(url, Err("timed out".into()));
    }

    /// GitHub's API answers `path` with `status` and `body`, and says `remaining`
    /// calls are left.
    pub fn api(&self, path: &str, status: u16, body: Value, remaining: u32) {
        self.api_after(path, status, body, remaining, None);
    }

    /// `api`, with `Retry-After: seconds` when given.
    pub fn api_after(
        &self,
        path: &str,
        status: u16,
        body: Value,
        remaining: u32,
        seconds: Option<u64>,
    ) {
        self.web.lock().unwrap().api.insert(
            path.into(),
            Ok(ApiAnswer {
                status,
                body: Some(body),
                remaining: Some(remaining),
                reset: Some(i64::MAX),
                retry_after: seconds,
            }),
        );
    }

    /// GitHub's API doesn't answer `path`.
    pub fn api_fails(&self, path: &str) {
        self.web
            .lock()
            .unwrap()
            .api
            .insert(path.into(), Err("timed out".into()));
    }

    /// `url` answers a request for JSON with `status` and `body`, or with text that
    /// isn't JSON when `body` is `None`.
    pub fn json(&self, url: &str, status: u16, body: Option<Value>) {
        self.web.lock().unwrap().json.insert(
            url.into(),
            Ok(JsonAnswer {
                status,
                body,
                retry_after: None,
            }),
        );
    }

    /// `url` answers a request for JSON with `status` and `Retry-After: seconds`.
    pub fn json_after(&self, url: &str, status: u16, seconds: u64) {
        self.web.lock().unwrap().json.insert(
            url.into(),
            Ok(JsonAnswer {
                status,
                body: None,
                retry_after: Some(seconds),
            }),
        );
    }

    /// A request for JSON from `url` doesn't answer.
    pub fn json_fails(&self, url: &str) {
        self.web
            .lock()
            .unwrap()
            .json
            .insert(url.into(), Err("timed out".into()));
    }

    /// Every URL asked for JSON, in order.
    pub fn json_fetched(&self) -> Vec<String> {
        self.web.lock().unwrap().json_fetched.clone()
    }

    /// Every URL fetched, in order.
    pub fn fetched(&self) -> Vec<String> {
        self.web.lock().unwrap().fetched.clone()
    }

    /// Every GitHub API path asked, in order.
    pub fn api_calls(&self) -> Vec<String> {
        self.web.lock().unwrap().api_calls.clone()
    }
}

impl Fetcher for FakeWeb {
    fn get<'a>(&'a self, url: &'a str) -> Fetching<'a, Answer> {
        Box::pin(async move {
            let gate = {
                let mut web = self.web.lock().unwrap();
                web.fetched.push(url.into());
                web.gated.get(url).cloned()
            };
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            let web = self.web.lock().unwrap();
            web.pages.get(url).cloned().unwrap_or(Ok(Answer {
                status: 200,
                location: None,
                retry_after: None,
            }))
        })
    }

    /// A page asked for JSON answers 404 unless a test says otherwise, so the fork
    /// check falls back to GitHub's API.
    fn get_json<'a>(&'a self, url: &'a str) -> Fetching<'a, JsonAnswer> {
        Box::pin(async move {
            let mut web = self.web.lock().unwrap();
            web.json_fetched.push(url.into());
            web.json.get(url).cloned().unwrap_or(Ok(JsonAnswer {
                status: 404,
                body: None,
                retry_after: None,
            }))
        })
    }

    fn github<'a>(&'a self, path: &'a str) -> Fetching<'a, ApiAnswer> {
        Box::pin(async move {
            let mut web = self.web.lock().unwrap();
            web.api_calls.push(path.into());
            if let Some(answer) = web.api.get(path) {
                return answer.clone();
            }
            let body = if path.contains("/compare/") {
                json!({"status": "identical", "base_commit": {"sha": "f".repeat(40)}})
            } else {
                json!([])
            };
            Ok(ApiAnswer {
                status: 200,
                body: Some(body),
                remaining: Some(4999),
                reset: Some(i64::MAX),
                retry_after: None,
            })
        })
    }
}
