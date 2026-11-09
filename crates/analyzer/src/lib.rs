#![forbid(unsafe_code)]
//! Reads a File Drawer entry by docs/entry-format.md: what would refuse it, what the
//! server couldn't read, the Results table, and the card. The server and the
//! `entry-check` CLI share this code.

mod body;
mod card;
mod choice;
mod encoded;
mod fields;
mod file_mentions;
mod front_matter;
mod links;
mod references;
mod report;
mod results;
mod series;

pub use card::{
    ANALYZER_VERSION, Author, CardContext, LINE_CAPS, PinFlag, Replication, render_card,
};
pub use choice::{
    Choice, Claim, CodeState, ControlChecks, ControlOutcome, ControlSource, Direction,
    HypothesisSet, Kind, Outcome, Role, SetWhen, Stage, Vocabulary,
};
pub use encoded::{
    Compression, Decoded, Encoding, compression, decodings, decodings_with_zlib, reads_as_text,
    starts_zlib,
};
pub use fields::{DataItem, Fields, Model, PositiveControl, SmallestEffect, Writer};
pub use file_mentions::file_mentions;
pub use links::{
    GithubCommit, LISTED_HOSTS, Pin, code_pin, github_commit, listed_host, zenodo_record,
};
pub use references::{MAX_REFERENCES, REFERENCE_KEYS_VERSION, Reference};
pub use report::{MAX_ENTRY_BYTES, MAX_LOGS, Refusal, WARNING_FEEDBACK, Warning, WarningKind};
pub use results::{
    CORE_COLUMNS, Interval, Intervals, NoEffect, Reading, Results, ResultsTable, Row, Tally,
    row_note,
};
pub use series::{
    MAX_DESCRIPTION_BYTES, MAX_TITLE_CHARS, SeriesCite, SeriesText, description_section,
    heading_anchor, parse_series_cite, read_series, without_series,
};

/// The reasons `text` would be refused for long encoded runs that hide text, for
/// text the server checks outside an entry, such as a run's declaration (design.md,
/// Submission and screening, step 3).
pub fn encoded_text(text: &str) -> Vec<String> {
    encoded::refusals(text)
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Everything the analyzer read from one entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub fields: Fields,
    /// Front-matter fields the format doesn't name. The server stores them.
    pub unrecognised_fields: Vec<String>,
    pub sections: Sections,
    pub results: Results,
    pub refusals: Vec<Refusal>,
    pub warnings: Vec<Warning>,
}

/// The body's fixed sections, each as its markdown source without the heading
/// (entry-format.md, Body).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    pub setup: Option<String>,
    pub results: Option<String>,
    pub caveats: Option<String>,
    pub not_tried: Option<String>,
    pub reproduce: Option<String>,
}

impl Analysis {
    /// Whether the server would refuse the entry.
    pub fn refused(&self) -> bool {
        !self.refusals.is_empty()
    }

    /// The first row of the Results table, the author's headline.
    pub fn primary_row(&self) -> Option<&Row> {
        match &self.results {
            Results::Read(table) => table.rows.first(),
            Results::NotRead(_) => None,
        }
    }

    /// The primary row as the analyzer read it, for the submitting agent to check
    /// (entry-format.md, The Results table). Holds author text.
    pub fn primary_row_summary(&self) -> String {
        let Some(row) = self.primary_row() else {
            return "none (Results table not read)".to_string();
        };
        let cells: Vec<String> = CORE_COLUMNS
            .iter()
            .map(|column| format!("{column} {}", row.cell(column)))
            .collect();
        let claim = row.claim.map_or("none", Claim::card_word);
        let reading = match &row.reading {
            Reading::Read(_) => String::new(),
            Reading::NotRead(why) => format!("; not read: {why}"),
        };
        format!("{}; claim {claim}{reading}", cells.join("; "))
    }

    /// How many rows make each claim.
    pub fn tally(&self) -> Tally {
        let mut tally = Tally::default();
        if let Results::Read(table) = &self.results {
            for row in &table.rows {
                match row.claim {
                    Some(Claim::Effect) => tally.effect += 1,
                    Some(Claim::NoEffect) => tally.no_effect += 1,
                    Some(Claim::Inconclusive) => tally.inconclusive += 1,
                    None => tally.no_claim += 1,
                }
            }
        }
        tally
    }
}

/// Reads one entry, a markdown file with YAML front matter (entry-format.md, Accept,
/// warn, and re-render). Never fails: what would refuse the entry is in
/// [`Analysis::refusals`], and what it couldn't read is in [`Analysis::warnings`].
pub fn analyze(text: &str) -> Analysis {
    let mut refusals = Vec::new();
    if text.len() > MAX_ENTRY_BYTES {
        refusals.push(Refusal::TooLarge { bytes: text.len() });
    }
    let text = normalised(text);
    let Some((yaml, body)) = front_matter::split(&text) else {
        refusals.push(Refusal::FrontMatterUnreadable(
            "the entry must start with a `---` line, the YAML fields, and another `---` line"
                .into(),
        ));
        return Analysis {
            fields: Fields::default(),
            unrecognised_fields: Vec::new(),
            sections: body::sections(&text),
            results: Results::NotRead("no front matter".into()),
            refusals,
            warnings: Vec::new(),
        };
    };
    let front = front_matter::read(yaml);
    refusals.extend(front.refusals);
    let mut warnings = front.warnings;
    let sections = body::sections(body);
    for (name, section) in [
        ("Setup", &sections.setup),
        ("Reproduce", &sections.reproduce),
    ] {
        if section.is_none() {
            refusals.push(Refusal::MissingSection(name));
        }
    }
    if front.fields.logs.len() > MAX_LOGS {
        refusals.push(Refusal::TooManyLogs {
            count: front.fields.logs.len(),
        });
    }
    refusals.extend(encoded::refusals(&text));
    if let Some(value) = front.fields.series.as_deref()
        && series::parse_series_cite(value).is_none()
    {
        refusals.push(Refusal::SeriesCite(value.to_string()));
    }
    links::check(
        &front.fields,
        sections.reproduce.as_deref(),
        &mut refusals,
        &mut warnings,
    );
    let results = results::read(
        sections.results.as_deref(),
        &front.fields.outcome,
        &mut warnings,
    );
    Analysis {
        fields: front.fields,
        unrecognised_fields: front.unrecognised,
        results,
        sections,
        refusals,
        warnings,
    }
}

/// The entry as the server stores it: the submitted text without `label` and the
/// `fd_` fields, which the server reserves (design.md, Submission and screening).
/// Each reserved top-level key goes with every line up to the next top-level key; the
/// YAML parser finds the keys, so quoting a key doesn't hide it. Everything else stays
/// byte for byte. Text whose front matter doesn't parse comes back unchanged, and the
/// analyzer refuses it.
pub fn stored_text(text: &str) -> String {
    front_matter::without_reserved_fields(text)
}

/// Whether `c` is a bidirectional embedding, override or isolate, which author text
/// shown beside the server's words drops, so it can't reorder them (entry-format.md,
/// The card: author text can't forge lines).
pub fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// The entry's front matter and body, split as [`analyze`] splits them, or `None`
/// when it doesn't start with front matter (entry-format.md, Front matter).
pub fn front_matter_and_body(text: &str) -> Option<(String, String)> {
    let text = normalised(text);
    front_matter::split(&text).map(|(yaml, body)| (yaml.to_string(), body.to_string()))
}

/// `text` without a leading byte-order mark and with `\r\n` line ends as `\n`, as
/// the analyzer reads every entry.
fn normalised(text: &str) -> String {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
}

/// The markdown options the analyzer reads bodies with, for anything else that
/// renders them, so both see the same document (entry-format.md, Body).
pub fn markdown_options() -> pulldown_cmark::Options {
    body::options()
}

/// Whether `slug` is a slug the format takes (entry-format.md, Fields): 1 to 64 of
/// `A-Z`, `a-z`, `0-9`, `.`, `_` and `-`. Slugs are shown beside the server's own
/// words, so nothing else is allowed in one.
pub fn is_valid_slug(slug: &str) -> bool {
    (1..=64).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Every `slug:<name>` an entry's text refers to (design.md, Agent interface), in
/// its fields or its body.
pub fn slug_references(text: &str) -> std::collections::BTreeSet<String> {
    slug_mentions(text)
        .into_iter()
        .map(|(_, name)| name.to_string())
        .collect()
}

/// Each `slug:<name>` in `text`, in order, as the byte range of the whole mention and
/// the name (entry-format.md, Fields: `slug`; design.md, Agent interface). A name runs
/// over letters, digits, `-`, `_` and `.`, less any final `.`, which ends a sentence.
pub fn slug_mentions(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    text.match_indices("slug:")
        .filter_map(|(at, prefix)| {
            let start = at + prefix.len();
            let rest = &text[start..];
            let length = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                .unwrap_or(rest.len());
            let name = rest[..length].trim_end_matches('.');
            is_valid_slug(name).then(|| (at..start + name.len(), name))
        })
        .collect()
}
