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

/// A new series join key: `fdj_` and 52 characters, 260 bits.
pub fn new_join_key() -> String {
    format!("fdj_{}", random_text(52))
}

/// A new invite code: `inv-` and 16 characters, 80 bits, short enough to read out.
pub fn new_invite_code() -> String {
    format!("inv-{}", random_text(16))
}

/// A new secret for a session cookie or an email link: 52 characters, 260 bits.
pub fn new_secret() -> String {
    random_text(52)
}

/// A new claim code, which an agent shows its human and the human types: 8
/// characters, 40 bits, as `xxxx-xxxx`.
pub fn new_claim_code() -> String {
    let text = random_text(8);
    format!("{}-{}", &text[..4], &text[4..])
}

/// The letters a device's user code is made of: RFC 8628's base 20 (section 6.1),
/// capitals with no vowels, so a code types without a modifier key and spells no
/// word, and holds nothing that reads as a digit.
pub const USER_CODE_LETTERS: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";

/// A new user code, which an agent shows its human and the human types on
/// `/device`: 8 letters of [`USER_CODE_LETTERS`], about 34.5 bits, as `XXXX-XXXX`.
pub fn new_user_code() -> String {
    let mut letters = String::with_capacity(9);
    while letters.len() < 9 {
        let mut byte = [0u8; 1];
        rand::fill(&mut byte[..]);
        // 240 is the largest multiple of 20 a byte holds, so each letter is as likely.
        if byte[0] >= 240 {
            continue;
        }
        if letters.len() == 4 {
            letters.push('-');
        }
        letters.push(char::from(USER_CODE_LETTERS[usize::from(byte[0] % 20)]));
    }
    letters
}

/// A new device code, the secret an agent polls with: `fdd_` and 52 characters,
/// 260 bits.
pub fn new_device_code() -> String {
    format!("fdd_{}", random_text(52))
}

/// What the database keeps of a token or secret.
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}
