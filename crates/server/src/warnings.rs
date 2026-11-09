//! The warnings a submission's reply and its author's read of a pending entry list
//! (design.md, Agent interface): the analyzer's, the link checks', and two only the
//! server can give, since they need the entry's attachment set or other entries:
//! files its body names that a reader can't open, and `replicates`/`reanalyzes`
//! items that name no entry.

use analyzer::{Analysis, Reference, WARNING_FEEDBACK, file_mentions};
use sqlx::SqlitePool;

use crate::attachments;
use crate::entries;
use crate::link_checks::{self, CheckedLink, Field, Reach};
use crate::respond::one_line;

/// How many files the warning about files a reader can't open names.
const NAMED_FILES: usize = 10;

/// Every warning for stored entry `seq` of `account_id`, whose text is `source`, read
/// as `analysis`, with its checked `links`.
pub async fn entry_warnings(
    pool: &SqlitePool,
    account_id: &str,
    seq: i64,
    source: &str,
    analysis: &Analysis,
    links: &[CheckedLink],
) -> Result<Vec<String>, sqlx::Error> {
    let mut warnings: Vec<String> = analysis.warnings.iter().map(ToString::to_string).collect();
    warnings.extend(link_checks::warnings(links));
    warnings.extend(unopenable_files(pool, seq, source, analysis, links).await?);
    warnings.extend(references_naming_nothing(pool, account_id, analysis).await?);
    Ok(warnings)
}

/// The files the body names in code spans that are neither in the entry's attachment
/// set (files the owner removed don't count) nor answered for by a `code` link that
/// isn't dead, since a pinned repo may hold them.
async fn unopenable_files(
    pool: &SqlitePool,
    seq: i64,
    source: &str,
    analysis: &Analysis,
    links: &[CheckedLink],
) -> Result<Option<String>, sqlx::Error> {
    let code_dead = links
        .iter()
        .any(|link| link.field == Field::Code && matches!(link.reach, Reach::Dead(_)));
    if analysis.fields.code.is_some() && !code_dead {
        return Ok(None);
    }
    let named = file_mentions(source);
    if named.is_empty() {
        return Ok(None);
    }
    let attached: Vec<String> = attachments::entry_files(pool, seq)
        .await?
        .into_iter()
        .filter(|file| !file.removed)
        .map(|file| file.path)
        .collect();
    let missing: Vec<&String> = named
        .iter()
        .filter(|name| !attached.iter().any(|path| same_file(path, name)))
        .collect();
    if missing.is_empty() {
        return Ok(None);
    }
    let mut list = missing
        .iter()
        .take(NAMED_FILES)
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if missing.len() > NAMED_FILES {
        list.push_str(&format!(" and {} more", missing.len() - NAMED_FILES));
    }
    Ok(Some(format!(
        "the body names {list}, which aren't in the attachment set; a reader can open only attached files and links, so if the entry relies on them, attach them, link them, or quote what it needs from them (a sentence saying a file was left out is fine) {WARNING_FEEDBACK}"
    )))
}

/// Whether attached `path` is the file `named`: the same path, or one ends with the
/// other at a `/`, as `src/x.py` and `x.py` are.
fn same_file(path: &str, named: &str) -> bool {
    let ends_with = |long: &str, short: &str| {
        long == short
            || long
                .strip_suffix(short)
                .is_some_and(|before| before.ends_with('/'))
    };
    ends_with(path, named) || ends_with(named, path)
}

/// A warning for each `replicates` or `reanalyzes` item that names an entry the
/// archive doesn't hold: a slug none of the account's entries has, or an ID of no
/// public entry and none of the account's own. Another account's hidden entry reads
/// as no entry, so the warning tells nothing about it.
async fn references_naming_nothing(
    pool: &SqlitePool,
    account_id: &str,
    analysis: &Analysis,
) -> Result<Vec<String>, sqlx::Error> {
    let mut warnings = Vec::new();
    for (name, items) in analysis.fields.follow_ups() {
        for item in items {
            let warning = match Reference::parse(item) {
                Some(Reference::Slug(slug))
                    if !entries::slug_taken(pool, account_id, &slug).await? =>
                {
                    format!(
                        "`{name}` item `slug:{slug}` names none of this account's entries; a slug works only within its account, so use the entry's ID for another account's entry"
                    )
                }
                Some(Reference::Entry(id))
                    if !entries::public_or_own(pool, account_id, &id).await? =>
                {
                    format!(
                        "`{name}` item `{}` names no public entry; check the ID",
                        one_line(&id)
                    )
                }
                _ => continue,
            };
            warnings.push(format!("{warning} {WARNING_FEEDBACK}"));
        }
    }
    Ok(warnings)
}
