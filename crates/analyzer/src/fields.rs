//! The front matter's fields (docs/entry-format.md, Front matter).

use crate::choice::{
    Choice, CodeState, ControlChecks, ControlOutcome, ControlSource, Direction, HypothesisSet,
    Kind, Outcome, Role, SetWhen, Stage,
};

/// The fields the analyzer knows, as the author wrote them (entry-format.md, Fields).
/// Text fields hold the author's text; a number YAML read where text belongs keeps
/// its original spelling. Each field is the front-matter field of its name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fields {
    /// The format's draft the entry was written for, such as `4`.
    pub schema: Option<String>,
    /// The title, card line 2.
    pub title: Option<String>,
    /// One sentence on why the result matters for AI safety.
    pub safety_motivation: Option<String>,
    /// The hypothesis, card line 3.
    pub hypothesis: Option<String>,
    /// The author's headline call.
    pub outcome: Choice<Outcome>,
    /// What sort of result this is; `experiment` when left out.
    pub kind: Choice<Kind>,
    /// The way the hypothesis says the metric moves.
    pub predicted_direction: Choice<Direction>,
    /// Whether the hypothesis was written before or after the run.
    pub hypothesis_set: Choice<HypothesisSet>,
    /// Where the hypothesis was written, so a reader can check when.
    pub hypothesis_source: Option<String>,
    /// How far the work got: probe, pilot or main.
    pub stage: Choice<Stage>,
    /// Why the work stopped, in the author's words.
    pub stopped_because: Option<String>,
    /// The entry that replaced this pilot, by ID or `slug:`.
    pub superseded_by: Option<String>,
    /// The first and last day of the runs.
    pub run_dates: Vec<String>,
    /// Every model, with its role and route.
    pub models: Vec<Model>,
    /// Free-text tags.
    pub tags: Vec<String>,
    /// The check that the pipeline or the manipulation can show an effect.
    pub positive_control: Option<PositiveControl>,
    /// The smallest effect the author would care about, S.
    pub smallest_effect_of_interest: Option<SmallestEffect>,
    /// A link to the code, pinned to a commit or version.
    pub code: Option<String>,
    /// Whether the code that ran is in a commit.
    pub code_state: Choice<CodeState>,
    /// The licence of attached code; MIT when left out.
    pub code_license: Option<String>,
    /// The stored data the results come from.
    pub data: Vec<DataItem>,
    /// Links to the run logs, at most [`crate::MAX_LOGS`].
    pub logs: Vec<String>,
    /// What the entry re-ran, as written, at most [`crate::MAX_REFERENCES`].
    pub replicates: Vec<String>,
    /// What the entry re-examined, as written, at most [`crate::MAX_REFERENCES`].
    pub reanalyzes: Vec<String>,
    /// The IDs of runs declared in a batch that the entry draws on.
    pub uses_runs: Vec<String>,
    /// The DOI or arXiv ID of a paper that now covers the result.
    pub published_in: Option<String>,
    /// A short name, unique among the account's entries, for `slug:` links.
    pub slug: Option<String>,
    /// The series version the entry cites, as `<id>@<version>[#<section>]`.
    pub series: Option<String>,
    /// The attachment set holding the entry's attached code (design.md, Attached
    /// code), as `as-...`.
    pub attachments: Option<String>,
    /// Each agent that wrote the entry.
    pub written_by: Vec<Writer>,
    /// A release date; the archive takes none for now (design.md, Embargo).
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
    /// The model ID the provider served, or `alias:<name>`.
    pub name: Option<String>,
    /// What the model did: subject, judge, monitor or other.
    pub role: Choice<Role>,
    /// How it was reached, such as `api` or `claude-cli`.
    pub route: Option<String>,
}

/// `positive_control` (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PositiveControl {
    /// Whether the control passed.
    pub outcome: Choice<ControlOutcome>,
    /// Whether it checks the pipeline or the manipulation.
    pub checks: Choice<ControlChecks>,
    /// Where it ran.
    pub source: Choice<ControlSource>,
}

/// `smallest_effect_of_interest`, the S on card line 5 (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmallestEffect {
    /// S, a positive number in the metric's units, as written.
    pub value: Option<String>,
    /// Whether S was set before or after the run.
    pub set: Choice<SetWhen>,
}

/// One item of `data`: a dataset and its SHA-256 (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataItem {
    /// Where the dataset is: a URL on an allowed host, or a DOI.
    pub url: Option<String>,
    /// The dataset's SHA-256.
    pub sha256: Option<String>,
}

/// One item of `written_by`: an agent that wrote the entry (entry-format.md, Fields).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Writer {
    /// The model ID.
    pub model: Option<String>,
    /// Its effort setting, when it can see it.
    pub effort: Option<String>,
    /// Its temperature, when it can see it.
    pub temperature: Option<String>,
    /// The harness it ran in, such as `Claude Code 2.1.318`.
    pub harness: Option<String>,
}
