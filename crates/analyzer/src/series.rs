//! A series document and the `series` field that cites it (design.md, Series;
//! entry-format.md, Fields: `series`).

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use yaml_rust2::{Yaml, YamlLoader};

use crate::front_matter;

/// The most bytes a series description may hold (design.md, Series: "at most
/// 100 KB"), counted as the entry cap is.
pub const MAX_DESCRIPTION_BYTES: usize = crate::MAX_ENTRY_BYTES;

/// The most characters a series title may hold.
pub const MAX_TITLE_CHARS: usize = 300;

/// A series' title and description, as `POST /series` and each new version send them:
/// front matter with `title`, then the description as the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesText {
    pub title: String,
    pub description: String,
}

/// Reads a series document, or lists every reason it can't be stored.
pub fn read_series(text: &str) -> Result<SeriesText, Vec<String>> {
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n");
    let Some((yaml, body)) = front_matter::split(&text) else {
        return Err(vec![
            "a series starts with a `---` line, `title: ...`, and another `---` line; the description follows".into(),
        ]);
    };
    let mut problems = Vec::new();
    let title = match title_of(yaml) {
        Ok(title) => Some(title),
        Err(problem) => {
            problems.push(problem);
            None
        }
    };
    let description = body.trim();
    if description.is_empty() {
        problems.push("the description, below the front matter, is empty".into());
    }
    if description.len() > MAX_DESCRIPTION_BYTES {
        problems.push(format!(
            "the description is {} bytes; the limit is {MAX_DESCRIPTION_BYTES}",
            description.len()
        ));
    }
    match (title, problems.is_empty()) {
        (Some(title), true) => Ok(SeriesText {
            title,
            description: description.to_string(),
        }),
        _ => Err(problems),
    }
}

/// The front matter's `title`, the only field a series takes.
fn title_of(yaml: &str) -> Result<String, String> {
    if front_matter::uses_aliases(yaml) {
        return Err(
            "the front matter uses a YAML alias (`*name`); write `title` out in full".into(),
        );
    }
    let documents = YamlLoader::load_from_str(yaml)
        .map_err(|error| format!("the front matter isn't YAML the server can read: {error}"))?;
    let Some(Yaml::Hash(map)) = documents.into_iter().next() else {
        return Err("the front matter needs `title: ...`".into());
    };
    let mut title = None;
    for (key, value) in &map {
        match (key.as_str(), value) {
            (Some("title"), Yaml::String(text)) => title = Some(text.trim().to_string()),
            (Some("title"), _) => return Err("`title` must be text".into()),
            (Some(other), _) => {
                return Err(format!(
                    "a series takes only `title` in its front matter, not `{other}`; put everything else in the description"
                ));
            }
            (None, _) => return Err("the front matter's keys must be names".into()),
        }
    }
    let title = title.ok_or("the front matter needs `title: ...`")?;
    let chars = title.chars().count();
    if chars == 0 || chars > MAX_TITLE_CHARS || title.chars().any(char::is_control) {
        return Err(format!(
            "a title is 1 to {MAX_TITLE_CHARS} characters on one line"
        ));
    }
    Ok(title)
}

/// What an entry's `series` field cites: `<id>@<version>`, optionally `#<section>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesCite {
    pub id: String,
    pub version: i64,
    pub section: Option<String>,
}

/// Reads a `series` value, or `None` when it isn't `<id>@<version>[#<section>]` with a
/// version from 1 and a section that could be an anchor ([`heading_anchor`]).
pub fn parse_series_cite(value: &str) -> Option<SeriesCite> {
    let value = value.trim();
    let (cite, section) = match value.split_once('#') {
        Some((cite, section)) => (cite, Some(section)),
        None => (value, None),
    };
    let (id, version) = cite.split_once('@')?;
    let is_anchor = |text: &str| {
        (1..=MAX_TITLE_CHARS).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
    };
    let id_ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let version: i64 = version.parse().ok().filter(|version| *version >= 1)?;
    if !id_ok || section.is_some_and(|section| !is_anchor(section)) {
        return None;
    }
    Some(SeriesCite {
        id: id.to_string(),
        version,
        section: section.map(str::to_string),
    })
}

/// A heading's anchor: lower case, spaces as `-`, and only ASCII letters, digits, `-`
/// and `_` kept, so `## Model access (v2)` is `model-access-v2`.
pub fn heading_anchor(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// The section of `markdown` under the first heading whose anchor is `anchor`: the
/// heading through to the next heading of the same or a higher level.
pub fn description_section<'a>(markdown: &'a str, anchor: &str) -> Option<&'a str> {
    let mut headings = Vec::new();
    let mut open: Option<(pulldown_cmark::HeadingLevel, usize, String)> = None;
    for (event, range) in Parser::new_ext(markdown, crate::body::options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                open = Some((level, range.start, String::new()))
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, name)) = open.as_mut() {
                    name.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start, name)) = open.take() {
                    headings.push((level, start, heading_anchor(&name)));
                }
            }
            _ => {}
        }
    }
    let at = headings.iter().position(|(_, _, name)| name == anchor)?;
    let (level, start, _) = headings.get(at)?;
    let end = headings
        .iter()
        .skip(at + 1)
        .find(|(other, _, _)| other <= level)
        .map_or(markdown.len(), |(_, start, _)| *start);
    markdown.get(*start..end).map(str::trim_end)
}

/// The entry as the server stores it once it has read `series` (design.md, Series:
/// the server "takes it out of the stored entry"). Like [`crate::stored_text`], text it
/// can't cut by lines comes back unchanged.
pub fn without_series(text: &str) -> String {
    front_matter::without_fields(text, |name| name == "series")
}
