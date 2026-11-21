//! The checks on what entries name and refer to (design.md, Agent interface):
//! slugs and declared runs. Submissions, PUTs, versions and batches all run them.

use std::collections::{BTreeMap, BTreeSet};

use analyzer::{Analysis, is_valid_slug};

use crate::AppState;
use crate::auth::TokenAccount;
use crate::error::AppError;
use crate::{entries, queue};

/// What may become of an entry's `slug`.
pub enum SlugRule {
    /// A first submission: any free slug, or none.
    New,
    /// A new version of a chain with this slug, or none: it keeps the chain's slug,
    /// and a chain with none may take a free one.
    Version(Option<String>),
    /// A PUT on an entry with this slug, or none: it stays as it is.
    Put(Option<String>),
}

/// One entry as the checks see it.
pub struct Linked<'a> {
    /// How replies name it: "entry 2 (slug b)", or "the entry".
    pub label: String,
    pub analysis: &'a Analysis,
    pub slug_rule: SlugRule,
}

/// What's wrong with a set of entries' slugs and runs, given the runs a batch
/// declares, each ID with its declaration. Empty when nothing is.
pub async fn problems(
    state: &AppState,
    account: &TokenAccount,
    members: &[Linked<'_>],
    declared_runs: &BTreeMap<String, String>,
) -> Result<Vec<String>, AppError> {
    let mut problems = Vec::new();
    let mut slugs_here = BTreeSet::new();
    for member in members {
        let Some(slug) = &member.analysis.fields.slug else {
            continue;
        };
        if !is_valid_slug(slug) {
            problems.push(format!(
                "{}: a slug is 1 to 64 letters, digits, `.`, `_` or `-`",
                member.label
            ));
        } else if slugs_here.contains(slug) {
            problems.push(format!(
                "{}: slug `{slug}` is used twice in this batch",
                member.label
            ));
        }
        slugs_here.insert(slug.clone());
    }
    for member in members {
        problems.extend(slug_problems(state, account, member).await?);
        problems.extend(run_problems(state, account, member, declared_runs).await?);
    }
    Ok(problems)
}

async fn slug_problems(
    state: &AppState,
    account: &TokenAccount,
    member: &Linked<'_>,
) -> Result<Vec<String>, AppError> {
    let slug = member.analysis.fields.slug.as_deref();
    let label = &member.label;
    let problem = match (&member.slug_rule, slug) {
        (SlugRule::Put(kept), _) if kept.as_deref() != slug => Some(format!(
            "{label}: a PUT keeps the entry's slug; write `slug: {}`",
            kept.as_deref().unwrap_or("")
        )),
        (SlugRule::Version(Some(kept)), Some(slug)) if slug != kept => Some(format!(
            "{label}: a new version keeps its chain's slug `{kept}`, not `{slug}`"
        )),
        (SlugRule::New | SlugRule::Version(None), Some(slug))
            if entries::slug_taken(&state.pool, &account.id, slug).await? =>
        {
            Some(format!(
                "{label}: slug `{slug}` is already one of this account's entries'"
            ))
        }
        _ => None,
    };
    Ok(problem.into_iter().collect())
}

async fn run_problems(
    state: &AppState,
    account: &TokenAccount,
    member: &Linked<'_>,
    declared_runs: &BTreeMap<String, String>,
) -> Result<Vec<String>, AppError> {
    let mut problems = Vec::new();
    // Each run the entry names, once, with its declaration.
    let mut seen = BTreeSet::new();
    let mut used: Vec<(String, String)> = Vec::new();
    for run in &member.analysis.fields.uses_runs {
        if !seen.insert(run) {
            continue;
        }
        let declaration = match declared_runs.get(run) {
            Some(declaration) => Some(declaration.clone()),
            None => entries::run_declaration(&state.pool, &account.id, run).await?,
        };
        match declaration {
            Some(declaration) => used.push((run.clone(), declaration)),
            None => problems.push(format!(
                "{}: `uses_runs` names `{run}`, which neither this batch nor an earlier one declares",
                member.label
            )),
        }
    }
    // The screen reads the declarations with the entry, so they are capped with it.
    let bytes = queue::run_text(&used).len();
    if bytes > queue::MAX_RUN_TEXT_BYTES {
        problems.push(format!(
            "{}: {}",
            member.label,
            queue::run_text_refusal(bytes)
        ));
    }
    Ok(problems)
}
