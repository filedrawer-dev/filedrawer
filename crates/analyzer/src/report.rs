//! What the server tells a submitting agent: refusals and warnings
//! (docs/entry-format.md, Accept, warn, and re-render).

use std::fmt;

/// The largest entry the server takes, in bytes (design.md, Limits and abuse).
pub const MAX_ENTRY_BYTES: usize = 100 * 1024;
/// The most `logs` URLs an entry may list (design.md, Limits and abuse).
pub const MAX_LOGS: usize = 10;

/// A reason the server refuses an entry. Anything else it can't read is a [`Warning`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No front matter the analyzer can read, with why.
    FrontMatterUnreadable(String),
    /// A required field is missing: its name.
    MissingField(&'static str),
    /// A required body section is missing: its heading.
    MissingSection(&'static str),
    /// The entry is over [`MAX_ENTRY_BYTES`].
    TooLarge {
        /// The entry's size.
        bytes: usize,
    },
    /// `logs` lists more than [`MAX_LOGS`] URLs.
    TooManyLogs {
        /// How many it lists.
        count: usize,
    },
    /// `code` names GitHub or Hugging Face without a 40-character commit (design.md,
    /// Submission and screening, step 2). A Zenodo concept record is refused by the
    /// server's online check instead.
    UnpinnedCode {
        /// The link as written.
        url: String,
    },
    /// `## Reproduce` fetches code at a branch or tag rather than a commit.
    MovingRef {
        /// The command that does it.
        command: String,
    },
    /// An `embargo_until` that isn't a date, as written.
    EmbargoUnreadable(String),
    /// A front-matter key asking for an embargo, refused while embargoes are paused
    /// (entry-format.md, Accept, warn, and re-render).
    EmbargoPaused {
        /// The key as written.
        field: String,
    },
    /// A long run of encoded text that hides readable text, such as base64.
    EncodedText {
        /// The encoding.
        kind: crate::encoded::Encoding,
        /// The run's first characters.
        start: String,
    },
    /// A `series` value that isn't `<id>@<version>[#<section>]` with a version from 1
    /// (entry-format.md, Fields: `series`); it holds the value as written.
    SeriesCite(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::FrontMatterUnreadable(why) => {
                write!(f, "the front matter can't be read: {why}")
            }
            Refusal::MissingField("outcome") => write!(
                f,
                "the required field `outcome` is missing; YAML reads a bare `null` as no \
                 value, so write `no_effect` for a null result"
            ),
            Refusal::MissingField(name) => write!(f, "the required field `{name}` is missing"),
            Refusal::MissingSection(name) => {
                write!(f, "the required section `## {name}` is missing")
            }
            Refusal::TooLarge { bytes } => write!(
                f,
                "the entry is {bytes} bytes, over the cap of {MAX_ENTRY_BYTES} bytes"
            ),
            Refusal::TooManyLogs { count } => {
                write!(f, "`logs` lists {count} URLs, over the cap of {MAX_LOGS}")
            }
            Refusal::UnpinnedCode { url } => write!(
                f,
                "the `code` link isn't pinned to a commit or version: {url}. Link a \
                 40-character commit hash, or a Zenodo version record"
            ),
            Refusal::SeriesCite(value) => write!(
                f,
                "`series: {value}` should be `<id>@<version>` or `<id>@<version>#<section>`, with a version from 1"
            ),
            Refusal::EncodedText { kind, start } => write!(
                f,
                "it holds a long run of {kind} (starting `{start}`) that decodes to text \
                 or a compressed stream; the archive is read as text, so link to encoded \
                 or compressed data (on Zenodo or in a repo) rather than pasting it"
            ),
            Refusal::EmbargoUnreadable(value) => write!(
                f,
                "`embargo_until: {value}` is not a date written YYYY-MM-DD; the server won't \
                 guess at an embargo, since publishing too early can't be undone"
            ),
            Refusal::EmbargoPaused { field } => write!(
                f,
                "`{field}` asks for an embargo, and embargoes are paused: the archive takes no \
                 release dates for now. Leave `{field}` out, and submit once the entry may be public"
            ),
            Refusal::MovingRef { command } => write!(
                f,
                "`## Reproduce` fetches code at a moving ref, not a commit: {command}"
            ),
        }
    }
}

/// The kind of thing the server couldn't read, or read with a doubt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// A mapping written `{a, b}`, where a comma splits a value into keys.
    FlowMapping,
    /// A paper written as a bare number, which YAML reads as a decimal.
    NumberForPaper,
    /// `published_in` that is neither a DOI nor `arXiv:<id>`.
    NotAPaper,
    /// A `replicates` or `reanalyzes` item in no form the format takes.
    NotAReference,
    /// `replicates` or `reanalyzes` past [`crate::MAX_REFERENCES`] items.
    TooManyReferences,
    /// A value the format doesn't name, for a field whose values it does.
    UnrecognisedValue,
    /// A field the format doesn't name; the server keeps it.
    UnrecognisedField,
    /// A field the server removes, such as `label` or one starting `fd_`.
    StrippedField,
    /// A field of the wrong shape, such as a list where one value belongs.
    MalformedField,
    /// A `code`, `logs` or `data` link on a host the server doesn't check.
    HostNotListed,
    /// A `code` link whose pin only the server's check at submission can confirm,
    /// such as a Zenodo record.
    PinNotChecked,
    /// No Results table the analyzer can read.
    ResultsNotRead,
    /// No line saying the intervals' level.
    LevelNotStated,
    /// A Results row the analyzer can't read.
    RowNotRead,
    /// An `outcome` cell that isn't `effect`, `no_effect` or `inconclusive`.
    ClaimNotRead,
    /// An `n` in none of the forms the format names, which are `16/16`, `96 paired`
    /// and `160 (2 days)`.
    OddN,
    /// A `no_effect_at` cell that isn't a plain decimal.
    NoEffectAtNotRead,
}

/// Something the server accepted but couldn't fully read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// What sort of thing went unread.
    pub kind: WarningKind,
    /// What the author reads, on one line.
    pub message: String,
}

impl Warning {
    /// A warning on one line: author text inside `message` loses its line breaks and
    /// other control characters, so it can't forge lines in a response.
    pub(crate) fn new(kind: WarningKind, message: impl Into<String>) -> Self {
        let message = message
            .into()
            .chars()
            .map(|c| {
                if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        Warning { kind, message }
    }
}

/// What every warning ends with (entry-format.md, Accept, warn, and re-render), the
/// server's own as well as the analyzer's.
pub const WARNING_FEEDBACK: &str =
    "(if the server should read this kind of submission, say so through POST /feedback)";

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {WARNING_FEEDBACK}", self.message)
    }
}
