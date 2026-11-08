//! Author markdown as HTML for the pages (design.md, Pages for people): entry bodies
//! and series descriptions. Raw HTML is dropped, every link is resolved and kept only
//! when it is `http` or `https`, and images become links, so nothing an author wrote
//! runs, styles the page as the server's, or makes a reader's browser fetch from
//! another host.

use pulldown_cmark::{
    Alignment, CowStr, Event, HeadingLevel, LinkType, Parser, Tag, TagEnd, TextMergeStream,
};
use url::Url;

/// Markdown rendered for a page.
pub struct Rendered {
    pub html: String,
    /// Whether anything the author wrote was left out: raw HTML, or a link or image
    /// whose address isn't `http` or `https`.
    pub dropped: bool,
}

/// `markdown` as HTML. Relative links resolve against `base`, the page's own URL
/// (without one, only absolute links are kept). A `slug:<name>` link, or a bare
/// `slug:<name>` in text outside code, goes to the entry `slugs` names for it, or
/// stays text.
pub fn to_html(markdown: &str, base: Option<&Url>, slugs: &[(String, String)]) -> Rendered {
    let mut dropped = false;
    // Each open link or image, and whether its tag was written, so its end matches.
    let mut open: Vec<bool> = Vec::new();
    let mut in_code_block = false;
    let mut events = Vec::new();
    // Merged, so a `slug:` mention that pulldown-cmark split, at a `_` say, is whole.
    let parser = TextMergeStream::new(Parser::new_ext(markdown, analyzer::markdown_options()));
    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                events.push(Event::Start(Tag::CodeBlock(kind)));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                events.push(Event::End(TagEnd::CodeBlock));
            }
            Event::Text(text) if !in_code_block && open.is_empty() => {
                link_slugs(&text, slugs, &mut events);
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                dropped |= !html.trim().is_empty();
            }
            Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) => {}
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                let target = if link_type == LinkType::Email || !open.is_empty() {
                    None
                } else {
                    let target = destination(&dest_url, base, slugs);
                    dropped |= target.is_none() && !dest_url.starts_with("slug:");
                    target
                };
                open.push(target.is_some());
                if let Some(target) = target {
                    events.push(Event::Html(anchor(&target).into()));
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                // An image inside a link shows only its alt text: no link nests in a
                // link, and the outer one still goes where it went.
                let target = if open.is_empty() {
                    let target = destination(&dest_url, base, slugs);
                    dropped |= target.is_none();
                    target
                } else {
                    None
                };
                open.push(target.is_some());
                if let Some(target) = target {
                    events.push(Event::Html(anchor(&target).into()));
                    events.push(Event::Text("image: ".into()));
                }
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if open.pop() == Some(true) {
                    events.push(Event::Html("</a>".into()));
                }
            }
            Event::Start(Tag::Heading { level, .. }) => events.push(Event::Start(Tag::Heading {
                level: demoted(level),
                id: None,
                classes: Vec::new(),
                attrs: Vec::new(),
            })),
            Event::End(TagEnd::Heading(level)) => {
                events.push(Event::End(TagEnd::Heading(demoted(level))));
            }
            // Without alignment, so pulldown-cmark writes no inline style, which the
            // pages' Content-Security-Policy would refuse.
            Event::Start(Tag::Table(alignments)) => {
                events.push(Event::Html(r#"<div class="tw">"#.into()));
                let plain = vec![Alignment::None; alignments.len()];
                events.push(Event::Start(Tag::Table(plain)));
            }
            Event::End(TagEnd::Table) => {
                events.push(Event::End(TagEnd::Table));
                events.push(Event::Html("</div>".into()));
            }
            other => events.push(other),
        }
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    Rendered { html, dropped }
}

/// `text` as events, with each `slug:<name>` that `slugs` resolves turned into a link
/// to its entry; one that doesn't resolve stays text, as a `[text](slug:name)` does.
fn link_slugs(text: &str, slugs: &[(String, String)], events: &mut Vec<Event<'_>>) {
    let mut done = 0;
    for (range, name) in analyzer::slug_mentions(text) {
        let Some((_, id)) = slugs.iter().find(|(slug, _)| slug == name) else {
            continue;
        };
        if range.start > done {
            events.push(Event::Text(text[done..range.start].to_string().into()));
        }
        events.push(Event::Html(anchor(&format!("/e/{id}")).into()));
        events.push(Event::Text(text[range.clone()].to_string().into()));
        events.push(Event::Html("</a>".into()));
        done = range.end;
    }
    if done < text.len() {
        events.push(Event::Text(text[done..].to_string().into()));
    }
}

/// Where a link to `dest` goes: resolved against `base`, and kept only when it is
/// `http` or `https`. A `slug:<name>` goes to the entry `slugs` names for it.
fn destination(dest: &CowStr, base: Option<&Url>, slugs: &[(String, String)]) -> Option<String> {
    if let Some(name) = dest.strip_prefix("slug:") {
        let (_, id) = slugs.iter().find(|(slug, _)| slug == name)?;
        return Some(format!("/e/{id}"));
    }
    let url = match base {
        Some(base) => base.join(dest),
        None => Url::parse(dest),
    }
    .ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.into())
}

/// The opening tag of a link to `target`.
fn anchor(target: &str) -> String {
    format!(r#"<a href="{}" rel="nofollow ugc">"#, attribute(target))
}

/// `text` escaped for a double-quoted HTML attribute.
fn attribute(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// An author's heading two levels down, so `#` sits below the page's own headings.
fn demoted(level: HeadingLevel) -> HeadingLevel {
    match level {
        HeadingLevel::H1 => HeadingLevel::H3,
        HeadingLevel::H2 => HeadingLevel::H4,
        HeadingLevel::H3 => HeadingLevel::H5,
        _ => HeadingLevel::H6,
    }
}
