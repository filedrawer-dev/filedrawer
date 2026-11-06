//! Long runs of encoded text, which the server refuses rather than screening their
//! decoding (the owner's decision, 2026-11-06; design.md, Submission and screening,
//! step 3). A run is refused only when it decodes to readable text, or to a
//! compressed stream, since that is what hides a payload from a screen that reads
//! text. Hashes, keys, IDs and identifiers decode to noise and pass: a false refusal
//! turns away an honest author.

use std::fmt;

use crate::report::Refusal;

/// An encoding the server refuses long runs of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Base64,
    Base32,
    Hex,
    UrlEncoding,
    Base85,
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Encoding::Base64 => "base64",
            Encoding::Base32 => "base32",
            Encoding::Hex => "hex",
            Encoding::UrlEncoding => "URL encoding",
            Encoding::Base85 => "base85",
        })
    }
}

/// The shortest decoded text that counts: shorter runs can't hide much.
const MIN_DECODED: usize = 24;
/// The fewest `%XX` escapes in one token, outside a link, that are refused.
const MIN_ESCAPES: usize = 6;
/// The shortest line that may be part of a blob wrapped over several lines.
const MIN_WRAPPED_LINE: usize = 16;
/// The shortest `<~ ... ~>` token refused as base85.
const MIN_BASE85: usize = 24;
/// How many characters of a refused run its reason shows.
const SHOWN: usize = 16;

/// A refusal for each kind of long encoded run in `text`, the first of each kind.
pub fn refusals(text: &str) -> Vec<Refusal> {
    let text = without_invisible(text);
    let mut found: Vec<Refusal> = Vec::new();
    let mut add = |kind: Encoding, run: &str| {
        let seen = found
            .iter()
            .any(|refusal| matches!(refusal, Refusal::EncodedText { kind: k, .. } if *k == kind));
        if !seen {
            found.push(Refusal::EncodedText {
                kind,
                start: run.chars().take(SHOWN).collect(),
            });
        }
    };
    let mut candidates: Vec<String> = text
        .split_whitespace()
        .flat_map(|token| runs(token, is_base64_char).map(str::to_string))
        .collect();
    candidates.extend(wrapped_blobs(&text));
    for run in &candidates {
        if hides_text(run, decode_base64) {
            add(Encoding::Base64, run);
        }
    }
    for token in text.split_whitespace() {
        for run in runs(token, is_base32_char) {
            if run.chars().any(|c| c.is_ascii_alphabetic()) && hides_text(run, decode_base32) {
                add(Encoding::Base32, run);
            }
        }
        for run in runs(token, |c| c.is_ascii_hexdigit()) {
            if run.chars().any(|c| c.is_ascii_alphabetic()) && hides_text(run, decode_hex) {
                add(Encoding::Hex, run);
            }
        }
        if !is_link(token) && escaped_ascii(token) >= MIN_ESCAPES {
            add(Encoding::UrlEncoding, token);
        }
        let bare = token.trim_end_matches(['.', ',', ';', ')', '`', '"']);
        let bare = bare.trim_start_matches(['(', '`', '"']);
        if bare.len() >= MIN_BASE85 && bare.starts_with("<~") && bare.ends_with("~>") {
            add(Encoding::Base85, bare);
        }
    }
    found
}

/// `text` without zero-width and other invisible format characters, which could
/// split a run without showing.
fn without_invisible(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{ad}'))
        .collect()
}

/// Blobs wrapped over consecutive lines that hold nothing else, joined.
fn wrapped_blobs(text: &str) -> Vec<String> {
    let mut blobs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut flush = |current: &mut Vec<&str>| {
        if current.len() >= 2 {
            blobs.push(current.concat());
        }
        current.clear();
    };
    for line in text.lines().map(str::trim) {
        if line.len() >= MIN_WRAPPED_LINE && line.chars().all(is_base64_char) {
            current.push(line);
        } else {
            flush(&mut current);
        }
    }
    flush(&mut current);
    blobs
}

/// A link, whose escapes are the site's business: one with a scheme, or a bare host
/// followed by a path or query, such as `huggingface.co/datasets?search=...`.
fn is_link(token: &str) -> bool {
    let token = token.trim_start_matches(['(', '[', '<', '`', '"']);
    if token.contains("://") {
        return true;
    }
    let host = token.split(['/', '?']).next().unwrap_or("");
    token.len() > host.len()
        && host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// How many `%XX` escapes in `token` stand for printable ASCII: escaped non-ASCII
/// text, such as a Chinese query, is honest.
fn escaped_ascii(token: &str) -> usize {
    let bytes = token.as_bytes();
    (0..bytes.len().saturating_sub(2))
        .filter(|&i| bytes[i] == b'%')
        .filter_map(|i| u8::from_str_radix(token.get(i + 1..i + 3)?, 16).ok())
        .filter(|byte| (0x20..0x7f).contains(byte))
        .count()
}

/// The maximal runs of characters in `token` that `keep` accepts.
fn runs(token: &str, keep: impl Fn(char) -> bool) -> impl Iterator<Item = &str> {
    token.split(move |c| !keep(c)).filter(|run| !run.is_empty())
}

fn is_base64_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_')
}

fn is_base32_char(c: char) -> bool {
    c.is_ascii_uppercase() || ('2'..='7').contains(&c) || c == '='
}

/// Whether `run` is a compressed stream, or, decoded by `decode` from any of its
/// first few characters (so a prefix such as `sha512-` can't misalign it), readable
/// text.
fn hides_text(run: &str, decode: fn(&str) -> Vec<u8>) -> bool {
    let run = run.trim_matches('=');
    is_compressed(&decode(run))
        || (0..8.min(run.len())).any(|offset| is_text(&decode(&run[offset..])))
}

/// Whether `bytes` start like gzip, bzip2, xz or zip. Only signatures of three
/// bytes or more, checked only at the start: zlib's two-byte header turns up in
/// about one random hash in a few thousand.
fn is_compressed(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b, 0x08])
        || (bytes.starts_with(b"BZh") && bytes.get(3).is_some_and(|b| (b'1'..=b'9').contains(b)))
        || bytes.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0])
        || bytes.starts_with(b"PK\x03\x04")
}

/// Whether `bytes` read as text: UTF-8 (a stray last byte allowed), long enough,
/// and almost free of control characters.
fn is_text(bytes: &[u8]) -> bool {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) if error.valid_up_to() + 3 >= bytes.len() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return false,
    };
    let count = text.chars().count();
    let control = text
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        .count();
    count >= MIN_DECODED && control * 20 <= count
}

/// Base64, standard or URL-safe or both, ignoring padding and stray characters.
fn decode_base64(run: &str) -> Vec<u8> {
    let values = run.chars().filter_map(|c| match c {
        'A'..='Z' => Some(c as u32 - 'A' as u32),
        'a'..='z' => Some(c as u32 - 'a' as u32 + 26),
        '0'..='9' => Some(c as u32 - '0' as u32 + 52),
        '+' | '-' => Some(62),
        '/' | '_' => Some(63),
        _ => None,
    });
    pack(values, 6)
}

fn decode_base32(run: &str) -> Vec<u8> {
    let values = run.chars().filter_map(|c| match c {
        'A'..='Z' => Some(c as u32 - 'A' as u32),
        '2'..='7' => Some(c as u32 - '2' as u32 + 26),
        _ => None,
    });
    pack(values, 5)
}

fn decode_hex(run: &str) -> Vec<u8> {
    pack(run.chars().filter_map(|c| c.to_digit(16)), 4)
}

/// Packs `bits`-wide values into bytes, dropping a partial last byte.
fn pack(values: impl Iterator<Item = u32>, bits: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let (mut buffer, mut held) = (0u32, 0u32);
    for value in values {
        buffer = (buffer << bits) | value;
        held += bits;
        if held >= 8 {
            held -= 8;
            bytes.push((buffer >> held) as u8);
            buffer &= (1 << held) - 1;
        }
    }
    bytes
}
