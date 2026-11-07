//! The offline half of the link checks: hosts and pins by the URL's shape
//! (design.md, Submission and screening, step 2). Nothing here fetches a URL.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::fields::Fields;
use crate::report::{Refusal, Warning, WarningKind};

/// The hosts the server fetches `code` and `logs` links on, matched exactly.
/// wandb.ai isn't one: it sends every link to forge.coreweave.com, which answers the
/// same page for a run that doesn't exist, so no fetch can tell a dead link.
pub const LISTED_HOSTS: [&str; 3] = ["github.com", "huggingface.co", "zenodo.org"];

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
    let path = path.as_str();
    match host.as_str() {
        "github.com" | "huggingface.co" if at_commit(&host, path) => Pin::Pinned,
        "github.com" | "huggingface.co" => Pin::Unpinned,
        "zenodo.org" => Pin::CheckedOnline,
        "doi.org" if path.starts_with("10.5281/zenodo.") => Pin::CheckedOnline,
        _ => Pin::HostNotListed,
    }
}

/// The repository and commit a pinned GitHub link names, as in
/// `github.com/<owner>/<repo>/tree/<hash>` (entry-format.md, Fields: `code`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubCommit {
    /// `<owner>/<repo>`, lowercased, without a `.git` suffix.
    pub repo: String,
    /// The 40-character hash, lowercased.
    pub sha: String,
}

/// The commit a GitHub link is pinned to, or `None` if it isn't a pinned GitHub link
/// whose owner and repository GitHub's API could be asked about (entry-format.md,
/// Fields: `code`).
pub fn github_commit(url: &str) -> Option<GithubCommit> {
    let (host, path) = host_and_path(url)?;
    if host != "github.com" {
        return None;
    }
    let segments: Vec<&str> = path.split(['/', '?', '#']).collect();
    let [owner, repo, rest @ ..] = segments.as_slice() else {
        return None;
    };
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let sha = rest.iter().find(|segment| is_commit_hash(segment))?;
    let name = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (name(owner) && name(repo)).then(|| GithubCommit {
        repo: format!("{owner}/{repo}").to_ascii_lowercase(),
        sha: sha.to_ascii_lowercase(),
    })
}

/// The Zenodo record a link names: `zenodo.org/records/<n>` (or `record`, or
/// `api/records`), or the DOI `10.5281/zenodo.<n>` on zenodo.org or doi.org
/// (entry-format.md, Fields: `code`).
pub fn zenodo_record(url: &str) -> Option<u64> {
    let (host, path) = host_and_path(url)?;
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let number = match host.as_str() {
        "zenodo.org" => {
            let path = path.strip_prefix("api/").unwrap_or(path);
            path.strip_prefix("records/")
                .or_else(|| path.strip_prefix("record/"))
                .or_else(|| path.strip_prefix("doi/10.5281/zenodo."))?
        }
        "doi.org" => path.strip_prefix("10.5281/zenodo.")?,
        _ => return None,
    };
    let number = number.split('/').next().unwrap_or(number);
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
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
    listed_host(url).is_some()
}

/// The host of an http(s) `url` when it is one of [`LISTED_HOSTS`], read as a
/// browser reads it (entry-format.md, Fields: `code` and `logs` belong on these
/// hosts).
pub fn listed_host(url: &str) -> Option<String> {
    host_and_path(url)
        .map(|(host, _)| host)
        .filter(|host| LISTED_HOSTS.contains(&host.as_str()))
}

/// The lowercased host of an http(s) URL, and everything after it without the
/// leading `/`. The URL is parsed as browsers parse it, so `https://a.example\@b.com`
/// is on a.example, as a browser opening it finds.
fn host_and_path(url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let mut path = parsed.path().trim_start_matches('/').to_string();
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    if let Some(fragment) = parsed.fragment() {
        path.push('#');
        path.push_str(fragment);
    }
    Some((host, path))
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
            !is_commit_hash(reference) && is_script(&path)
        }
        ("github.com", [_, _, "archive", rest @ ..]) => !rest
            .iter()
            .any(|segment| is_commit_hash(segment.split('.').next().unwrap_or(segment))),
        ("codeload.github.com", [_, _, _, reference, ..]) => !is_commit_hash(reference),
        _ => false,
    }
}
