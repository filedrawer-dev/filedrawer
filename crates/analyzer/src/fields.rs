//! The front matter's fields (docs/entry-format.md, Front matter).

use crate::choice::{
    Choice, CodeState, ControlChecks, ControlOutcome, ControlSource, Direction, HypothesisSet,
    Kind, Outcome, Role, SetWhen, Stage,
};

/// The fields the analyzer knows, as the author wrote them (entry-format.md, Fields).
/// Text fields hold the author's text; a number YAML read where text belongs keeps
/// its original spelling.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fields {
    pub schema: Option<String>,
    pub title: Option<String>,
    pub safety_motivation: Option<String>,
    pub hypothesis: Option<String>,
    pub outcome: Choice<Outcome>,
    pub kind: Choice<Kind>,
    pub predicted_direction: Choice<Direction>,
    pub hypothesis_set: Choice<HypothesisSet>,
    pub hypothesis_source: Option<String>,
    pub stage: Choice<Stage>,
    pub stopped_because: Option<String>,
    pub superseded_by: Option<String>,
    pub run_dates: Vec<String>,
    pub models: Vec<Model>,
    pub tags: Vec<String>,
    pub positive_control: Option<PositiveControl>,
    pub smallest_effect_of_interest: Option<SmallestEffect>,
    pub code: Option<String>,
    pub code_state: Choice<CodeState>,
    pub code_license: Option<String>,
    pub data: Vec<DataItem>,
    pub logs: Vec<String>,
    /// What the entry re-ran, and what it re-examined, as written, at most
    /// [`crate::MAX_REFERENCES`] each.
    pub replicates: Vec<String>,
    pub reanalyzes: Vec<String>,
    pub uses_runs: Vec<String>,
    pub published_in: Option<String>,
    pub slug: Option<String>,
    pub series: Option<String>,
    /// The attachment set holding the entry's attached code (design.md, Attached
    /// code), as `as-...`.
    pub attachments: Option<String>,
    pub written_by: Vec<Writer>,
    pub embargo_until: Option<String>,
}

impl Fields {
    /// The works the entry follows up (entry-format.md, Fields): `replicates` and then
    /// `reanalyzes`, each by its field's name.
    pub fn follow_ups(&self) -> [(&'static str, &[String]); 2] {
        [
            ("replicates", &self.replicates),
            ("reanalyzes", &self.reanalyzes),
        ]
    }
}

/// One item of `models`: a model with its role and route (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub name: Option<String>,
    pub role: Choice<Role>,
    pub route: Option<String>,
}

/// `positive_control` (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PositiveControl {
    pub outcome: Choice<ControlOutcome>,
    pub checks: Choice<ControlChecks>,
    pub source: Choice<ControlSource>,
}

/// `smallest_effect_of_interest`, the S on card line 5 (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmallestEffect {
    pub value: Option<String>,
    pub set: Choice<SetWhen>,
}

/// One item of `data`: a dataset and its SHA-256 (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataItem {
    pub url: Option<String>,
    pub sha256: Option<String>,
}

/// One item of `written_by`: an agent that wrote the entry (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Writer {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub temperature: Option<String>,
    pub harness: Option<String>,
}
