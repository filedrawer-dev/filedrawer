//! The Results table and what analyzer v1 reads from each row
//! (docs/entry-format.md, The Results table; Outcome and the card's numbers).

use std::collections::BTreeMap;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::choice::{Choice, Claim, Outcome, Vocabulary};
use crate::report::{Warning, WarningKind};

/// The six columns a table needs before the analyzer reads it.
pub const CORE_COLUMNS: [&str; 6] = ["model", "condition", "estimate", "ci_low", "ci_high", "n"];

/// The Results table, or why the analyzer couldn't read one.
#[derive(Debug, Clone, PartialEq)]
pub enum Results {
    NotRead(String),
    Read(ResultsTable),
}

/// The table the analyzer read, with its "Intervals:" line (entry-format.md, The
/// Results table).
#[derive(Debug, Clone, PartialEq)]
pub struct ResultsTable {
    pub intervals: Intervals,
    pub rows: Vec<Row>,
}

/// The "Intervals:" line above the table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Intervals {
    /// The level as written, without the percent sign, such as "95".
    pub level: Option<String>,
    pub method: Option<String>,
    pub per: Option<String>,
}

/// One row: its cells by lowercased column name, its claim and its reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: BTreeMap<String, String>,
    /// `None` when the row claims nothing.
    pub claim: Option<Claim>,
    pub reading: Reading,
}

impl Row {
    /// The cell under `column`, trimmed; empty when the table has no such column.
    pub fn cell(&self, column: &str) -> &str {
        self.cells.get(column).map_or("", |text| text.trim())
    }
}

/// What the analyzer read from a row's numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Read(Interval),
    /// The row couldn't be read, with the reason, such as "estimate is not a number".
    NotRead(String),
}

/// A row's numbers as analyzer v1 read them (entry-format.md, Outcome and the card's
/// numbers).
#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub estimate: f64,
    pub low: f64,
    pub high: f64,
    /// The no-effect value, and whether the interval includes it. `None` on a single
    /// group with no `no_effect_at`, whose level has no no-effect value.
    pub no_effect: Option<NoEffect>,
}

/// The row's no-effect value and whether the interval includes it, the note card line 4
/// starts with (entry-format.md, Outcome and the card's numbers).
#[derive(Debug, Clone, PartialEq)]
pub struct NoEffect {
    /// As written in `no_effect_at`, or "0" by default.
    pub value_text: String,
    pub value: f64,
    pub included: bool,
}

/// The analyzer's note on one row, as `/e/<id>` shows it beside the row
/// (entry-format.md, Outcome and the card's numbers): whether the interval includes
/// the no-effect value, then the claim. It holds no author text, so the server can
/// show it above the author-text delimiter. The no-effect value was read as a plain
/// decimal, so it is digits only.
pub fn row_note(row: &Row) -> String {
    let claim = row.claim.map_or("no claim", Claim::card_word);
    match &row.reading {
        Reading::NotRead(why) => format!("not read: {why}; {claim}"),
        Reading::Read(Interval {
            no_effect: Some(no_effect),
            ..
        }) => {
            let verb = if no_effect.included {
                "includes"
            } else {
                "excludes"
            };
            format!("interval {verb} {}; {claim}", no_effect.value_text)
        }
        Reading::Read(_) => claim.to_string(),
    }
}

/// How many rows make each claim, for the card's tally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub effect: usize,
    pub no_effect: usize,
    pub inconclusive: usize,
    pub no_claim: usize,
}

/// Reads the Results section: the first table whose header has every core column,
/// the "Intervals:" line just above it, and each row.
pub(crate) fn read(
    section: Option<&str>,
    entry_outcome: &Choice<Outcome>,
    warnings: &mut Vec<Warning>,
) -> Results {
    let Some(section) = section else {
        return not_read("there is no `## Results` section", warnings);
    };
    let Some(raw) = tables(section).into_iter().find(|table| {
        CORE_COLUMNS
            .iter()
            .all(|column| table.header.iter().any(|name| name == column))
    }) else {
        return not_read(
            &format!(
                "no table under `## Results` has all six core columns ({})",
                CORE_COLUMNS.join(", ")
            ),
            warnings,
        );
    };
    if raw.rows.is_empty() {
        return not_read("the Results table has no rows", warnings);
    }
    let intervals = raw
        .intervals_line
        .as_deref()
        .map(intervals)
        .unwrap_or_default();
    if intervals.level.is_none() {
        warnings.push(Warning::new(
            WarningKind::LevelNotStated,
            "no \"Intervals:\" line just above the Results table states the interval level, \
             such as \"Intervals: 95%, Wilson, per session.\"; rows show \"level not stated\"",
        ));
    }
    let rows = raw
        .rows
        .into_iter()
        .enumerate()
        .map(|(index, cells)| {
            let cells: BTreeMap<String, String> = raw.header.iter().cloned().zip(cells).collect();
            row(index, cells, entry_outcome, warnings)
        })
        .collect();
    Results::Read(ResultsTable { intervals, rows })
}

fn not_read(why: &str, warnings: &mut Vec<Warning>) -> Results {
    warnings.push(Warning::new(
        WarningKind::ResultsNotRead,
        format!("Results table not read: {why}"),
    ));
    Results::NotRead(why.to_string())
}

/// A table as text: lowercased header names, row cells, and the "Intervals:" line
/// of the paragraph just above it.
struct RawTable {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    intervals_line: Option<String>,
}

fn tables(section: &str) -> Vec<RawTable> {
    let mut tables = Vec::new();
    let mut paragraph: Option<String> = None;
    let mut last_paragraph: Option<String> = None;
    let mut table: Option<RawTable> = None;
    let mut cell: Option<String> = None;
    let mut in_head = false;
    for event in Parser::new_ext(section, crate::body::options()) {
        match event {
            Event::Start(Tag::Paragraph) => paragraph = Some(String::new()),
            Event::End(TagEnd::Paragraph) => last_paragraph = paragraph.take(),
            Event::Start(Tag::Table(_)) => {
                let intervals_line = last_paragraph.take().and_then(|text| {
                    text.lines()
                        .find(|line| line.trim_start().starts_with("Intervals:"))
                        .map(str::to_string)
                });
                table = Some(RawTable {
                    header: Vec::new(),
                    rows: Vec::new(),
                    intervals_line,
                });
            }
            Event::End(TagEnd::Table) => tables.extend(table.take()),
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) => {
                if let Some(table) = table.as_mut() {
                    table.rows.push(Vec::new());
                }
            }
            Event::Start(Tag::TableCell) => cell = Some(String::new()),
            Event::End(TagEnd::TableCell) => {
                if let (Some(text), Some(table)) = (cell.take(), table.as_mut()) {
                    if in_head {
                        table.header.push(text.trim().to_lowercase());
                    } else if let Some(row) = table.rows.last_mut() {
                        row.push(text.trim().to_string());
                    }
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(cell) = cell.as_mut() {
                    cell.push_str(&text);
                } else if let Some(paragraph) = paragraph.as_mut() {
                    paragraph.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(paragraph) = paragraph.as_mut() {
                    paragraph.push('\n');
                }
            }
            Event::Start(_) => last_paragraph = None,
            _ => {}
        }
    }
    tables
}

/// Reads "Intervals: 95%, Wilson, per session.": the first percentage is the level,
/// and the whole word "per" splits the method from the sampling unit.
fn intervals(line: &str) -> Intervals {
    let text = line.trim_start().trim_start_matches("Intervals:");
    let (level, rest) = match first_percentage(text) {
        Some((start, end)) => (
            Some(text[start..end].trim_end_matches('%').trim().to_string()),
            format!("{} {}", &text[..start], &text[end..]),
        ),
        None => (None, text.to_string()),
    };
    let (method, per) = match whole_word(&rest, "per") {
        Some(at) => (&rest[..at], Some(&rest[at + 3..])),
        None => (rest.as_str(), None),
    };
    Intervals {
        level,
        method: tidy(method),
        per: per.and_then(tidy),
    }
}

/// The byte range of the first "<number>%" (spaces allowed before the sign). A `%`
/// with no number before it, as in "(n%)", is skipped.
fn first_percentage(text: &str) -> Option<(usize, usize)> {
    text.match_indices('%').find_map(|(percent, _)| {
        let before = text[..percent].trim_end();
        let digits = before.len()
            - before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .count();
        (digits < before.len()).then_some((digits, percent + 1))
    })
}

/// The byte offset of the ASCII `word` standing alone, in any case.
fn whole_word(text: &str, word: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    lower.match_indices(word).map(|(at, _)| at).find(|&at| {
        let before = lower[..at].chars().next_back();
        let after = lower[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn tidy(text: &str) -> Option<String> {
    let text = text.trim_matches(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':'));
    (!text.is_empty()).then(|| text.to_string())
}

fn row(
    index: usize,
    cells: BTreeMap<String, String>,
    entry_outcome: &Choice<Outcome>,
    warnings: &mut Vec<Warning>,
) -> Row {
    let mut row = Row {
        cells,
        claim: None,
        reading: Reading::NotRead(String::new()),
    };
    let name = format!(
        "Results row {} ({}, {})",
        index + 1,
        row.cell("model"),
        row.cell("condition")
    );
    row.claim = claim(row.cell("outcome"), index == 0, entry_outcome).unwrap_or_else(|text| {
        warnings.push(Warning::new(
            WarningKind::ClaimNotRead,
            format!("{name}: outcome `{text}` is not effect, no_effect or inconclusive, so the row claims nothing"),
        ));
        None
    });
    let n = n_form(row.cell("n"));
    if n.is_none() {
        warnings.push(Warning::new(
            WarningKind::OddN,
            format!("{name}: n `{}` is not in a form the format names, such as 16/16, 96 paired or 160 (2 days); shown as written", row.cell("n")),
        ));
    }
    let single_group =
        row.cell("baseline").is_empty() && n.is_some_and(|n| n.groups == 1 && !n.paired);
    row.reading = match interval(&row, single_group, &name, warnings) {
        Ok(interval) => Reading::Read(interval),
        Err(why) => {
            warnings.push(Warning::new(
                WarningKind::RowNotRead,
                format!(
                    "{name} not read: {why} (estimate `{}`, ci_low `{}`, ci_high `{}`)",
                    row.cell("estimate"),
                    row.cell("ci_low"),
                    row.cell("ci_high")
                ),
            ));
            Reading::NotRead(why)
        }
    };
    row
}

/// The row's claim, or the unrecognised cell text. An empty primary cell takes the
/// entry's outcome unless that is `mixed`.
fn claim(
    cell: &str,
    primary: bool,
    entry_outcome: &Choice<Outcome>,
) -> Result<Option<Claim>, String> {
    if cell.is_empty() {
        return Ok(match (primary, entry_outcome.known()) {
            (true, Some(Outcome::Effect)) => Some(Claim::Effect),
            (true, Some(Outcome::NoEffect)) => Some(Claim::NoEffect),
            (true, Some(Outcome::Inconclusive)) => Some(Claim::Inconclusive),
            _ => None,
        });
    }
    Claim::parse(cell).map(Some).ok_or_else(|| cell.to_string())
}

fn interval(
    row: &Row,
    single_group: bool,
    name: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Interval, String> {
    let estimate = number(row, "estimate")?;
    let low = number(row, "ci_low")?;
    let high = number(row, "ci_high")?;
    if low > high {
        return Err("ci_low is above ci_high".into());
    }
    let written = row.cell("no_effect_at");
    let no_effect_at = match (written, single_group) {
        ("", true) => None,
        ("", false) => Some(("0".to_string(), 0.0)),
        (text, _) => match decimal(text) {
            Some(value) => Some((text.to_string(), value)),
            None => {
                warnings.push(Warning::new(
                    WarningKind::NoEffectAtNotRead,
                    format!("{name}: no_effect_at `{text}` is not a plain decimal, so the row gets no note on it"),
                ));
                None
            }
        },
    };
    Ok(Interval {
        estimate,
        low,
        high,
        no_effect: no_effect_at.map(|(value_text, value)| NoEffect {
            value_text,
            value,
            included: low <= value && value <= high,
        }),
    })
}

fn number(row: &Row, column: &str) -> Result<f64, String> {
    let text = row.cell(column);
    if text.is_empty() {
        return Err(format!("{column} is empty"));
    }
    decimal(text).ok_or_else(|| format!("{column} is not a plain decimal"))
}

/// A plain decimal: an optional sign (`+`, `-` or U+2212), digits and an optional
/// fraction. No exponent, percent sign or thousands separator.
pub(crate) fn decimal(text: &str) -> Option<f64> {
    let (negative, digits) = match text.chars().next()? {
        '-' | '\u{2212}' => (true, &text[text.chars().next()?.len_utf8()..]),
        '+' => (false, &text[1..]),
        _ => (false, text),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let all_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() && fraction.is_empty()
        || !all_digits(whole)
        || !all_digits(fraction)
        || digits.ends_with('.')
    {
        return None;
    }
    let value: f64 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// The parts of `n` that matter to the reading.
#[derive(Clone, Copy)]
struct NForm {
    groups: usize,
    paired: bool,
}

/// Reads `16/16`, `16`, `96 paired`, and any of those with a cluster count in
/// brackets, such as `80/80 (2/2 days)`.
fn n_form(text: &str) -> Option<NForm> {
    let (units, clusters) = match text.split_once(" (") {
        Some((units, rest)) => (units, Some(rest.strip_suffix(')')?)),
        None => (text, None),
    };
    let (counts, paired) = match units.strip_suffix(" paired") {
        Some(counts) => (counts, true),
        None => (units, false),
    };
    let groups = counts_of(counts)?;
    if let Some(clusters) = clusters {
        let (counts, what) = clusters.split_once(' ')?;
        counts_of(counts)?;
        if what.trim().is_empty() {
            return None;
        }
    }
    Some(NForm { groups, paired })
}

/// The number of `/`-separated whole numbers, or `None` if any part isn't one.
fn counts_of(text: &str) -> Option<usize> {
    let parts: Vec<&str> = text.split('/').collect();
    parts
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        .then_some(parts.len())
}
