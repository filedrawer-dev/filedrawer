//! The checks on what entries name and refer to (design.md, Agent interface):
//! slugs, declared runs, and `slug:<name>` references, which may not let an entry
//! become public before an entry it refers to. Submissions, PUTs, versions and
//! batches all run them.

use std::collections::{BTreeMap, BTreeSet};

use analyzer::{Analysis, is_valid_slug, slug_references};

use crate::AppState;
use crate::auth::TokenAccount;
use crate::entries;
use crate::error::AppError;

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
    pub source: &'a str,
    pub slug_rule: SlugRule,
    /// The stored entry a PUT replaces, so it doesn't count as referring to itself.
    pub seq: Option<i64>,
}

/// What's wrong with a set of entries' slugs, runs and references, given the runs a
/// batch declares. Empty when nothing is.
pub async fn problems(
    state: &AppState,
    account: &TokenAccount,
    members: &[Linked<'_>],
    declared_runs: &BTreeSet<String>,
) -> Result<Vec<String>, AppError> {
    let mut problems = Vec::new();
    // Each slug in this set, with the embargo of the entry that has it.
    let mut slugs_here: BTreeMap<String, Option<String>> = BTreeMap::new();
    for member in members {
        let Some(slug) = &member.analysis.fields.slug else {
            continue;
        };
        if !is_valid_slug(slug) {
            problems.push(format!(
                "{}: a slug is 1 to 64 letters, digits, `.`, `_` or `-`",
                member.label
            ));
        } else if slugs_here.contains_key(slug) {
            problems.push(format!(
                "{}: slug `{slug}` is used twice in this batch",
                member.label
            ));
        }
        slugs_here.insert(slug.clone(), member.analysis.fields.embargo_until.clone());
    }
    for member in members {
        problems.extend(slug_problems(state, account, member).await?);
        problems.extend(run_problems(state, account, member, declared_runs).await?);
        problems.extend(reference_problems(state, account, member, &slugs_here).await?);
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
    declared_runs: &BTreeSet<String>,
) -> Result<Vec<String>, AppError> {
    let mut problems = Vec::new();
    for run in &member.analysis.fields.uses_runs {
        if !declared_runs.contains(run)
            && !entries::run_declared(&state.pool, &account.id, run).await?
        {
            problems.push(format!(
                "{}: `uses_runs` names `{run}`, which neither this batch nor an earlier one declares",
                member.label
            ));
        }
    }
    Ok(problems)
}

/// An entry may not become public before an entry it refers to: the reference would
/// reveal that the other exists. Checked both ways: what this entry refers to, and
/// what already refers to this entry's slug.
async fn reference_problems(
    state: &AppState,
    account: &TokenAccount,
    member: &Linked<'_>,
    slugs_here: &BTreeMap<String, Option<String>>,
) -> Result<Vec<String>, AppError> {
    let mut problems = Vec::new();
    let own = member.analysis.fields.embargo_until.as_deref();
    let before = |earlier: Option<&str>, later: &str| earlier.is_none_or(|earlier| earlier < later);
    for name in slug_references(member.source) {
        let target = match slugs_here.get(&name) {
            Some(embargo) => embargo.clone(),
            None => entries::slug_embargo(&state.pool, &account.id, &name).await?,
        };
        if let Some(target) = target
            && before(own, &target)
        {
            problems.push(format!(
                "{}: it refers to slug:{name}, which is embargoed until {target}; an entry may not become public before an entry it refers to",
                member.label
            ));
        }
    }
    let own_slug = match (&member.analysis.fields.slug, &member.slug_rule) {
        (Some(slug), _) => Some(slug.clone()),
        (None, SlugRule::Version(kept) | SlugRule::Put(kept)) => kept.clone(),
        (None, SlugRule::New) => None,
    };
    if let (Some(slug), Some(own)) = (own_slug, own) {
        for (seq, id, embargo) in entries::referrers(&state.pool, &account.id, &slug).await? {
            if Some(seq) != member.seq && before(embargo.as_deref(), own) {
                problems.push(format!(
                    "{}: entry {id} refers to it and would become public first; an entry can't take an embargo while an earlier-public entry refers to it",
                    member.label
                ));
            }
        }
    }
    Ok(problems)
}
