//! What `replicates` and `reanalyzes` name (entry-format.md, Fields): an entry, a
//! paper or a page, read into the key that search's `cites` filter matches.

use percent_encoding::percent_decode_str;
use url::Url;

use crate::is_valid_slug;

/// The version of the keys [`Reference::key`] gives (entry-format.md, Fields). Bump
/// it whenever a key changes, so the server rebuilds the keys it stored.
pub const REFERENCE_KEYS_VERSION: u32 = 1;

/// The most items `replicates` or `reanalyzes` keeps (entry-format.md, Fields); the
/// rest are dropped with a warning.
pub const MAX_REFERENCES: usize = 10;

/// A work an entry follows up, read from one item of `replicates` or `reanalyzes`
/// (entry-format.md, Fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// A File Drawer entry, by its ID.
    Entry(String),
    /// An entry of the same account, by its slug.
    Slug(String),
    /// An arXiv paper, by its ID without the version.
    Arxiv(String),
    /// A DOI, lowercased.
    Doi(String),
    /// Any other `http` or `https` page.
    Url(String),
}

impl Reference {
    /// The reference `text` names, or `None` when it names none in a form the format
    /// takes (entry-format.md, Fields: an entry ID, `slug:`, arXiv, a DOI or a URL).
    pub fn parse(text: &str) -> Option<Reference> {
        let text = text.trim();
        if let Some(name) = text.strip_prefix("slug:") {
            return is_valid_slug(name).then(|| Reference::Slug(name.to_string()));
        }
        if is_entry_id(text) {
            return Some(Reference::Entry(text.to_ascii_lowercase()));
        }
        if let Some(id) = strip_prefix_ignoring_case(text, "arXiv:") {
            return arxiv_id(id).map(Reference::Arxiv);
        }
        if let Some(doi) = strip_prefix_ignoring_case(text, "doi:") {
            return doi_reference(doi);
        }
        if text.starts_with("10.") {
            return doi_reference(text);
        }
        // An arXiv page written without its scheme, as people copy it.
        let schemeless = ["arxiv.org/", "www.arxiv.org/", "export.arxiv.org/"]
            .iter()
            .any(|host| strip_prefix_ignoring_case(text, host).is_some());
        if schemeless {
            return page_reference(&format!("https://{text}"));
        }
        page_reference(text)
    }

    /// The key search's `cites` matches (entry-format.md, Fields): the same work
    /// written another way has the same key, and a key read back gives itself.
    pub fn key(&self) -> String {
        match self {
            Reference::Entry(id) => id.clone(),
            Reference::Slug(name) => format!("slug:{name}"),
            Reference::Arxiv(id) => format!("arXiv:{id}"),
            Reference::Doi(key) | Reference::Url(key) => key.clone(),
        }
    }
}

/// Crockford's base32 in lowercase, the alphabet of the server's IDs.
const ID_ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

/// Whether `text` is an entry ID: 10 characters of the server's alphabet, any case.
fn is_entry_id(text: &str) -> bool {
    text.len() == 10
        && text
            .chars()
            .all(|c| ID_ALPHABET.contains(c.to_ascii_lowercase()))
}

fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let start = text.get(..prefix.len())?;
    start
        .eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// An arXiv ID without its version, or `None` when `id` isn't one: `YYMM.NNNNN`
/// (four or more digits after the dot), or the old `archive/YYMMNNN`, kept without
/// the subject class (`math.GT/0309136` is `math/0309136`).
fn arxiv_id(id: &str) -> Option<String> {
    let id = without_version(id);
    if let Some((year_month, number)) = id.split_once('.')
        && !id.contains('/')
    {
        let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
        return (year_month.len() == 4
            && number.len() >= 4
            && digits(year_month)
            && digits(number))
        .then(|| id.to_string());
    }
    let (archive, number) = id.split_once('/')?;
    let archive = archive.split_once('.').map_or(archive, |(archive, class)| {
        if class.len() == 2 && class.bytes().all(|b| b.is_ascii_uppercase()) {
            archive
        } else {
            ""
        }
    });
    let archive_ok =
        !archive.is_empty() && archive.bytes().all(|b| b.is_ascii_lowercase() || b == b'-');
    let number_ok = number.len() == 7 && number.bytes().all(|b| b.is_ascii_digit());
    (archive_ok && number_ok).then(|| format!("{archive}/{number}"))
}

/// `id` without a trailing `vN`.
fn without_version(id: &str) -> &str {
    match id.rfind('v') {
        Some(at)
            if at > 0 && id.len() > at + 1 && id[at + 1..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            &id[..at]
        }
        _ => id,
    }
}

/// The DOI arXiv gives each paper, which names the paper as its arXiv ID does.
const ARXIV_DOI: &str = "10.48550/arxiv.";

/// A DOI, `10.<4 to 9 digits>/<suffix>`, with the suffix in the characters Crossref
/// recommends; arXiv's own DOIs read as the arXiv paper.
fn doi_reference(doi: &str) -> Option<Reference> {
    let lower = doi.to_ascii_lowercase();
    if lower.starts_with(ARXIV_DOI) {
        // Sliced from the DOI as written: the old scheme's subject class is uppercase.
        return arxiv_id(&doi[ARXIV_DOI.len()..]).map(Reference::Arxiv);
    }
    let (registrant, suffix) = lower.strip_prefix("10.")?.split_once('/')?;
    let registrant_ok =
        (4..=9).contains(&registrant.len()) && registrant.bytes().all(|b| b.is_ascii_digit());
    let suffix_ok = !suffix.is_empty()
        && suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._;()/:".contains(&b));
    (registrant_ok && suffix_ok).then_some(Reference::Doi(lower))
}

/// A web page: arXiv's and doi.org's read as their paper, File Drawer's entry pages
/// as their entry, and any other `http` or `https` page, those hosts' other pages
/// included, by its normalised URL.
fn page_reference(text: &str) -> Option<Reference> {
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    let mut url = Url::parse(text).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let path = url.path().trim_end_matches('/').to_string();
    let paper = match host.as_str() {
        "arxiv.org" | "export.arxiv.org" => ["/abs/", "/pdf/", "/html/"]
            .iter()
            .find_map(|prefix| path.strip_prefix(prefix))
            .and_then(|id| arxiv_id(id.strip_suffix(".pdf").unwrap_or(id)))
            .map(Reference::Arxiv),
        // A DOI in a link arrives percent-encoded.
        "doi.org" | "dx.doi.org" => path
            .strip_prefix('/')
            .and_then(|doi| percent_decode_str(doi).decode_utf8().ok())
            .and_then(|doi| doi_reference(&doi)),
        _ => None,
    };
    if paper.is_some() {
        return paper;
    }
    if host == "filedrawer.dev" || host.ends_with(".filedrawer.dev") {
        let page = path.strip_prefix("/e/");
        let id = page.map(|page| page.strip_suffix(".txt").unwrap_or(page));
        if let Some(id) = id.filter(|id| is_entry_id(id)) {
            return Some(Reference::Entry(id.to_ascii_lowercase()));
        }
    }
    url.set_fragment(None);
    url.set_host(Some(&host)).ok()?;
    url.set_path(&path);
    let mut key = url.to_string();
    // `Url` writes a root path as `/`, which the trailing-slash rule drops.
    if url.path() == "/" && url.query().is_none() {
        key.pop();
    }
    Some(Reference::Url(key))
}
