//! The card: the few lines search and /cards.txt return (docs/entry-format.md, The card).

use crate::Analysis;
use crate::choice::{
    Choice, Claim, CodeState, ControlChecks, ControlOutcome, ControlSource, Direction,
    HypothesisSet, Kind, Outcome, Role, SetWhen, Stage, Vocabulary,
};
use crate::fields::{Fields, is_positive_decimal};
use crate::links::{Pin, code_pin};
use crate::references::{Reference, paper_key, successor_key};
use crate::results::{Reading, Results, Row};

/// The analyzer version every card names.
pub const ANALYZER_VERSION: u32 = 2;

/// The longest each of the card's 11 lines may be, in characters, after its label.
pub const LINE_CAPS: [usize; 11] = [200, 200, 300, 300, 150, 300, 200, 300, 300, 300, 100];

/// The server's label that starts each of the card's 11 lines, so a reader tells the
/// server's words from an author's by the start of the line (entry-format.md, The
/// card).
const LABELS: [&str; 11] = [
    "entry",
    "title",
    "hypothesis",
    "result",
    "design",
    "stats",
    "models",
    "first caveat",
    "not tried",
    "flags",
    "author",
];

/// What only the server knows about an entry. The CLI passes the default, and the
/// card shows placeholders such as `<id>` for what's missing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardContext {
    /// The entry's ID, line 1.
    pub id: Option<String>,
    /// Who posted it, line 11.
    pub author: Option<Author>,
    /// Haiku's one-line doubt that the body matches the outcome.
    pub screening_note: Option<String>,
    /// The entry cites an attachment set: "code attached".
    pub code_attached: bool,
    /// The link check found the `code` link dead.
    pub code_link_dead: bool,
    /// The link check found a `logs` link dead.
    pub logs_link_dead: bool,
    /// What the server's online check of the `code` link's pin flags, if anything.
    pub pin: Option<PinFlag>,
    /// The entry that supersedes this one, as the server found it: a newer public
    /// version, or else the public entry of the same account that the author's
    /// `superseded_by` names. The card shows the author's value only through this.
    pub superseded_by: Option<String>,
    /// The entries that replicate this one, line 10.
    pub replications: Vec<Replication>,
}

/// What the server's online check of a `code` link's pin puts on card line 10
/// (entry-format.md, The card). A link has one pin state, so one flag at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinFlag {
    /// "pin not verified": the server couldn't confirm the pin.
    NotVerified,
    /// "code pin failed": a check after submission found a Zenodo concept record.
    Failed,
    /// "commit on no branch": the GitHub commit is on no branch or tag of the
    /// repository it names, a fork's or one whose branch was squashed or rebased
    /// away.
    OnNoBranch,
}

/// The account behind an entry, for card line 11 (entry-format.md, The card).
#[derive(Debug, Clone, PartialEq)]
pub struct Author {
    /// `None` once the account is deleted: the card then says so and shows no name.
    pub display_name: Option<String>,
    /// The account's ID, `a-...`.
    pub account_id: String,
    /// Linked GitHub or ORCID handles, as the card shows them.
    pub handles: Vec<String>,
}

/// An entry that replicates this one, listed in card line 10's flags (entry-format.md,
/// Replications).
#[derive(Debug, Clone, PartialEq)]
pub struct Replication {
    /// The replication's ID.
    pub id: String,
    /// Its outcome.
    pub outcome: Choice<Outcome>,
    /// Posted by the original's own account.
    pub self_replication: bool,
}

/// Renders the card, one line per newline (entry-format.md, The card). Each line
/// starts with the server's label, such as `title: `. Line 8, the first caveat,
/// appears only on an `effect` card, so a card has 10 or 11 lines.
pub fn render_card(analysis: &Analysis, context: &CardContext) -> String {
    let fields = &analysis.fields;
    let is_effect = fields.outcome.known() == Some(Outcome::Effect);
    let lines = [
        Some(header(analysis, context)),
        Some(whole_line(fields.title.as_deref(), "(no title)")),
        Some(whole_line(fields.hypothesis.as_deref(), "(no hypothesis)")),
        Some(primary_row(analysis)),
        Some(design(analysis)),
        Some(interval_details(analysis)),
        Some(models_and_tags(fields)),
        is_effect.then(|| first_caveat(analysis)),
        Some(not_tried(analysis)),
        Some(flags(analysis, context)),
        Some(byline(context)),
    ];
    lines
        .into_iter()
        .zip(LINE_CAPS)
        .zip(LABELS)
        .filter_map(|((parts, cap), label)| {
            parts.map(|parts| format!("{label}: {}", fit(&parts, cap)))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A piece of a card line: the server's own words, or text an author wrote.
enum Part {
    Server(String),
    Author(String),
}

fn server(text: impl Into<String>) -> Part {
    Part::Server(text.into())
}

fn author(text: impl Into<String>) -> Part {
    Part::Author(text.into())
}

/// Joins a line's parts within `cap` characters. The server's words are kept whole and
/// author text is cut to fit what they leave, in order, so author text can never push
/// a server note off the line. Author text loses its line breaks first.
fn fit(parts: &[Part], cap: usize) -> String {
    let reserved: usize = parts
        .iter()
        .map(|part| match part {
            Part::Server(text) => text.chars().count(),
            Part::Author(_) => 0,
        })
        .sum();
    let mut budget = cap.saturating_sub(reserved);
    let mut line = String::new();
    for part in parts {
        match part {
            Part::Server(text) => line.push_str(text),
            Part::Author(text) => {
                let text = cut(&one_line(text), budget);
                budget -= text.chars().count();
                line.push_str(&text);
            }
        }
    }
    cut(&line, cap)
}

/// `text` within `cap` characters, ending in "…" when it was longer.
fn cut(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let mut kept: String = text.chars().take(cap.saturating_sub(1)).collect();
    if cap > 0 {
        kept.push('…');
    }
    kept
}

/// Author text on one line: line breaks and other control characters become spaces,
/// bidirectional overrides are dropped, and runs of spaces collapse.
fn one_line(text: &str) -> String {
    let spaced: String = text
        .chars()
        .filter(|c| !crate::is_bidi_control(*c))
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A line that is author text alone after its label, lines 2 and 3, or the server's
/// `absent` when there is none or it shows nothing: a blank line would end the card
/// where cards are listed one after another.
fn whole_line(text: Option<&str>, absent: &str) -> Vec<Part> {
    match text.filter(|text| !crate::shows_nothing(text)) {
        Some(text) => vec![author(text)],
        None => vec![server(absent)],
    }
}

/// A known value's words, or "unrecognized <field>", or `absent` when it's missing.
fn label<T: Vocabulary>(value: &Choice<T>, absent: &str, words: fn(T) -> &'static str) -> String {
    match value {
        Choice::Known(known) => words(*known).to_string(),
        Choice::Unrecognized(_) => format!("unrecognized {}", T::FIELD),
        Choice::Absent => absent.to_string(),
    }
}

/// Line 1: ID, kind, outcome, stage and the tally of row claims.
fn header(analysis: &Analysis, context: &CardContext) -> Vec<Part> {
    let fields = &analysis.fields;
    let tally = analysis.tally();
    let rows = match analysis.results {
        Results::NotRead(_) => "rows: not read".to_string(),
        Results::Read(_) => {
            let counts: Vec<String> = [
                (tally.effect, "effect"),
                (tally.no_effect, "null"),
                (tally.inconclusive, "inconclusive"),
                (tally.no_claim, "no claim"),
            ]
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, name)| format!("{count} {name}"))
            .collect();
            format!("rows: {}", counts.join(", "))
        }
    };
    let mut parts = vec![server(format!(
        "{}; {}; {}; {}; {rows}",
        context.id.as_deref().unwrap_or("<id>"),
        label(&fields.kind, Kind::Experiment.name(), Kind::name),
        label(&fields.outcome, "no outcome", Outcome::card_word),
        label(&fields.stage, "no stage", Stage::name),
    ))];
    parts.extend(followed_up(fields));
    parts
}

/// Line 1's end: how many works the entry replicates and re-examines, then the key of
/// the first, so the line's only author text comes last.
fn followed_up(fields: &Fields) -> Vec<Part> {
    let relations = fields.follow_ups();
    let Some(first) = relations.iter().find_map(|(_, items)| items.first()) else {
        return Vec::new();
    };
    let counts: Vec<String> = relations
        .iter()
        .filter(|(_, items)| !items.is_empty())
        .map(|(name, items)| format!("{name} {}", items.len()))
        .collect();
    let counts = format!("; {}: ", counts.join(", "));
    match Reference::parse(first) {
        Some(reference) => vec![server(counts), author(reference.key())],
        None => vec![server(format!("{counts}unrecognized reference"))],
    }
}

/// Line 4: the server's note on the primary row, its claim and how many rows follow,
/// then the row's numbers as written.
fn primary_row(analysis: &Analysis) -> Vec<Part> {
    let Results::Read(table) = &analysis.results else {
        return vec![server("Results table not read")];
    };
    let Some(row) = table.rows.first() else {
        return vec![server("Results table not read")];
    };
    let more = match table.rows.len() - 1 {
        0 => String::new(),
        1 => "; and 1 more row".to_string(),
        n => format!("; and {n} more rows"),
    };
    let interval = match &row.reading {
        Reading::NotRead(why) => {
            return vec![
                server("primary row not read: "),
                author(why.as_str()),
                server(more),
            ];
        }
        Reading::Read(interval) => interval,
    };
    let mut parts = Vec::new();
    if let Some(no_effect) = &interval.no_effect {
        let verb = if no_effect.included {
            "includes"
        } else {
            "excludes"
        };
        parts.extend([
            server(format!("interval {verb} ")),
            author(no_effect.value_text.as_str()),
            server("; "),
        ]);
    }
    let claim = row.claim.map_or("no claim", Claim::card_word);
    let signed = |column: &str| written_number(row.cell(column), interval.no_effect.is_some());
    parts.extend([
        server(format!("{claim}{more}: ")),
        author(signed("estimate")),
        server(" ["),
        author(signed("ci_low")),
        server(", "),
        author(signed("ci_high")),
        server("], n "),
        author(row.cell("n")),
    ]);
    if !row.cell("unit").is_empty() {
        parts.extend([server(", "), author(row.cell("unit"))]);
    }
    parts
}

/// A number as the author wrote it, with its sign shown: U+2212 for minus, and `+`
/// on a positive difference. A single group's level gets no `+`.
fn written_number(text: &str, difference: bool) -> String {
    if let Some(rest) = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('\u{2212}'))
    {
        return format!("\u{2212}{rest}");
    }
    let unsigned = text.strip_prefix('+').unwrap_or(text);
    let zero = unsigned.chars().all(|c| c == '0' || c == '.');
    if difference && !zero {
        format!("+{unsigned}")
    } else {
        unsigned.to_string()
    }
}

/// Line 5: the positive control, the prediction and, for an effect, whether the
/// primary row went as predicted; then S if given.
fn design(analysis: &Analysis) -> Vec<Part> {
    let fields = &analysis.fields;
    let control = match &fields.positive_control {
        None => "no positive control reported".to_string(),
        Some(control)
            if [
                matches!(control.outcome, Choice::Unrecognized(_)),
                matches!(control.checks, Choice::Unrecognized(_)),
                matches!(control.source, Choice::Unrecognized(_)),
            ]
            .contains(&true) =>
        {
            "unrecognized positive_control".to_string()
        }
        Some(control) => {
            let checks = label(&control.checks, "positive control", |checks| match checks {
                ControlChecks::Pipeline => "pipeline check",
                ControlChecks::Manipulation => "manipulation check",
            });
            let outcome = label(
                &control.outcome,
                "outcome not stated",
                |outcome| match outcome {
                    ControlOutcome::NotRun => "not run",
                    other => other.name(),
                },
            );
            let source = match &control.source {
                Choice::Absent => String::new(),
                source => format!(
                    " ({})",
                    label(source, "", |source| match source {
                        ControlSource::ThisRun => "this run",
                        ControlSource::OtherRun => "other run",
                        ControlSource::OtherEntry => "other entry",
                    })
                ),
            };
            format!("{checks} {outcome}{source}")
        }
    };
    let prediction = match &fields.predicted_direction {
        Choice::Absent => "no prediction".to_string(),
        direction => {
            let direction = label(direction, "", |direction| match direction {
                Direction::Increase => "predicted increase",
                Direction::Decrease => "predicted decrease",
                Direction::Either => "predicted either direction",
                Direction::None => "predicted no effect",
            });
            let set = label(
                &fields.hypothesis_set,
                "set when not stated",
                |set| match set {
                    HypothesisSet::BeforeRun => "set before run",
                    HypothesisSet::AfterRun | HypothesisSet::Unknown => "set after run",
                },
            );
            let verdict = as_predicted(analysis)
                .map(|verdict| format!(", {verdict}"))
                .unwrap_or_default();
            format!("{direction}, {set}{verdict}")
        }
    };
    let mut parts = vec![server(format!("{control}; {prediction}"))];
    if let Some(smallest) = &analysis.fields.smallest_effect_of_interest {
        let set = match &smallest.set {
            Choice::Unrecognized(_) => "unrecognized S set".to_string(),
            set => label(set, "set when not stated", |set| match set {
                SetWhen::BeforeRun => "set before run",
                SetWhen::AfterRun => "set after run",
            }),
        };
        // S sits among the server's labels, so it shows only as a number.
        match smallest.value.as_deref() {
            Some(value) if !is_positive_decimal(value) => {
                parts.push(server("; unrecognized smallest_effect_of_interest"));
            }
            value => parts.extend([
                server("; S "),
                author(value.unwrap_or("not given")),
                server(format!(", {set}")),
            ]),
        }
    }
    parts
}

/// For an `effect` entry, whether the primary row moved the way the hypothesis said.
fn as_predicted(analysis: &Analysis) -> Option<&'static str> {
    if analysis.fields.outcome.known() != Some(Outcome::Effect) {
        return None;
    }
    let Reading::Read(interval) = &analysis.primary_row()?.reading else {
        return None;
    };
    let no_effect = interval.no_effect.as_ref()?.value;
    let went_up = match interval.estimate.partial_cmp(&no_effect)? {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => return None,
    };
    let predicted_up = match analysis.fields.predicted_direction.known()? {
        Direction::Increase => true,
        Direction::Decrease => false,
        Direction::None => return Some("opposite to prediction"),
        Direction::Either => return None,
    };
    Some(if went_up == predicted_up {
        "as predicted"
    } else {
        "opposite to prediction"
    })
}

/// Line 6: the baseline, the interval's level, and the method and sampling unit.
fn interval_details(analysis: &Analysis) -> Vec<Part> {
    let (row, intervals) = match &analysis.results {
        Results::Read(table) => (table.rows.first(), Some(&table.intervals)),
        Results::NotRead(_) => (None, None),
    };
    let cell = |column: &str| row.map_or("", |row: &Row| row.cell(column));
    let mut parts = match cell("baseline") {
        "" => vec![server("no baseline")],
        baseline => vec![server("baseline "), author(baseline)],
    };
    match intervals.and_then(|intervals| intervals.level.as_deref()) {
        Some(level) => parts.extend([server("; "), author(level), server("%")]),
        None => parts.push(server("; level not stated")),
    }
    let method = Some(cell("method"))
        .filter(|method| !method.is_empty())
        .or_else(|| intervals.and_then(|intervals| intervals.method.as_deref()));
    let per = Some(cell("per"))
        .filter(|per| !per.is_empty())
        .or_else(|| intervals.and_then(|intervals| intervals.per.as_deref()));
    if let Some(method) = method {
        parts.extend([server(", "), author(method)]);
    }
    if let Some(per) = per {
        parts.extend([server(", per "), author(per)]);
    }
    parts
}

/// Line 7: the subject models (and models with no role given) and the tags.
fn models_and_tags(fields: &Fields) -> Vec<Part> {
    let subjects: Vec<&str> = fields
        .models
        .iter()
        .filter(|model| matches!(model.role, Choice::Known(Role::Subject) | Choice::Absent))
        .filter_map(|model| model.name.as_deref())
        .collect();
    let mut parts = Vec::new();
    if subjects.is_empty() {
        parts.push(server("none listed as subject"));
    } else {
        parts.push(author(subjects.join(", ")));
    }
    parts.push(server("; tags: "));
    if fields.tags.is_empty() {
        parts.push(server("none"));
    } else {
        parts.push(author(fields.tags.join(", ")));
    }
    parts
}

/// Line 8, on an `effect` card: the first caveat.
fn first_caveat(analysis: &Analysis) -> Vec<Part> {
    let first = analysis
        .sections
        .caveats
        .as_deref()
        .and_then(|caveats| crate::body::items(caveats).into_iter().next());
    match first {
        Some(caveat) => vec![author(caveat)],
        None => vec![server("none listed")],
    }
}

/// Line 9: the first three items of `## Not tried`.
fn not_tried(analysis: &Analysis) -> Vec<Part> {
    let items = analysis
        .sections
        .not_tried
        .as_deref()
        .map(crate::body::items)
        .unwrap_or_default();
    if items.is_empty() {
        return vec![server("none listed")];
    }
    let first: Vec<String> = items
        .into_iter()
        .take(3)
        .map(|item| one_line(&item))
        .collect();
    vec![author(first.join("; "))]
}

/// Whether `c` is a combining mark of the block Latin and Greek letters take theirs
/// from.
fn is_combining_mark(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

/// Signs outside ASCII that a note about numbers may hold.
const NOTE_SIGNS: &str = "±−×÷≤≥≈≠→←…–—°µ·‰";

/// The screen's note as it goes between the server's quotes on line 10. A model
/// wrote it from the author's text, so nothing in it may end the quotes or begin
/// what reads as the server's next flag. `"` and a backtick become `'`, and `;`
/// becomes `,`. Outside ASCII only letters, digits, a combining mark on one of
/// those, and [`NOTE_SIGNS`] stay; anything else becomes a space, which takes every
/// other script's quote marks and semicolons with it, and the modifier letters,
/// some of which look like quote marks. A run of `'` shrinks to one, so two can't
/// stand in for `"`.
fn quotable(note: &str) -> String {
    let mut kept = String::with_capacity(note.len());
    for c in note.chars() {
        let c = match c {
            '"' | '`' => '\'',
            ';' => ',',
            c if c.is_ascii() => c,
            // Modifier letters, which count as letters.
            '\u{2b0}'..='\u{2ff}'
            | '\u{a700}'..='\u{a721}'
            | '\u{a788}'..='\u{a78c}'
            | '\u{ff70}'
            | '\u{ff9e}'..='\u{ff9f}' => ' ',
            // A combining mark alone, or on a space, can draw a quote mark, so one
            // stays only on a letter or digit, or on another mark that does.
            c if is_combining_mark(c) => {
                let on_a_letter = kept.ends_with(|before: char| {
                    before.is_alphanumeric() || is_combining_mark(before)
                });
                if on_a_letter { c } else { ' ' }
            }
            c if c.is_alphanumeric() || NOTE_SIGNS.contains(c) => c,
            _ => ' ',
        };
        if !(c == '\'' && kept.ends_with('\'')) {
            kept.push(c);
        }
    }
    kept
}

/// Line 10: the analyzer version, the screening note, and the flags that apply.
fn flags(analysis: &Analysis, context: &CardContext) -> Vec<Part> {
    let fields = &analysis.fields;
    let mut parts = vec![server(format!("analyzer v{ANALYZER_VERSION}"))];
    if let Some(note) = &context.screening_note {
        parts.extend([
            server("; screening note: \""),
            author(quotable(note)),
            server("\""),
        ]);
    }
    if context.code_attached {
        parts.push(server("; code attached"));
    } else if fields.code.is_none() {
        parts.push(server("; no code"));
    }
    if matches!(
        fields.code_state.known(),
        Some(CodeState::Uncommitted | CodeState::NeverCommitted)
    ) {
        parts.push(server("; code that ran not committed"));
    }
    if context.code_link_dead {
        parts.push(server("; code link dead"));
    }
    if context.logs_link_dead {
        parts.push(server("; logs link dead"));
    }
    // A link the pin rule refuses can be on the card only of an entry stored before
    // the rule, so nothing checked its pin either.
    let unlisted = || {
        fields
            .code
            .as_deref()
            .is_some_and(|url| matches!(code_pin(url), Pin::HostNotListed | Pin::Unpinned))
    };
    match context.pin {
        Some(PinFlag::Failed) => parts.push(server("; code pin failed")),
        Some(PinFlag::OnNoBranch) => parts.push(server("; commit on no branch")),
        Some(PinFlag::NotVerified) => parts.push(server("; pin not verified")),
        None if unlisted() => parts.push(server("; pin not verified")),
        None => {}
    }
    if fields.data.is_empty() && reanalyzes_stored_data(analysis) {
        parts.push(server("; no data"));
    }
    // The flags are the server's own, so the two fields shown among them appear
    // only as the server read them, never as written.
    if let Some(paper) = &fields.published_in {
        match paper_key(paper) {
            Some(key) => parts.extend([server("; now part of a paper "), author(key)]),
            None => parts.push(server("; unrecognized published_in")),
        }
    }
    // An author's `superseded_by` shows only as the server found it, since an
    // author could otherwise name any entry as the newer one; a value in no form
    // the server reads is flagged.
    match (&context.superseded_by, &fields.superseded_by) {
        (Some(successor), _) => parts.push(server(format!("; superseded by {successor}"))),
        (None, Some(named)) if successor_key(named).is_none() => {
            parts.push(server("; unrecognized superseded_by"));
        }
        (None, _) => {}
    }
    for replication in &context.replications {
        let kind = if replication.self_replication {
            "self-replication"
        } else {
            "replication"
        };
        parts.push(server(format!(
            "; {kind} {} ({})",
            replication.id,
            label(&replication.outcome, "no outcome", Outcome::card_word)
        )));
    }
    parts
}

/// Whether `## Reproduce` says it re-analyzes stored data.
fn reanalyzes_stored_data(analysis: &Analysis) -> bool {
    analysis.sections.reproduce.as_deref().is_some_and(|text| {
        let text = text.to_lowercase();
        text.contains("re-analy") || text.contains("reanaly")
    })
}

/// Line 11: the account ID, then the author's display name and linked handles, or
/// "account deleted" once the account is deleted. The ID comes first, since a name
/// may hold text shaped like one. The name loses its invisible characters, and one
/// that then shows nothing is the server's "(no name)".
fn byline(context: &CardContext) -> Vec<Part> {
    let Some(person) = &context.author else {
        return vec![server("<author>")];
    };
    let account = format!("account {}, ", person.account_id);
    let Some(name) = &person.display_name else {
        return vec![server(format!("{account}account deleted"))];
    };
    let mut parts = vec![server(account), author(crate::shown_name(name))];
    if !person.handles.is_empty() {
        parts.push(server(format!(", {}", person.handles.join(", "))));
    }
    parts
}
