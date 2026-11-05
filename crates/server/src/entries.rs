//! Storing entries and reading them back (design.md, Entry status; Agent interface).

use analyzer::{
    Analysis, Author, CardContext, Choice, Claim, Kind, Outcome, Results, Role, Stage, Vocabulary,
    analyze, render_card,
};
use sqlx::SqlitePool;

use crate::{ids, time::now};

/// A public entry with its author, as reads need it.
pub struct PublicEntry {
    pub id: String,
    pub source: String,
    pub account_id: String,
    pub display_name: String,
    pub submitted_at: String,
    pub published_at: Option<String>,
}

/// The card context only the server knows: the entry's ID and its author.
pub fn card_context(id: &str, account_id: &str, display_name: &str) -> CardContext {
    CardContext {
        id: Some(id.to_string()),
        author: Some(Author {
            display_name: display_name.to_string(),
            account_id: account_id.to_string(),
            handles: Vec::new(),
        }),
        ..CardContext::default()
    }
}

/// Where screening sent an entry that passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    Public,
    Embargoed,
}

impl Placed {
    /// The status as the database and responses write it (design.md, Entry status).
    pub fn status(self) -> &'static str {
        match self {
            Placed::Public => "public",
            Placed::Embargoed => "embargoed",
        }
    }
}

impl PublicEntry {
    /// The card, rendered from the stored text by the current analyzer.
    pub fn card(&self) -> String {
        let context = card_context(&self.id, &self.account_id, &self.display_name);
        render_card(&analyze(&self.source), &context)
    }
}

/// The columns search filters on, as an analysis gives them.
struct Columns {
    outcome: Option<&'static str>,
    kind: Option<&'static str>,
    stage: Option<&'static str>,
    version: i64,
}

fn columns(analysis: &Analysis) -> Columns {
    let fields = &analysis.fields;
    Columns {
        outcome: fields.outcome.known().map(|outcome| outcome.name()),
        kind: match &fields.kind {
            Choice::Absent => Some("experiment"),
            kind => kind.known().map(|kind| kind.name()),
        },
        stage: fields.stage.known().map(|stage| stage.name()),
        version: i64::from(analyzer::ANALYZER_VERSION),
    }
}

/// Stores an accepted submission as `pending` until `hold_until`, with the columns
/// search filters on, and returns its row number and ID.
pub async fn insert(
    pool: &SqlitePool,
    account_id: &str,
    source: &str,
    analysis: &Analysis,
    hold_until: &str,
) -> Result<(i64, String), sqlx::Error> {
    let id = ids::new_id();
    let submitted_at = now();
    let columns = columns(analysis);
    let mut transaction = pool.begin().await?;
    let seq = sqlx::query_scalar!(
        r#"INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
               embargo_until, analyzer_version, outcome, kind, stage, hold_until)
           VALUES (?, ?, 'pending', ?, ?, ?, ?, ?, ?, ?, ?, ?)
           RETURNING seq AS "seq!: i64""#,
        id,
        account_id,
        source,
        submitted_at,
        submitted_at,
        analysis.fields.embargo_until,
        columns.version,
        columns.outcome,
        columns.kind,
        columns.stage,
        hold_until
    )
    .fetch_one(&mut *transaction)
    .await?;
    write_derived(&mut transaction, seq, analysis).await?;
    transaction.commit().await?;
    Ok((seq, id))
}

/// Replaces a pending entry's text and everything read from it, and restarts its
/// hold. Its ID and first submission date stay. Returns false if it is no longer
/// pending.
pub async fn replace(
    pool: &SqlitePool,
    seq: i64,
    source: &str,
    analysis: &Analysis,
    hold_until: &str,
) -> Result<bool, sqlx::Error> {
    let submitted_at = now();
    let columns = columns(analysis);
    let mut transaction = pool.begin().await?;
    let done = sqlx::query!(
        "UPDATE entries SET source = ?, submitted_at = ?, embargo_until = ?,
             analyzer_version = ?, outcome = ?, kind = ?, stage = ?, hold_until = ?
         WHERE seq = ? AND status = 'pending'",
        source,
        submitted_at,
        analysis.fields.embargo_until,
        columns.version,
        columns.outcome,
        columns.kind,
        columns.stage,
        hold_until,
        seq
    )
    .execute(&mut *transaction)
    .await?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query!("DELETE FROM entry_models WHERE entry_seq = ?", seq)
        .execute(&mut *transaction)
        .await?;
    sqlx::query!("DELETE FROM entry_tags WHERE entry_seq = ?", seq)
        .execute(&mut *transaction)
        .await?;
    sqlx::query!("DELETE FROM entry_rows WHERE entry_seq = ?", seq)
        .execute(&mut *transaction)
        .await?;
    sqlx::query!("DELETE FROM entry_unknown_fields WHERE entry_seq = ?", seq)
        .execute(&mut *transaction)
        .await?;
    write_derived(&mut transaction, seq, analysis).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Stores the models, tags, rows and unknown field names read from an entry.
async fn write_derived(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
    analysis: &Analysis,
) -> Result<(), sqlx::Error> {
    let fields = &analysis.fields;
    for model in &fields.models {
        let Some(name) = &model.name else { continue };
        let role = match &model.role {
            Choice::Absent => None,
            role => Some(role.known().map_or("unrecognised", Role::name)),
        };
        sqlx::query!(
            "INSERT INTO entry_models (entry_seq, name, role) VALUES (?, ?, ?)",
            seq,
            name,
            role
        )
        .execute(&mut *transaction)
        .await?;
    }
    for tag in &fields.tags {
        sqlx::query!(
            "INSERT INTO entry_tags (entry_seq, tag) VALUES (?, ?)",
            seq,
            tag
        )
        .execute(&mut *transaction)
        .await?;
    }
    if let Results::Read(table) = &analysis.results {
        for (position, row) in (1_i64..).zip(&table.rows) {
            let model = row.cell("model");
            let claim = row.claim.map(|claim| claim.name());
            sqlx::query!(
                "INSERT INTO entry_rows (entry_seq, position, model, claim) VALUES (?, ?, ?, ?)",
                seq,
                position,
                model,
                claim
            )
            .execute(&mut *transaction)
            .await?;
        }
    }
    for name in &analysis.unrecognised_fields {
        sqlx::query!(
            "INSERT INTO entry_unknown_fields (entry_seq, name) VALUES (?, ?)",
            seq,
            name
        )
        .execute(&mut *transaction)
        .await?;
    }
    Ok(())
}

/// An entry that isn't public, as its author's token sees it.
pub struct OwnEntry {
    pub seq: i64,
    pub id: String,
    pub status: String,
    pub source: String,
    pub hold_until: Option<String>,
    pub embargo_until: Option<String>,
    /// The embargo cap counts from here (design.md, Entry lifecycle).
    pub first_submitted_at: String,
}

/// The entry `id` if `account_id` wrote it.
pub async fn own_entry(
    pool: &SqlitePool,
    id: &str,
    account_id: &str,
) -> Result<Option<OwnEntry>, sqlx::Error> {
    sqlx::query_as!(
        OwnEntry,
        r#"SELECT seq AS "seq!: i64", id, status, source, hold_until, embargo_until,
                  first_submitted_at
           FROM entries WHERE id = ? AND account_id = ?"#,
        id,
        account_id
    )
    .fetch_optional(pool)
    .await
}

/// Ends a pending entry's hold: moves it to `screening` and returns its text, or
/// `None` if something else ended the hold first. With `due_by`, only if the hold
/// ends by then, so a PUT that restarted it since the entry was listed as due wins.
pub async fn start_screening(
    pool: &SqlitePool,
    seq: i64,
    due_by: Option<&str>,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "UPDATE entries SET status = 'screening', hold_until = NULL
         WHERE seq = ?1 AND status = 'pending' AND (?2 IS NULL OR hold_until <= ?2)
         RETURNING source",
        seq,
        due_by
    )
    .fetch_optional(pool)
    .await
}

/// Entries in `screening` and their text. With the pass-all screen, one is still
/// there only when placing it after screening failed, so it is placed again.
pub async fn in_screening(pool: &SqlitePool) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT seq AS "seq!: i64", source FROM entries WHERE status = 'screening'"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| (row.seq, row.source)).collect())
}

/// The pending entries whose hold ends at or before `now`.
pub async fn due(pool: &SqlitePool, now: &str) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries WHERE status = 'pending' AND hold_until <= ?"#,
        now
    )
    .fetch_all(pool)
    .await
}

/// Moves a screened entry to `embargoed`.
/// `None` if the entry was no longer in `screening`: something else placed it.
pub async fn embargo(pool: &SqlitePool, seq: i64) -> Result<Option<Placed>, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE entries SET status = 'embargoed' WHERE seq = ? AND status = 'screening'",
        seq
    )
    .execute(pool)
    .await?;
    Ok((done.rows_affected() == 1).then_some(Placed::Embargoed))
}

/// Makes a screened entry public and adds it to the search index.
pub async fn publish(
    pool: &SqlitePool,
    seq: i64,
    analysis: &Analysis,
) -> Result<Option<Placed>, sqlx::Error> {
    let published_at = now();
    let fields = &analysis.fields;
    let mut transaction = pool.begin().await?;
    // Only from `screening`, so an entry placed twice is indexed once.
    let Some(source) = sqlx::query_scalar!(
        "UPDATE entries SET status = 'public', published_at = ?
         WHERE seq = ? AND status = 'screening' RETURNING source",
        published_at,
        seq
    )
    .fetch_optional(&mut *transaction)
    .await?
    else {
        return Ok(None);
    };
    sqlx::query!(
        "INSERT INTO entries_fts (rowid, title, hypothesis, safety_motivation, body)
         VALUES (?, ?, ?, ?, ?)",
        seq,
        fields.title,
        fields.hypothesis,
        fields.safety_motivation,
        source
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(Placed::Public))
}

/// A public entry by ID.
pub async fn public_entry(pool: &SqlitePool, id: &str) -> Result<Option<PublicEntry>, sqlx::Error> {
    sqlx::query_as!(
        PublicEntry,
        "SELECT e.id, e.source, e.account_id, a.display_name, e.submitted_at, e.published_at
         FROM entries e JOIN accounts a ON a.id = e.account_id
         WHERE e.id = ? AND e.status = 'public'",
        id
    )
    .fetch_optional(pool)
    .await
}

/// What `/search` filters on. `None` means no filter.
#[derive(Debug, Default)]
pub struct Filters {
    /// An FTS5 query, already made safe by `fts_query`.
    pub words: Option<String>,
    pub outcome: Option<Outcome>,
    pub kind: Option<Kind>,
    pub stage: Option<Stage>,
    pub tag: Option<String>,
    /// A model that was a subject (or had no role given).
    pub model: Option<String>,
    pub row_model: Option<String>,
    pub row_claim: Option<Claim>,
}

/// Public entries matching `filters`: best match first when there are search words
/// (bm25, with the title weighted most), otherwise newest first.
pub async fn search(
    pool: &SqlitePool,
    filters: &Filters,
    limit: i64,
    offset: i64,
) -> Result<Vec<PublicEntry>, sqlx::Error> {
    let outcome = filters.outcome.map(Outcome::name);
    let kind = filters.kind.map(Kind::name);
    let stage = filters.stage.map(Stage::name);
    let row_claim = filters.row_claim.map(Claim::name);
    sqlx::query_as!(
        PublicEntry,
        r#"SELECT e.id, e.source, e.account_id, a.display_name, e.submitted_at, e.published_at
           FROM entries e JOIN accounts a ON a.id = e.account_id
           WHERE e.status = 'public'
             AND (?1 IS NULL OR e.seq IN (SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?1))
             AND (?2 IS NULL OR e.outcome = ?2)
             AND (?3 IS NULL OR e.kind = ?3)
             AND (?4 IS NULL OR e.stage = ?4)
             AND (?5 IS NULL OR e.seq IN (SELECT entry_seq FROM entry_tags WHERE tag = ?5))
             AND (?6 IS NULL OR e.seq IN (SELECT entry_seq FROM entry_models
                                          WHERE name = ?6 AND (role IS NULL OR role = 'subject')))
             AND ((?7 IS NULL AND ?8 IS NULL) OR e.seq IN (
                    SELECT entry_seq FROM entry_rows
                    WHERE (?7 IS NULL OR model = ?7) AND (?8 IS NULL OR claim = ?8)))
           ORDER BY
             -- CASE runs the MATCH only when there are words; MATCH NULL is an error.
             CASE WHEN ?1 IS NULL THEN NULL ELSE (
               SELECT bm25(entries_fts, 10.0, 5.0, 2.0, 1.0) FROM entries_fts
               WHERE entries_fts MATCH ?1 AND rowid = e.seq) END,
             e.published_at DESC, e.seq DESC
           LIMIT ?9 OFFSET ?10"#,
        filters.words,
        outcome,
        kind,
        stage,
        filters.tag,
        filters.model,
        filters.row_model,
        row_claim,
        limit,
        offset
    )
    .fetch_all(pool)
    .await
}

/// Every public entry, oldest first, for `/cards.txt`.
pub async fn all_public(pool: &SqlitePool) -> Result<Vec<PublicEntry>, sqlx::Error> {
    sqlx::query_as!(
        PublicEntry,
        "SELECT e.id, e.source, e.account_id, a.display_name, e.submitted_at, e.published_at
         FROM entries e JOIN accounts a ON a.id = e.account_id
         WHERE e.status = 'public' ORDER BY e.seq"
    )
    .fetch_all(pool)
    .await
}

/// Tags on public entries, most used first.
pub async fn tags(pool: &SqlitePool) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT t.tag, COUNT(*) AS "count!: i64" FROM entry_tags t
           JOIN entries e ON e.seq = t.entry_seq
           WHERE e.status = 'public'
           GROUP BY t.tag ORDER BY 2 DESC, t.tag"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| (row.tag, row.count)).collect())
}

/// Turns free text into an FTS5 query that matches any of its words, each quoted so a
/// reader's text is never read as query syntax. Agents ask in sentences, and bm25
/// ranks entries with more of the words, and rarer ones, first. `None` when it has no
/// words.
pub fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}
