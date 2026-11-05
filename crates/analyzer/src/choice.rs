//! Fields whose values the format names (docs/entry-format.md, Fields).

/// A field whose values the format names. A value the server doesn't know is kept,
/// but the card shows it only as "unrecognised <field>" (entry-format.md, The card).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Choice<T> {
    #[default]
    Absent,
    Known(T),
    Unrecognised(String),
}

impl<T: Copy> Choice<T> {
    pub fn known(&self) -> Option<T> {
        match self {
            Choice::Known(value) => Some(*value),
            _ => None,
        }
    }
}

/// The values one field may take, and the field's name for messages and cards.
pub trait Vocabulary: Sized + Copy {
    const FIELD: &'static str;
    fn parse(text: &str) -> Option<Self>;
    fn name(self) -> &'static str;
}

macro_rules! vocabulary {
    ($(#[$doc:meta])* $ty:ident, $field:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $ty { $($variant),+ }

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
