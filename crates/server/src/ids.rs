//! Random IDs and API tokens.

use sha2::{Digest, Sha256};

/// Crockford's base32 in lowercase: no i, l, o or u, so IDs are easy to copy.
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// `length` random characters from [`ALPHABET`].
fn random_text(length: usize) -> String {
    let mut bytes = vec![0u8; length];
    rand::fill(&mut bytes[..]);
    bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte % 32)]))
        .collect()
}

/// A new entry, token or feedback ID: 10 characters, 50 bits.
pub fn new_id() -> String {
    random_text(10)
}

/// A new account ID, an ID with an `a-` prefix.
pub fn new_account_id() -> String {
    format!("a-{}", new_id())
}

/// A new API token: `fdt_` and 52 characters, 260 bits.
pub fn new_token() -> String {
    format!("fdt_{}", random_text(52))
}

/// What the database keeps of a token.
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}
