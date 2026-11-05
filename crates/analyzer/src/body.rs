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
