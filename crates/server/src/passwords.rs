//! Password hashing with argon2id (design.md, Accounts).

use std::sync::LazyLock;

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

/// The PHC string for `password`, with a random salt and the crate's argon2id
/// defaults (19 MiB, 2 passes, 1 lane).
pub fn hash(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("argon2id with default parameters hashes any input")
        .to_string()
}

/// Whether `password` matches `stored`. With no stored hash it checks a decoy and
/// returns false, taking the same time.
pub fn matches(password: &str, stored: Option<&str>) -> bool {
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
