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
/// The shortest base64 run tried as a zlib stream: one that inflates to
/// [`MIN_DECODED`] characters is rarely shorter.
const MIN_ZLIB_RUN: usize = 24;
/// The shortest `<~ ... ~>` token refused as base85.
const MIN_BASE85: usize = 24;
/// How many characters of a refused run its reason shows.
const SHOWN: usize = 16;

/// A long run of encoded text that decodes to readable text or starts a compressed
/// stream (a `<~ ... ~>` base85 token or a URL-escaped token counts whatever it
/// decodes to).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub kind: Encoding,
    /// The run's first characters, as a refusal shows them.
    pub start: String,
    /// What the run decodes to.
    pub bytes: Vec<u8>,
}

/// A refusal for each kind of long encoded run in `text`, the first of each kind.
pub fn refusals(text: &str) -> Vec<Refusal> {
    let mut found: Vec<Refusal> = Vec::new();
    for Decoded { kind, start, .. } in decodings(text) {
        let seen = found
            .iter()
            .any(|refusal| matches!(refusal, Refusal::EncodedText { kind: k, .. } if *k == kind));
        if !seen {
            found.push(Refusal::EncodedText { kind, start });
        }
    }
    found
}

/// A compressed stream, as its signature names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Gzip,
    Bzip2,
    Xz,
    Zip,
}

/// The compressed stream `bytes` start, by a signature of three bytes or more,
/// checked only at the start: zlib's two-byte header turns up in about one random
/// hash in a few thousand.
pub fn compression(bytes: &[u8]) -> Option<Compression> {
    if bytes.starts_with(&[0x1f, 0x8b, 0x08]) {
        Some(Compression::Gzip)
    } else if bytes.starts_with(b"BZh") && bytes.get(3).is_some_and(|b| (b'1'..=b'9').contains(b)) {
        Some(Compression::Bzip2)
    } else if bytes.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        Some(Compression::Xz)
    } else if bytes.starts_with(b"PK\x03\x04") {
        Some(Compression::Zip)
    } else {
        None
    }
}

/// Like [`decodings`], with base64 runs whose bytes start with a zlib header too.
/// A random hash starts with one about once in a few thousand, so only a caller
/// that then inflates the stream to see what it holds should ask.
pub fn decodings_with_zlib(text: &str) -> Vec<Decoded> {
    find(text, true)
}

/// Every long encoded run in `text` that hides text or a compressed stream, decoded,
/// base64 runs first.
pub fn decodings(text: &str) -> Vec<Decoded> {
    find(text, false)
}

fn find(text: &str, with_zlib: bool) -> Vec<Decoded> {
    let text = without_invisible(text);
    let mut found = Vec::new();
    let mut add = |kind: Encoding, run: &str, bytes: Vec<u8>| {
        found.push(Decoded {
            kind,
            start: run.chars().take(SHOWN).collect(),
            bytes,
        });
    };
    let decode = |run: &str, decode: fn(&str) -> Vec<u8>| {
        hidden(run, decode).or_else(|| with_zlib.then(|| zlib_stream(run, decode)).flatten())
    };
    // A blob wrapped over lines is decoded whole, and its lines then not one by one,
    // so the screen reads it once.
    let mut wrapped = Vec::new();
    let mut wrapped_lines = Vec::new();
    for (blob, lines) in wrapped_blobs(&text) {
        if let Some(bytes) = decode(&blob, decode_base64) {
            wrapped.push((blob, bytes));
            wrapped_lines.extend(lines);
        }
    }
    for token in text.split_whitespace() {
        if wrapped_lines.contains(&token) {
            continue;
        }
        for run in runs(token, is_base64_char) {
            if let Some(bytes) = decode(run, decode_base64) {
                add(Encoding::Base64, run, bytes);
            }
        }
    }
    for (blob, bytes) in wrapped {
        add(Encoding::Base64, &blob, bytes);
    }
    for token in text.split_whitespace() {
        for run in runs(token, is_base32_char) {
            if run.chars().any(|c| c.is_ascii_alphabetic())
                && let Some(bytes) = decode(run, decode_base32)
            {
                add(Encoding::Base32, run, bytes);
            }
        }
        for run in runs(token, |c| c.is_ascii_hexdigit()) {
            if run.chars().any(|c| c.is_ascii_alphabetic())
                && let Some(bytes) = decode(run, decode_hex)
            {
                add(Encoding::Hex, run, bytes);
            }
        }
        if !is_link(token) && escaped_ascii(token) >= MIN_ESCAPES {
            add(Encoding::UrlEncoding, token, decode_url(token));
        }
        let bare = token.trim_end_matches(['.', ',', ';', ')', '`', '"']);
        let bare = bare.trim_start_matches(['(', '`', '"']);
        if bare.len() >= MIN_BASE85 && bare.starts_with("<~") && bare.ends_with("~>") {
            add(
                Encoding::Base85,
                bare,
                decode_base85(&bare[2..bare.len() - 2]),
            );
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

/// Blobs wrapped over consecutive lines that hold nothing else, joined, each with
/// its lines.
fn wrapped_blobs(text: &str) -> Vec<(String, Vec<&str>)> {
    let mut blobs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.len() >= MIN_WRAPPED_LINE && line.chars().all(is_base64_char) {
            current.push(line);
            continue;
        }
        let lines = std::mem::take(&mut current);
        if lines.len() >= 2 {
            blobs.push((lines.concat(), lines));
        }
    }
    if current.len() >= 2 {
        blobs.push((current.concat(), current));
    }
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

/// What `run` decodes to by `decode`, when that is a compressed stream, or, decoded
/// from any of its first few characters (so a prefix such as `sha512-` can't
/// misalign it), readable text.
fn hidden(run: &str, decode: fn(&str) -> Vec<u8>) -> Option<Vec<u8>> {
    let run = run.trim_matches('=');
    let whole = decode(run);
    if compression(&whole).is_some() {
        return Some(whole);
    }
    (0..8.min(run.len()))
        .map(|offset| decode(&run[offset..]))
        .find(|bytes| is_text(bytes))
}

/// What `run` decodes to by `decode`, when that starts with a zlib header and the
/// run is long enough to hold a stream worth inflating.
fn zlib_stream(run: &str, decode: fn(&str) -> Vec<u8>) -> Option<Vec<u8>> {
    let bytes = decode(run.trim_matches('='));
    (run.len() >= MIN_ZLIB_RUN && starts_zlib(&bytes)).then_some(bytes)
}

/// Whether `bytes` start with a zlib header for a deflate stream with no preset
/// dictionary.
pub fn starts_zlib(bytes: &[u8]) -> bool {
    match bytes {
        [cmf, flg, ..] => {
            cmf & 0x0f == 8
                && cmf >> 4 <= 7
                && flg & 0x20 == 0
                && (u16::from(*cmf) * 256 + u16::from(*flg)) % 31 == 0
        }
        _ => false,
    }
}

/// Whether `bytes` read as text long enough to hide something.
fn is_text(bytes: &[u8]) -> bool {
    text_chars(bytes).is_some_and(|count| count >= MIN_DECODED)
}

/// Whether `bytes` read as text of any length, as what a stream decompresses to
/// may: a stream that inflates is no accident, so it needs no minimum.
pub fn reads_as_text(bytes: &[u8]) -> bool {
    text_chars(bytes).is_some_and(|count| count > 0)
}

/// How many characters `bytes` hold when they read as text: UTF-8 (a stray last
/// character cut short allowed) and almost free of control characters.
fn text_chars(bytes: &[u8]) -> Option<usize> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) if error.valid_up_to() + 3 >= bytes.len() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return None,
    };
    let count = text.chars().count();
    let control = text
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        .count();
    (control * 20 <= count).then_some(count)
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

/// `%XX` escapes decoded; everything else as it stands.
fn decode_url(token: &str) -> Vec<u8> {
    let bytes = token.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| token.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

/// Ascii85 (Adobe's, between `<~` and `~>`): groups of five characters from `!` to
/// `u` make four bytes, `z` four zero bytes, and a short last group fewer.
fn decode_base85(inner: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut group: Vec<u32> = Vec::with_capacity(5);
    let flush = |group: &mut Vec<u32>, out: &mut Vec<u8>| {
        if group.len() < 2 {
            group.clear();
            return;
        }
        let kept = group.len() - 1;
        group.resize(5, 84);
        let value = group
            .iter()
            .fold(0u64, |value, digit| value * 85 + u64::from(*digit));
        let value = u32::try_from(value).unwrap_or(u32::MAX);
        out.extend_from_slice(&value.to_be_bytes()[..kept]);
        group.clear();
    };
    for c in inner.chars() {
        match c {
            'z' if group.is_empty() => out.extend_from_slice(&[0; 4]),
            '!'..='u' => {
                group.push(c as u32 - '!' as u32);
                if group.len() == 5 {
                    flush(&mut group, &mut out);
                }
            }
            _ => {}
        }
    }
    flush(&mut group, &mut out);
    out
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
