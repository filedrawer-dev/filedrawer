//! The body's fixed sections (docs/entry-format.md, Body).

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::Sections;

/// The markdown extensions the analyzer reads with: tables, for the Results table.
pub(crate) fn options() -> Options {
    Options::ENABLE_TABLES
}

/// Splits the body at its `##` headings and keeps the sections the format names.
/// A name matches in any case. When a name repeats, the first section counts.
pub(crate) fn sections(body: &str) -> Sections {
    let mut sections = Sections::default();
    let mut current: Option<(String, usize)> = None;
    let mut heading: Option<String> = None;
    let mut ends: Vec<(String, usize, usize)> = Vec::new();
    for (event, range) in Parser::new_ext(body, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H2,
                ..
            }) => {
                if let Some((name, start)) = current.take() {
                    ends.push((name, start, range.start));
                }
                heading = Some(String::new());
            }
            Event::Text(text) | Event::Code(text) if heading.is_some() => {
                if let Some(name) = heading.as_mut() {
                    name.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(HeadingLevel::H2)) => {
                if let Some(name) = heading.take() {
                    current = Some((name, range.end));
                }
            }
            _ => {}
        }
    }
    if let Some((name, start)) = current {
        ends.push((name, start, body.len()));
    }
    for (name, start, end) in ends {
        let slot = match name.trim().to_lowercase().as_str() {
            "setup" => &mut sections.setup,
            "results" => &mut sections.results,
            "caveats" => &mut sections.caveats,
            "not tried" => &mut sections.not_tried,
            "reproduce" => &mut sections.reproduce,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(body[start..end].trim().to_string());
        }
    }
    sections
}

/// The text of each top-level list item in `markdown`, or the whole text as one item
/// when it has no list. Used for the card's first caveat and `## Not tried`.
pub(crate) fn items(markdown: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    for event in Parser::new_ext(markdown, options()) {
        match event {
            Event::Start(Tag::Item) => {
                depth += 1;
                if depth == 1 {
                    items.push(String::new());
                }
            }
            Event::End(TagEnd::Item) => depth = depth.saturating_sub(1),
            Event::Text(text) | Event::Code(text) if depth >= 1 => {
                if let Some(item) = items.last_mut() {
                    item.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak if depth >= 1 => {
                if let Some(item) = items.last_mut() {
                    item.push(' ');
                }
            }
            _ => {}
        }
    }
    items.retain(|item| !item.trim().is_empty());
    if items.is_empty() && !markdown.trim().is_empty() {
        items.push(markdown.trim().to_string());
    }
    items
}
