//! What wrote an entry's text: the skill version and User-Agent its request sent
//! (#45). The server stores them with the entry for the owner to read through SQL,
//! and shows them nowhere.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};

/// The request header a skill names itself in, as `name/N`.
pub const SKILL_HEADER: &str = "x-filedrawer-skill";

/// The longest skill name stored, and the most digits in its version.
const MAX_NAME: usize = 64;
const MAX_VERSION_DIGITS: usize = 9;

/// How much of a User-Agent is stored, in characters.
const MAX_USER_AGENT: usize = 200;

/// The skill and User-Agent a request sent, each `None` when absent or unusable.
#[derive(Debug)]
pub struct Provenance {
    pub skill: Option<String>,
    pub user_agent: Option<String>,
}

impl Provenance {
    pub fn of(headers: &HeaderMap) -> Provenance {
        Provenance {
            skill: text(headers, SKILL_HEADER)
                .filter(|value| is_skill(value))
                .map(str::to_string),
            user_agent: text(headers, header::USER_AGENT.as_str())
                .filter(|value| !value.is_empty())
                .map(|value| value.chars().take(MAX_USER_AGENT).collect()),
        }
    }
}

/// The header's first value as UTF-8 text, which a User-Agent may hold beyond ASCII.
fn text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    std::str::from_utf8(headers.get(name)?.as_bytes()).ok()
}

/// Whether `value` is `name/N`: a name of 1 to 64 lowercase letters, digits and
/// hyphens, and a version of 1 to 9 digits, with nothing around them.
fn is_skill(value: &str) -> bool {
    let Some((name, version)) = value.split_once('/') else {
        return false;
    };
    (1..=MAX_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && (1..=MAX_VERSION_DIGITS).contains(&version.len())
        && version.bytes().all(|byte| byte.is_ascii_digit())
}

impl<S: Send + Sync> FromRequestParts<S> for Provenance {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Infallible> {
        Ok(Provenance::of(&parts.headers))
    }
}
