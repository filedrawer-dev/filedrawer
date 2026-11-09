//! The files an entry's body names in code spans, which a reader can open only when
//! they are attached or linked (design.md, Agent interface). The server matches them
//! against the entry's attachment set.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::{front_matter_and_body, markdown_options};

/// Extensions of code and text files an entry could attach or link. Data, logs, lock
/// files and binaries are left out: they belong in `data` or `logs`, and a set holds
/// only UTF-8 text.
const FILE_EXTENSIONS: &[&str] = &[
    "bash", "bib", "c", "cc", "cfg", "cpp", "css", "go", "h", "hpp", "html", "ini", "ipynb",
    "java", "jl", "js", "kt", "lua", "m", "md", "mjs", "pl", "py", "r", "rb", "rmd", "rs", "rst",
    "scala", "sh", "sql", "swift", "tex", "toml", "ts", "tsx", "txt", "yaml", "yml", "zsh",
];

/// Each file path the body of `text` names as a whole inline code span outside a
/// link, cleaned for matching and once each, in order (design.md, Agent interface;
/// entry-format.md leaves it out, as the server's warning, not the format's). A span
/// holding a command, a URL, a glob or a name whose extension isn't a code or text
/// file's isn't one.
pub fn file_mentions(text: &str) -> Vec<String> {
    let Some((_, body)) = front_matter_and_body(text) else {
        return Vec::new();
    };
    let mut in_link = 0_usize;
    let mut found: Vec<String> = Vec::new();
    for event in Parser::new_ext(&body, markdown_options()) {
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => in_link += 1,
            Event::End(TagEnd::Link | TagEnd::Image) => in_link = in_link.saturating_sub(1),
            Event::Code(span) if in_link == 0 => {
                if let Some(path) = file_path(&span)
                    && !found.contains(&path)
                {
                    found.push(path);
                }
            }
            _ => {}
        }
    }
    found
}

/// The path a code span names, without an origin prefix (`author:`, or
/// `<repo>@<commit>:` as attachment origins are written), a trailing `:<line>` or
/// `@<rev>`, or leading `./`, `../`, `/` and `~/`; `None` when it isn't a file path.
fn file_path(span: &str) -> Option<String> {
    let span = span.trim();
    if span.is_empty()
        || span.chars().any(char::is_whitespace)
        || span.contains("://")
        || span.contains(['*', '?', '['])
    {
        return None;
    }
    let mut path = without_origin(span);
    if let Some((before, line)) = path.rsplit_once(':')
        && !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
    {
        path = before;
    }
    if let Some((before, _)) = path.rsplit_once('@') {
        path = before;
    }
    loop {
        let rest = ["./", "../", "~/", "/"]
            .iter()
            .find_map(|prefix| path.strip_prefix(prefix));
        match rest {
            Some(rest) => path = rest,
            None => break,
        }
    }
    let name = path.rsplit('/').next()?;
    let (stem, extension) = name.rsplit_once('.')?;
    let listed = FILE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str());
    (!stem.is_empty() && listed && !path.contains(':')).then(|| path.to_string())
}

/// `span` without the origin prefix attachment sets write: `author:` or
/// `<repo>@<7 to 40 hex digits>:`.
fn without_origin(span: &str) -> &str {
    if let Some(rest) = span.strip_prefix("author:") {
        return rest;
    }
    if let Some((repo_at_commit, rest)) = span.split_once(':')
        && let Some((repo, commit)) = repo_at_commit.split_once('@')
        && !repo.is_empty()
        && (7..=40).contains(&commit.len())
        && commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return rest;
    }
    span
}
