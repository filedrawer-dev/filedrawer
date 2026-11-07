//! Hosts that asked the server to wait (design.md, Submission and screening, step 2):
//! a 429, a 503 with `Retry-After`, or GitHub's API's secondary rate limit. Until the
//! wait is over, the server sends that host nothing. Kept in memory: a restart
//! forgets them.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The wait when a host gives no `Retry-After`: a minute, as GitHub asks, doubling
/// with each such answer in a row, at most an hour.
const FIRST_WAIT: Duration = Duration::from_secs(60);
const LONGEST_WAIT: Duration = Duration::from_secs(60 * 60);

/// The longest `Retry-After` the server honours: a day.
const LONGEST_RETRY_AFTER_SECS: u64 = 24 * 60 * 60;

#[derive(Default)]
pub(super) struct Pauses {
    hosts: Mutex<HashMap<String, Pause>>,
}

struct Pause {
    until: Instant,
    /// Answers in a row that asked the server to wait.
    strikes: u32,
}

impl Pauses {
    fn hosts(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pause>> {
        self.hosts.lock().expect("no thread panics holding it")
    }

    /// Whether `host` asked the server to wait, and the wait isn't over.
    pub(super) fn holds(&self, host: &str) -> bool {
        self.hosts()
            .get(host)
            .is_some_and(|pause| Instant::now() < pause.until)
    }

    /// `host` asked the server to wait: `retry_after` seconds, or without it a
    /// minute, doubling with each such answer in a row. A pause only grows: a
    /// shorter wait doesn't cut a longer one short.
    pub(super) fn pause(&self, host: &str, retry_after: Option<u64>) {
        let mut hosts = self.hosts();
        let earlier = hosts.get(host);
        let strikes = earlier.map_or(0, |pause| pause.strikes) + 1;
        let wait = match retry_after {
            Some(seconds) => Duration::from_secs(seconds.min(LONGEST_RETRY_AFTER_SECS)),
            None => FIRST_WAIT
                .saturating_mul(2u32.saturating_pow(strikes - 1))
                .min(LONGEST_WAIT),
        };
        let until = earlier.map_or(Instant::now() + wait, |pause| {
            pause.until.max(Instant::now() + wait)
        });
        hosts.insert(host.to_string(), Pause { until, strikes });
    }

    /// The hosts asking the server to wait now, each with how long is left, by name.
    pub(super) fn active(&self) -> Vec<(String, Duration)> {
        let now = Instant::now();
        let mut active: Vec<(String, Duration)> = self
            .hosts()
            .iter()
            .filter(|(_, pause)| now < pause.until)
            .map(|(host, pause)| (host.clone(), pause.until - now))
            .collect();
        active.sort();
        active
    }

    /// `host` answered without asking the server to wait, so its strikes are over.
    /// The pause stands: the answer may be to a fetch sent before the host asked.
    pub(super) fn clear(&self, host: &str) {
        if let Some(pause) = self.hosts().get_mut(host) {
            pause.strikes = 0;
        }
    }
}
