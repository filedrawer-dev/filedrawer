//! Fields whose values the format names (docs/entry-format.md, Fields).

/// A field whose values the format names. A value the server doesn't know is kept,
/// but the card shows it only as "unrecognised <field>" (entry-format.md, The card).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Choice<T> {
    /// No value: the entry leaves the field out or empty, or gives it a list or
    /// mapping, which is warned.
    #[default]
    Absent,
    /// One of the values the format names.
    Known(T),
    /// A value the format doesn't name, as written.
    Unrecognised(String),
}

impl<T: Copy> Choice<T> {
    /// The value, when it is one the format names.
    pub fn known(&self) -> Option<T> {
        match self {
            Choice::Known(value) => Some(*value),
            _ => None,
        }
    }
}

/// The values one field may take, and the field's name for messages and cards.
pub trait Vocabulary: Sized + Copy {
    /// The field's name, as messages and cards give it.
    const FIELD: &'static str;
    /// The value `text` names, if it is one of the field's.
    fn parse(text: &str) -> Option<Self>;
    /// The value as an entry writes it.
    fn name(self) -> &'static str;
}

/// Declares a field's values: an enum with one variant per value, each documented by
/// the text an entry writes for it, and its `Vocabulary`.
macro_rules! vocabulary {
    ($(#[$doc:meta])* $ty:ident, $field:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[doc = concat!("The values of `", $field, "`.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $ty { $(#[doc = concat!("`", $text, "`")] $variant),+ }

        impl Vocabulary for $ty {
            const FIELD: &'static str = $field;
            fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
            fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    };
}

vocabulary!(
    /// The author's headline call.
    Outcome, "outcome", {
    NoEffect => "no_effect",
    Inconclusive => "inconclusive",
    Effect => "effect",
    Mixed => "mixed",
});
vocabulary!(Kind, "kind", {
    Experiment => "experiment",
    Measurement => "measurement",
    Simulation => "simulation",
    Reanalysis => "reanalysis",
    Audit => "audit",
});
vocabulary!(Direction, "predicted_direction", {
    Increase => "increase",
    Decrease => "decrease",
    Either => "either",
    None => "none",
});
vocabulary!(HypothesisSet, "hypothesis_set", {
    BeforeRun => "before_run",
    AfterRun => "after_run",
    Unknown => "unknown",
});
vocabulary!(Stage, "stage", {
    Probe => "probe",
    Pilot => "pilot",
    Main => "main",
});
vocabulary!(Role, "models.role", {
    Subject => "subject",
    Judge => "judge",
    Monitor => "monitor",
    Other => "other",
});
vocabulary!(ControlOutcome, "positive_control.outcome", {
    Passed => "passed",
    Failed => "failed",
    NotRun => "not_run",
    Mixed => "mixed",
});
vocabulary!(ControlChecks, "positive_control.checks", {
    Pipeline => "pipeline",
    Manipulation => "manipulation",
});
vocabulary!(ControlSource, "positive_control.source", {
    ThisRun => "this_run",
    OtherRun => "other_run",
    OtherEntry => "other_entry",
});
vocabulary!(SetWhen, "smallest_effect_of_interest.set", {
    BeforeRun => "before_run",
    AfterRun => "after_run",
});
vocabulary!(CodeState, "code_state", {
    Committed => "committed",
    Uncommitted => "uncommitted",
    NeverCommitted => "never_committed",
});
vocabulary!(
    /// What one row of the Results table claims, from its `outcome` cell.
    Claim, "outcome cell", {
    Effect => "effect",
    NoEffect => "no_effect",
    Inconclusive => "inconclusive",
});

impl Outcome {
    /// The word cards use: "null" for `no_effect` (entry-format.md, The card, line 1).
    pub fn card_word(self) -> &'static str {
        match self {
            Outcome::NoEffect => "null",
            other => other.name(),
        }
    }
}

impl Claim {
    /// The word cards use: "null" for `no_effect`, as for [`Outcome::card_word`].
    pub fn card_word(self) -> &'static str {
        match self {
            Claim::NoEffect => "null",
            other => other.name(),
        }
    }
}
