//! The offline half of the link checks: hosts and pins by the URL's shape
//! (design.md, Submission and screening, step 2). Nothing here fetches a URL.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::fields::Fields;
use crate::report::{Refusal, Warning, WarningKind};

/// The hosts the server fetches `code` and `logs` links on, matched exactly.
pub const LISTED_HOSTS: [&str; 4] = ["github.com", "huggingface.co", "zenodo.org", "wandb.ai"];

/// How far a `code` link can be checked offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    /// A 40-character commit hash on GitHub or Hugging Face.
    Pinned,
    /// On a listed host but not at a commit: the server refuses it.
    Unpinned,
    /// A Zenodo record: the server checks online that it names a version, not a concept.
    CheckedOnline,
    /// On a host the server doesn't fetch from: the card says "pin not verified".
    HostNotListed,
}

/// The pin of a `code` link, judged by the URL's shape (design.md, Submission and
/// screening, step 2).
pub fn code_pin(url: &str) -> Pin {
    let Some((host, path)) = host_and_path(url) else {
        return Pin::HostNotListed;
    };
    match host.as_str() {
        "github.com" | "huggingface.co" if at_commit(&host, path) => Pin::Pinned,
        "github.com" | "huggingface.co" => Pin::Unpinned,
        "zenodo.org" => Pin::CheckedOnline,
        "doi.org" if path.starts_with("10.5281/zenodo.") => Pin::CheckedOnline,
        _ => Pin::HostNotListed,
    }
}

/// Checks the `code`, `logs` and `data` links, and `## Reproduce`'s fetches.
pub(crate) fn check(
    fields: &Fields,
    reproduce: Option<&str>,
    refusals: &mut Vec<Refusal>,
    warnings: &mut Vec<Warning>,
) {
    if let Some(url) = &fields.code {
        match code_pin(url) {
            Pin::Pinned => {}
            Pin::Unpinned => refusals.push(Refusal::UnpinnedCode { url: url.clone() }),
            Pin::CheckedOnline => warnings.push(Warning::new(
                WarningKind::PinNotChecked,
                format!(
                    "the server checks at submission that `code: {url}` names a Zenodo version \
                     record, not its concept DOI"
                ),
            )),
            Pin::HostNotListed => warnings.push(not_listed("code", url)),
        }
    }
    for url in &fields.logs {
        if !is_listed(url) {
            warnings.push(not_listed("logs", url));
        }
    }
    for url in fields.data.iter().filter_map(|item| item.url.as_ref()) {
        if !is_listed(url) && !crate::front_matter::is_paper_id(url) {
            warnings.push(not_listed("data", url));
        }
    }
    if let Some(reproduce) = reproduce {
        for command in moving_ref_commands(reproduce) {
            refusals.push(Refusal::MovingRef { command });
        }
    }
}

fn not_listed(field: &str, url: &str) -> Warning {
    Warning::new(
        WarningKind::HostNotListed,
        format!(
            "`{field}: {url}` is not on a host the server checks ({}); the card will say so",
            LISTED_HOSTS.join(", ")
        ),
    )
}

fn is_listed(url: &str) -> bool {
    host_and_path(url).is_some_and(|(host, _)| LISTED_HOSTS.contains(&host.as_str()))
}

/// The lowercased host and the path after it, for an http(s) URL.
fn host_and_path(url: &str) -> Option<(String, &str)> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    let path = rest[end..].trim_start_matches('/');
    Some((host.to_ascii_lowercase(), path))
}

/// Whether a GitHub or Hugging Face path names a commit after the owner and repo, as in
/// `<owner>/<repo>/tree/<hash>`. Hugging Face datasets and spaces carry a prefix.
fn at_commit(host: &str, path: &str) -> bool {
    let segments: Vec<&str> = path.split(['/', '?', '#']).collect();
    let repo_end = match segments.first() {
        Some(&("datasets" | "spaces")) if host == "huggingface.co" => 3,
        _ => 2,
    };
    segments
        .iter()
        .skip(repo_end)
        .any(|segment| is_commit_hash(segment))
}

fn is_commit_hash(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The commands in `## Reproduce`'s code (inline code and code blocks, not prose) that
/// fetch code at a branch, tag or default branch: `git+` URLs without `@<commit>`,
/// scripts on raw.githubusercontent.com, and GitHub archives at a ref that isn't a
/// commit hash.
fn moving_ref_commands(reproduce: &str) -> Vec<String> {
    let mut code = Vec::new();
    let mut block: Option<String> = None;
    for event in Parser::new_ext(reproduce, crate::body::options()) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => block = Some(String::new()),
            Event::End(TagEnd::CodeBlock) => code.extend(block.take()),
            Event::Text(text) => {
                if let Some(block) = block.as_mut() {
                    block.push_str(&text);
                }
            }
            Event::Code(text) => code.push(text.to_string()),
            _ => {}
        }
    }
    code.iter()
        .flat_map(|snippet| snippet.lines())
        .filter(|line| line.split_whitespace().any(fetches_moving_ref))
        .map(|line| line.trim().to_string())
        .collect()
}

/// File extensions of code a command might download and run.
const SCRIPT_EXTENSIONS: [&str; 10] = [
    "py", "sh", "bash", "ipynb", "js", "ts", "rb", "pl", "r", "jl",
];

fn is_script(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        SCRIPT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
    })
}

fn fetches_moving_ref(word: &str) -> bool {
    let word = word.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '(' | ')' | ','));
    if let Some(url) = word.strip_prefix("git+") {
        let after_path = url.split_once("://").map_or(url, |(_, rest)| rest);
        let reference = after_path
            .rsplit_once('@')
            .map(|(_, reference)| reference.split(['#', '&']).next().unwrap_or(reference));
        return !reference.is_some_and(is_commit_hash);
    }
    let Some((host, path)) = host_and_path(word) else {
        return false;
    };
    let segments: Vec<&str> = path.split('/').collect();
    match (host.as_str(), segments.as_slice()) {
        ("raw.githubusercontent.com", [_, _, reference, ..]) => {
            !is_commit_hash(reference) && is_script(path)
        }
        ("github.com", [_, _, "archive", rest @ ..]) => !rest
            .iter()
            .any(|segment| is_commit_hash(segment.split('.').next().unwrap_or(segment))),
        ("codeload.github.com", [_, _, _, reference, ..]) => !is_commit_hash(reference),
        _ => false,
    }
}
