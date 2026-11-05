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
    TooLarge { bytes: usize },
    TooManyLogs { count: usize },
    UnpinnedCode { url: String },
    MovingRef { command: String },
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
    pub(crate) fn new(kind: WarningKind, message: impl Into<String>) -> Self {
        Warning {
            kind,
            message: message.into(),
        }
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
