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
    FrontMatterUnreadable(String),
    MissingField(&'static str),
    MissingSection(&'static str),
    TooLarge {
        bytes: usize,
    },
    TooManyLogs {
        count: usize,
    },
    UnpinnedCode {
        url: String,
    },
    MovingRef {
        command: String,
    },
    EmbargoUnreadable(String),
    /// A long run of encoded text that hides readable text, such as base64: `kind`
    /// names the encoding, and `start` is the run's first characters.
    EncodedText {
        kind: crate::encoded::Encoding,
        start: String,
    },
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
    FlowMapping,
    NumberForPaper,
    NotAPaper,
    UnrecognisedValue,
    UnrecognisedField,
    StrippedField,
    MalformedField,
    HostNotListed,
    PinNotChecked,
    ResultsNotRead,
    LevelNotStated,
    RowNotRead,
    ClaimNotRead,
    OddN,
    NoEffectAtNotRead,
}

/// Something the server accepted but couldn't fully read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub kind: WarningKind,
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

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (if the server should read this kind of submission, say so through POST /feedback)",
            self.message
        )
    }
}
