//! Password hashing with argon2id (design.md, Accounts), a few at a time: each hash
//! fills 19 MiB and takes a core for tens of milliseconds, so a [`Hasher`] gives out
//! places, and a request that gets none in time is refused (uncertainties.md, The
//! server's own bounds).

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::AppError;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};

/// The shortest and longest password the server takes. No rules on what's in it.
pub const MIN_CHARS: usize = 12;
pub const MAX_CHARS: usize = 1024;

/// A hash of a random password nobody knows, checked when the email is unknown so a
/// login takes as long either way. Made once, on first use.
static DECOY: LazyLock<String> = LazyLock::new(|| hash(&crate::ids::new_secret()));

/// Why a password isn't accepted, or `None`.
pub fn problem(password: &str) -> Option<String> {
    let length = password.chars().count();
    if length < MIN_CHARS {
        return Some(format!("A password needs at least {MIN_CHARS} characters."));
    }
    if length > MAX_CHARS {
        return Some(format!(
            "A password can have at most {MAX_CHARS} characters."
        ));
    }
    None
}

/// The most hashes the site runs at once, however many cores it has: 76 MiB.
const MOST_AT_ONCE: usize = 4;

/// How long a request waits for a place before it is refused, and the `Retry-After`
/// of that refusal.
const SITE_WAIT: Duration = Duration::from_secs(5);

/// Runs every password hash and check, at most so many at once.
#[derive(Debug)]
pub struct Hasher {
    places: Arc<Semaphore>,
    wait: Duration,
}

/// No place came free within the hasher's wait.
#[derive(Debug, PartialEq, Eq)]
pub struct Busy {
    retry_after_secs: u64,
}

impl From<Busy> for AppError {
    fn from(busy: Busy) -> AppError {
        AppError::TooManyRequests {
            what: "logins and password changes in progress".to_string(),
            retry_after_secs: busy.retry_after_secs,
            instead: None,
        }
    }
}

/// The right to run one hash or check, which uses it up.
#[derive(Debug)]
pub struct Place {
    /// Free again when this is dropped.
    _held: OwnedSemaphorePermit,
}

impl Hasher {
    /// A hasher that runs `at_once` hashes together and lets a request wait `wait`
    /// for its place.
    pub fn new(at_once: usize, wait: Duration) -> Hasher {
        Hasher {
            places: Arc::new(Semaphore::new(at_once)),
            wait,
        }
    }

    /// A place, once one is free; `Busy` when none came free in time. A handler
    /// takes it before it counts the request against an hourly or daily limit, so a
    /// refusal here costs the client nothing.
    pub async fn place(&self) -> Result<Place, Busy> {
        let places = self.places.clone();
        match tokio::time::timeout(self.wait, places.acquire_owned()).await {
            Ok(Ok(permit)) => Ok(Place { _held: permit }),
            // The semaphore is never closed, so only the wait ends without a place.
            Ok(Err(_)) | Err(_) => Err(Busy {
                retry_after_secs: self.wait.as_secs().max(1),
            }),
        }
    }
}

impl Default for Hasher {
    /// The site's: a hash for every core but one, which stays free for requests
    /// that hash nothing, at least one and at most [`MOST_AT_ONCE`].
    fn default() -> Hasher {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        Hasher::new(cores.saturating_sub(1).clamp(1, MOST_AT_ONCE), SITE_WAIT)
    }
}

impl Place {
    /// The PHC string for `password`.
    pub async fn hash(self, password: String) -> String {
        self.run(move || hash(&password)).await
    }

    /// Whether `password` matches `stored`. With no stored hash it checks a decoy
    /// and returns false, taking the same time.
    pub async fn matches(self, password: String, stored: Option<String>) -> bool {
        self.run(move || matches(&password, stored.as_deref()))
            .await
    }

    /// Runs `work` on a blocking thread, off the runtime's own. The place goes with
    /// it and comes free when the work ends, not when the request waiting for it
    /// does: a client that hangs up mid-hash frees nothing early.
    async fn run<T: Send + 'static>(self, work: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(move || {
            let _place = self;
            work()
        })
        .await
        .expect("hashing a password doesn't panic")
    }
}

/// The PHC string for `password`, with a random salt and the crate's argon2id
/// defaults (19 MiB, 2 passes, 1 lane).
fn hash(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("argon2id with default parameters hashes any input")
        .to_string()
}

/// Whether `password` matches `stored`. With no stored hash it checks a decoy and
/// returns false, taking the same time.
fn matches(password: &str, stored: Option<&str>) -> bool {
    let (phc, real) = match stored {
        Some(stored) => (stored, true),
        None => (DECOY.as_str(), false),
    };
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    let ok = Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok();
    ok && real
}
