//! Storing entries and reading them back (design.md, Entry status; Agent interface).

use analyzer::{
    Analysis, Author, CardContext, Choice, Claim, Kind, Outcome, Results, Role, Stage, Vocabulary,
    analyze, render_card,
};
use sqlx::SqlitePool;

use crate::screening::Verdict;
use crate::series::Cited;
use crate::{allowance, ids, time::now};

/// A public entry with its author, as reads need it.
pub struct PublicEntry {
    pub seq: i64,
    pub id: String,
    pub source: String,
    pub account_id: String,
    /// `None` once the account is deleted.
    pub display_name: Option<String>,
    pub submitted_at: String,
    pub published_at: Option<String>,
    /// The version this one replaced.
    pub previous_id: Option<String>,
    /// The public version that replaced this one.
    pub superseded_by: Option<String>,
    /// The screen's doubt that the body matches `outcome`, for the card.
    pub screening_note: Option<String>,
    /// It cites an attachment set.
    pub code_attached: bool,
}

/// The card context only the server knows: the entry's ID and its author, whose
/// name is `None` once the account is deleted.
pub fn card_context(id: &str, account_id: &str, display_name: Option<&str>) -> CardContext {
    CardContext {
        id: Some(id.to_string()),
        author: Some(Author {
            display_name: display_name.map(str::to_string),
            account_id: account_id.to_string(),
            handles: Vec::new(),
        }),
        ..CardContext::default()
    }
}

/// The card context of an entry not yet public, as its author sees it: its
/// attachment set, checked at submission, means "code attached".
pub fn own_card_context(
    id: &str,
    account_id: &str,
    display_name: &str,
    analysis: &Analysis,
) -> CardContext {
    CardContext {
        code_attached: analysis.fields.attachments.is_some(),
        ..card_context(id, account_id, Some(display_name))
    }
}

/// Where screening sent an entry that passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    Public,
    Embargoed,
    /// Passed, but its series, or one another member of its batch cites, is hidden.
    WaitingOnSeries,
}

impl PublicEntry {
    /// The card, rendered from the stored text by the current analyzer.
    pub fn card(&self) -> String {
        self.card_with(self.superseded_by.clone())
    }

    /// The card, with `superseded_by` as the server's record of what replaced it.
    pub fn card_with(&self, superseded_by: Option<String>) -> String {
        let context = CardContext {
            superseded_by_version: superseded_by,
            screening_note: self.screening_note.clone(),
            code_attached: self.code_attached,
            ..card_context(&self.id, &self.account_id, self.display_name.as_deref())
        };
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

/// The entry a new version replaces: its row, and the first submission date of its
/// chain, which the version keeps (design.md, Entry lifecycle).
pub struct Previous<'a> {
    pub seq: i64,
    pub first_submitted_at: &'a str,
}

/// An accepted submission, as `insert_on` stores it.
pub struct NewEntry<'a> {
    pub account_id: &'a str,
    pub source: &'a str,
    pub analysis: &'a Analysis,
    /// When its hold ends.
    pub hold_until: &'a str,
    /// The entry it is a new version of.
    pub previous: Option<Previous<'a>>,
    /// The batch it came in.
    pub batch_seq: Option<i64>,
    /// Whether its `slug` is new for the account: a first submission's, or one a new
    /// version gives a chain that had none. Checked as free beforehand.
    pub register_slug: bool,
    /// The series version it cites, checked beforehand.
    pub series: Option<&'a Cited>,
    /// The attachment set it cites, checked beforehand to be its account's.
    pub attachments: Option<i64>,
}

/// Stores an accepted submission as `pending` until its hold ends, with the columns
/// search filters on, in the caller's transaction, and returns its row number and ID.
/// A batch stores all its members in one. A first version with a `slug` registers it
/// for the account, and the entry's `uses_runs` are recorded; both were checked
/// beforehand.
pub async fn insert_on(
    transaction: &mut sqlx::SqliteConnection,
    entry: &NewEntry<'_>,
) -> Result<(i64, String), sqlx::Error> {
    let NewEntry {
        account_id,
        source,
        analysis,
        hold_until,
        ref previous,
        batch_seq,
        register_slug,
        series,
        attachments,
    } = *entry;
    let id = ids::new_id();
    let submitted_at = now();
    let first_submitted_at = previous
        .as_ref()
        .map_or(submitted_at.as_str(), |p| p.first_submitted_at);
    let previous_seq = previous.as_ref().map(|p| p.seq);
    let columns = columns(analysis);
    let (series_seq, series_version, series_section) = series_columns(series);
    let seq = sqlx::query_scalar!(
        r#"INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
               embargo_until, analyzer_version, outcome, kind, stage, hold_until, previous_seq,
               batch_seq, series_seq, series_version, series_section, attachment_set_seq)
           VALUES (?, ?, 'pending', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           RETURNING seq AS "seq!: i64""#,
        id,
        account_id,
        source,
        submitted_at,
        first_submitted_at,
        analysis.fields.embargo_until,
        columns.version,
        columns.outcome,
        columns.kind,
        columns.stage,
        hold_until,
        previous_seq,
        batch_seq,
        series_seq,
        series_version,
        series_section,
        attachments
    )
    .fetch_one(&mut *transaction)
    .await?;
    write_derived(&mut *transaction, seq, analysis, previous_seq).await?;
    if let (Some(slug), true) = (&analysis.fields.slug, register_slug) {
        sqlx::query!(
            "INSERT INTO slugs (account_id, slug, entry_seq) VALUES (?, ?, ?)",
            account_id,
            slug,
            seq
        )
        .execute(&mut *transaction)
        .await?;
    }
    write_runs(&mut *transaction, seq, account_id, analysis).await?;
    Ok((seq, id))
}

/// The entry columns that record a cited series version.
fn series_columns(series: Option<&Cited>) -> (Option<i64>, Option<i64>, Option<&str>) {
    match series {
        Some(cited) => (
            Some(cited.series_seq),
            Some(cited.version),
            cited.section.as_deref(),
        ),
        None => (None, None, None),
    }
}

/// Records which declared runs an entry uses.
async fn write_runs(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
    account_id: &str,
    analysis: &Analysis,
) -> Result<(), sqlx::Error> {
    for run in &analysis.fields.uses_runs {
        sqlx::query!(
            "INSERT INTO entry_runs (entry_seq, account_id, run_id) VALUES (?, ?, ?)",
            seq,
            account_id,
            run
        )
        .execute(&mut *transaction)
        .await?;
    }
    Ok(())
}

/// Replaces a pending entry's text and everything read from it, and restarts its
/// hold, in the caller's transaction. Its ID and first submission date stay. Returns
/// false if it is no longer pending.
pub async fn replace_on(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
    source: &str,
    analysis: &Analysis,
    hold_until: &str,
    series: Option<&Cited>,
    attachments: Option<i64>,
) -> Result<bool, sqlx::Error> {
    let submitted_at = now();
    let columns = columns(analysis);
    let (series_seq, series_version, series_section) = series_columns(series);
    let done = sqlx::query!(
        "UPDATE entries SET source = ?, submitted_at = ?, embargo_until = ?,
             analyzer_version = ?, outcome = ?, kind = ?, stage = ?, hold_until = ?,
             series_seq = ?, series_version = ?, series_section = ?, series_removed_at = NULL,
             attachment_set_seq = ?
         WHERE seq = ? AND status = 'pending'",
        source,
        submitted_at,
        analysis.fields.embargo_until,
        columns.version,
        columns.outcome,
        columns.kind,
        columns.stage,
        hold_until,
        series_seq,
        series_version,
        series_section,
        attachments,
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
    sqlx::query!("DELETE FROM entry_runs WHERE entry_seq = ?", seq)
        .execute(&mut *transaction)
        .await?;
    let kept = sqlx::query!(
        "SELECT previous_seq, batch_seq, account_id FROM entries WHERE seq = ?",
        seq
    )
    .fetch_one(&mut *transaction)
    .await?;
    write_derived(&mut *transaction, seq, analysis, kept.previous_seq).await?;
    write_runs(&mut *transaction, seq, &kept.account_id, analysis).await?;
    // A batch is held as one: a PUT on any member restarts every member's hold.
    if let Some(batch_seq) = kept.batch_seq {
        sqlx::query!(
            "UPDATE entries SET hold_until = ? WHERE batch_seq = ? AND status = 'pending'",
            hold_until,
            batch_seq
        )
        .execute(&mut *transaction)
        .await?;
    }
    Ok(true)
}

/// Stores the models, tags, rows and unknown field names read from an entry.
/// A version that points to a paper and has no Results table of its own carries the
/// rows of the version before it, which carried its own predecessor's, so search
/// still finds the chain's newest numbers (design.md, Entry lifecycle).
async fn write_derived(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
    analysis: &Analysis,
    previous_seq: Option<i64>,
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
    } else if let (Some(previous), Some(_), None) = (
        previous_seq,
        &analysis.fields.published_in,
        &analysis.sections.results,
    ) {
        // Only with no `## Results` section at all: one the analyzer can't read is
        // still the version's own, and may retract the numbers.
        sqlx::query!(
            "INSERT INTO entry_rows (entry_seq, position, model, claim)
             SELECT ?, position, model, claim FROM entry_rows WHERE entry_seq = ?",
            seq,
            previous
        )
        .execute(&mut *transaction)
        .await?;
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
    /// The version this one replaced.
    pub previous_id: Option<String>,
    /// The batch it came in, which publishes as one.
    pub batch_seq: Option<i64>,
    /// Why screening rejected it, a JSON array of strings.
    pub rejection_reasons: Option<String>,
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
                  first_submitted_at,
                  (SELECT p.id FROM entries p WHERE p.seq = entries.previous_seq) AS "previous_id?",
                  batch_seq, rejection_reasons
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
    to_screening(pool, seq, "pending", due_by).await
}

/// Moves entry `seq` from `from` to `screening`, as a hold's end does: it joins the
/// screening queue one place behind its account's last waiting entry (design.md: the
/// fair queue; `to_review` places it in the review queue alike), and its hold is
/// over. With `due_by`, only if its hold ends by then.
async fn to_screening(
    pool: &SqlitePool,
    seq: i64,
    from: &str,
    due_by: Option<&str>,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "UPDATE entries SET status = 'screening', hold_until = NULL, screen_verdict = NULL,
             queue_position = 1 + COALESCE((
                 SELECT MAX(w.queue_position) FROM entries w
                 WHERE w.account_id = entries.account_id AND w.status = 'screening'
                   AND w.screen_verdict IS NULL), 0)
         WHERE seq = ?1 AND status = ?3 AND (?2 IS NULL OR hold_until <= ?2)
         RETURNING source",
        seq,
        due_by,
        from
    )
    .fetch_optional(pool)
    .await
}

/// Entries with `status` and their text: those in `screening`, which with the
/// pass-all screen are there only when placing them failed, and those in
/// `waiting_on_series`.
pub async fn in_status(pool: &SqlitePool, status: &str) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT seq AS "seq!: i64", source FROM entries WHERE status = ?"#,
        status
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| (row.seq, row.source)).collect())
}

/// Moves a screened entry to `waiting_on_series`.
/// `None` if the entry was no longer in `screening`: something else placed it.
pub async fn wait_on_series(pool: &SqlitePool, seq: i64) -> Result<Option<Placed>, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE entries SET status = 'waiting_on_series'
         WHERE seq = ? AND status = 'screening' AND screen_verdict = 'pass'",
        seq
    )
    .execute(pool)
    .await?;
    Ok((done.rows_affected() == 1).then_some(Placed::WaitingOnSeries))
}

/// The pending entries whose hold ends at or before `now`.
pub async fn due(pool: &SqlitePool, now: &str) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries WHERE status = 'pending' AND hold_until <= ?
           ORDER BY hold_until, seq"#,
        now
    )
    .fetch_all(pool)
    .await
}

/// Moves a screened entry to `embargoed`.
/// `None` if the entry was no longer in `screening`: something else placed it.
pub async fn embargo(pool: &SqlitePool, seq: i64) -> Result<Option<Placed>, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE entries SET status = 'embargoed'
         WHERE seq = ? AND status = 'screening' AND screen_verdict = 'pass'",
        seq
    )
    .execute(pool)
    .await?;
    Ok((done.rows_affected() == 1).then_some(Placed::Embargoed))
}

/// Makes a screened entry, or one that waited on its series, public and adds it to
/// the search index.
pub async fn publish(
    pool: &SqlitePool,
    seq: i64,
    analysis: &Analysis,
) -> Result<Option<Placed>, sqlx::Error> {
    let published_at = now();
    let fields = &analysis.fields;
    let mut transaction = pool.begin().await?;
    // Only from `screening` or `waiting_on_series`, so an entry placed twice is
    // indexed once.
    let Some(source) = sqlx::query_scalar!(
        "UPDATE entries SET status = 'public', published_at = ?
         WHERE seq = ? AND screen_verdict = 'pass' AND status IN ('screening', 'waiting_on_series')
         RETURNING source",
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
    crate::series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(Some(Placed::Public))
}

/// A public entry by ID.
pub async fn public_entry(pool: &SqlitePool, id: &str) -> Result<Option<PublicEntry>, sqlx::Error> {
    sqlx::query_as!(
        PublicEntry,
        r#"SELECT e.seq AS "seq!: i64", e.id, e.source, e.account_id,
                  a.display_name AS "display_name?: String", e.submitted_at, e.published_at,
                  (SELECT p.id FROM entries p WHERE p.seq = e.previous_seq AND p.status = 'public') AS "previous_id?",
                  (SELECT n.id FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public') AS "superseded_by?",
                  e.screening_note,
                  e.attachment_set_seq IS NOT NULL AS "code_attached!: bool"
         FROM entries e JOIN authors a ON a.id = e.account_id
         WHERE e.id = ? AND e.status = 'public'"#,
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
        r#"SELECT e.seq AS "seq!: i64", e.id, e.source, e.account_id,
                  a.display_name AS "display_name?: String", e.submitted_at, e.published_at,
                  (SELECT p.id FROM entries p WHERE p.seq = e.previous_seq AND p.status = 'public') AS "previous_id?",
                  (SELECT n.id FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public') AS "superseded_by?",
                  e.screening_note,
                  e.attachment_set_seq IS NOT NULL AS "code_attached!: bool"
           FROM entries e JOIN authors a ON a.id = e.account_id
           WHERE e.status = 'public'
             -- Only the latest version in each chain (design.md, Entry lifecycle).
             AND NOT EXISTS (SELECT 1 FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public')
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
        r#"SELECT e.seq AS "seq!: i64", e.id, e.source, e.account_id,
                  a.display_name AS "display_name?: String", e.submitted_at, e.published_at,
                  (SELECT p.id FROM entries p WHERE p.seq = e.previous_seq AND p.status = 'public') AS "previous_id?",
                  (SELECT n.id FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public') AS "superseded_by?",
                  e.screening_note,
                  e.attachment_set_seq IS NOT NULL AS "code_attached!: bool"
         FROM entries e JOIN authors a ON a.id = e.account_id
         WHERE e.status = 'public'
           AND NOT EXISTS (SELECT 1 FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public')
         ORDER BY e.seq"#
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
             AND NOT EXISTS (SELECT 1 FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public')
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

/// The live version that replaced entry `seq`, if any: its ID and status. A rejected,
/// withdrawn or removed version doesn't count.
pub async fn successor(
    pool: &SqlitePool,
    seq: i64,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT id, status FROM entries
         WHERE previous_seq = ? AND status NOT IN ('rejected', 'withdrawn', 'removed')",
        seq
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| (row.id, row.status)))
}

/// The entry a slug names for `account_id`: the latest public version of the chain
/// it was given to, if one is public.
pub async fn public_by_slug(
    pool: &SqlitePool,
    account_id: &str,
    slug: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"WITH RECURSIVE chain(seq) AS (
               SELECT entry_seq FROM slugs WHERE account_id = ?1 AND slug = ?2
               UNION ALL
               SELECT e.seq FROM entries e JOIN chain c ON e.previous_seq = c.seq
           )
           SELECT e.id AS "id!" FROM entries e JOIN chain c ON c.seq = e.seq
           WHERE e.status = 'public' ORDER BY e.seq DESC LIMIT 1"#,
        account_id,
        slug
    )
    .fetch_optional(pool)
    .await
}

/// Whether `account_id` has given out `slug`.
pub async fn slug_taken(
    pool: &SqlitePool,
    account_id: &str,
    slug: &str,
) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar!(
        "SELECT 1 AS found FROM slugs WHERE account_id = ? AND slug = ?",
        account_id,
        slug
    )
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// The embargo date of the chain a slug names, if its latest version is held back:
/// for the rule that a public entry may not refer to an embargoed one.
pub async fn slug_embargo(
    pool: &SqlitePool,
    account_id: &str,
    slug: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query!(
        r#"WITH RECURSIVE chain(seq) AS (
               SELECT entry_seq FROM slugs WHERE account_id = ?1 AND slug = ?2
               UNION ALL
               SELECT e.seq FROM entries e JOIN chain c ON e.previous_seq = c.seq
           )
           SELECT e.embargo_until FROM entries e JOIN chain c ON c.seq = e.seq
           WHERE e.status IN ('pending', 'screening', 'embargoed')
           ORDER BY e.seq DESC LIMIT 1"#,
        account_id,
        slug
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|row| row.embargo_until))
}

/// Whether `account_id` has declared run `run_id`.
pub async fn run_declared(
    pool: &SqlitePool,
    account_id: &str,
    run_id: &str,
) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar!(
        "SELECT 1 AS found FROM runs WHERE account_id = ? AND id = ?",
        account_id,
        run_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// The runs an entry uses, each with its description once every entry using it is
/// public (design.md, Agent interface), or `None` before then.
pub async fn entry_runs(
    pool: &SqlitePool,
    id: &str,
) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT r.id, r.declaration,
                  NOT EXISTS (
                      SELECT 1 FROM entry_runs other JOIN entries x ON x.seq = other.entry_seq
                      WHERE other.account_id = r.account_id AND other.run_id = r.id
                        AND x.status != 'public'
                  ) AS "all_public!: bool"
           FROM entry_runs er JOIN runs r ON r.account_id = er.account_id AND r.id = er.run_id
           WHERE er.entry_seq = (SELECT seq FROM entries WHERE id = ?) ORDER BY r.id"#,
        id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let description = row.all_public.then(|| {
                serde_json::from_str::<serde_json::Value>(&row.declaration)
                    .ok()
                    .and_then(|run| run["description"].as_str().map(str::to_string))
                    .unwrap_or_default()
            });
            (row.id, description)
        })
        .collect())
}

/// Stores a batch and returns its row number and ID.
pub async fn insert_batch(
    transaction: &mut sqlx::SqliteConnection,
    account_id: &str,
) -> Result<(i64, String), sqlx::Error> {
    let id = ids::new_id();
    let created_at = now();
    let seq = sqlx::query_scalar!(
        r#"INSERT INTO batches (id, account_id, created_at) VALUES (?, ?, ?)
           RETURNING seq AS "seq!: i64""#,
        id,
        account_id,
        created_at
    )
    .fetch_one(&mut *transaction)
    .await?;
    Ok((seq, id))
}

/// Stores a run a batch declares, as the batch sent it.
pub async fn insert_run(
    transaction: &mut sqlx::SqliteConnection,
    account_id: &str,
    run_id: &str,
    declaration: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO runs (account_id, id, declaration) VALUES (?, ?, ?)",
        account_id,
        run_id,
        declaration
    )
    .execute(&mut *transaction)
    .await?;
    Ok(())
}

/// A batch's pending members, if `account_id` made it.
pub async fn batch_members(
    pool: &SqlitePool,
    batch_id: &str,
    account_id: &str,
) -> Result<Option<Vec<i64>>, sqlx::Error> {
    let batch = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM batches WHERE id = ? AND account_id = ?"#,
        batch_id,
        account_id
    )
    .fetch_optional(pool)
    .await?;
    let Some(batch_seq) = batch else {
        return Ok(None);
    };
    let members = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries WHERE batch_seq = ? AND status = 'pending'"#,
        batch_seq
    )
    .fetch_all(pool)
    .await?;
    Ok(Some(members))
}

/// Counts one unusable screening reply against an entry still being screened.
pub async fn count_screen_attempt(pool: &SqlitePool, seq: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE entries SET screen_attempts = screen_attempts + 1
         WHERE seq = ? AND status = 'screening'",
        seq
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Entry `seq`'s status.
pub async fn status(pool: &SqlitePool, seq: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!("SELECT status FROM entries WHERE seq = ?", seq)
        .fetch_optional(pool)
        .await
}

/// Each member of batch `batch_id` and its status, in order.
pub async fn batch_statuses(
    pool: &SqlitePool,
    batch_id: &str,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT e.id, e.status FROM entries e JOIN batches b ON b.seq = e.batch_seq
         WHERE b.id = ? ORDER BY e.seq",
        batch_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.status)).collect())
}

/// The slug a chain of versions has: the slug given to its first entry, if any.
pub async fn chain_slug(pool: &SqlitePool, seq: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"WITH RECURSIVE chain(seq, previous) AS (
               SELECT seq, previous_seq FROM entries WHERE seq = ?
               UNION ALL
               SELECT e.seq, e.previous_seq FROM entries e JOIN chain c ON e.seq = c.previous
           )
           SELECT s.slug AS "slug!" FROM slugs s JOIN chain c ON s.entry_seq = c.seq LIMIT 1"#,
        seq
    )
    .fetch_optional(pool)
    .await
}

/// The account's entries, not yet withdrawn, whose text refers to `slug:<slug>`, with
/// their embargo dates.
pub async fn referrers(
    pool: &SqlitePool,
    account_id: &str,
    slug: &str,
) -> Result<Vec<(i64, String, Option<String>)>, sqlx::Error> {
    let pattern = format!("%slug:{slug}%");
    let rows = sqlx::query!(
        r#"SELECT seq AS "seq!: i64", id, embargo_until, source FROM entries
           WHERE account_id = ? AND source LIKE ?
             AND status IN ('pending', 'screening', 'embargoed', 'public')"#,
        account_id,
        pattern
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| analyzer::slug_references(&row.source).contains(slug))
        .map(|row| (row.seq, row.id, row.embargo_until))
        .collect())
}

/// The ID and declaration (a JSON object, as its batch sent it) of each run entry
/// `seq` uses, in ID order.
pub async fn run_declarations(
    pool: &SqlitePool,
    seq: i64,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT r.id, r.declaration FROM entry_runs er
         JOIN runs r ON r.account_id = er.account_id AND r.id = er.run_id
         WHERE er.entry_seq = ? ORDER BY r.id",
        seq
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.declaration))
        .collect())
}

/// The IDs of the entries that use a run, in order.
pub async fn run_entries(
    pool: &SqlitePool,
    account_id: &str,
    run_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT e.id FROM entry_runs er JOIN entries e ON e.seq = er.entry_seq
         WHERE er.account_id = ? AND er.run_id = ? ORDER BY e.seq",
        account_id,
        run_id
    )
    .fetch_all(pool)
    .await
}

/// Stores the verdict screening reached on an entry still in `screening` that has none.
pub async fn store_verdict(
    pool: &SqlitePool,
    seq: i64,
    verdict: &Verdict,
) -> Result<(), sqlx::Error> {
    let (name, note, reasons, doubt, review_reason) = match verdict {
        Verdict::Pass { note, hazard_doubt } => {
            ("pass", note.clone(), None, hazard_doubt.clone(), None)
        }
        Verdict::Reject { reasons } => (
            "reject",
            None,
            Some(serde_json::json!(reasons).to_string()),
            None,
            None,
        ),
        Verdict::Review { why } => {
            // `why` names a hazard's kind, never its passage, so it may go in the log
            // and on the owner's page. It may quote an attached file's path or an
            // archive member's name, cut short; the page escapes it.
            tracing::info!(seq, why, "an entry goes to review");
            ("review", None, None, None, Some(why.clone()))
        }
    };
    sqlx::query!(
        "UPDATE entries SET screen_verdict = ?, screening_note = ?, rejection_reasons = ?,
             hazard_doubt = ?, review_reason = ?
         WHERE seq = ? AND status = 'screening' AND screen_verdict IS NULL",
        name,
        note,
        reasons,
        doubt,
        review_reason,
        seq
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Rejects an entry in screening, review, held or frozen, with `reasons` (a JSON
/// array) when it has none of its own, and gives back the slug it took, so its
/// author can submit again with it. An entry rejected before it was screened, as its
/// batch's other members are, frozen ones included, also gives back its allowance
/// (design.md, Agent interface).
pub async fn reject(pool: &SqlitePool, seq: i64, reasons: Option<&str>) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let unscreened = sqlx::query_scalar!(
        r#"SELECT 1 AS "one!: i64" FROM entries
           WHERE seq = ? AND (status = 'frozen'
               OR (status = 'screening' AND screen_verdict IS NULL))"#,
        seq
    )
    .fetch_optional(&mut *transaction)
    .await?;
    if unscreened.is_some() {
        allowance::give_back(&mut transaction, seq).await?;
    }
    sqlx::query!(
        "UPDATE entries SET status = 'rejected', screen_verdict = 'reject',
             rejection_reasons = COALESCE(?, rejection_reasons)
         WHERE seq = ? AND status IN ('screening', 'review', 'held', 'frozen')",
        reasons,
        seq
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!("DELETE FROM slugs WHERE entry_seq = ?", seq)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await
}

/// Moves an entry whose verdict is review out of screening, to the owner's queue.
pub async fn to_review(pool: &SqlitePool, seq: i64) -> Result<(), sqlx::Error> {
    // It joins the review queue one place behind its account's last entry there, as
    // `start_screening` places it in the screening queue.
    sqlx::query!(
        "UPDATE entries SET status = 'review',
             queue_position = 1 + COALESCE((
                 SELECT MAX(w.queue_position) FROM entries w
                 WHERE w.account_id = entries.account_id
                   AND w.status IN ('review', 'held')), 0)
         WHERE seq = ? AND status = 'screening' AND screen_verdict = 'review'",
        seq
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Puts the account that wrote entry `seq` under the review guard: the owner held or
/// rejected one of its entries (the guard trips on that, not on reaching review).
async fn guard_author(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
) -> Result<(), sqlx::Error> {
    let account = sqlx::query_scalar!("SELECT account_id FROM entries WHERE seq = ?", seq)
        .fetch_one(&mut *transaction)
        .await?;
    crate::accounts::guard(transaction, &account).await
}

/// The owner releases a frozen entry back to screening, as if its hold had just
/// ended; it keeps its allowance day. False if it wasn't frozen.
pub async fn release_frozen(pool: &SqlitePool, seq: i64) -> Result<bool, sqlx::Error> {
    Ok(to_screening(pool, seq, "frozen", None).await?.is_some())
}

/// The owner fails a frozen entry with `reasons` (a JSON array), as a review
/// rejection does, but without guarding its account: it was frozen for its invite
/// code, not its text (the owner's decision, 2026-11-06). It was never screened, so
/// it gives back its allowance. False if it wasn't frozen.
pub async fn reject_frozen(
    pool: &SqlitePool,
    seq: i64,
    reasons: &str,
) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let done = sqlx::query!(
        "UPDATE entries SET status = 'screening', screen_verdict = 'reject',
             rejection_reasons = ?
         WHERE seq = ? AND status = 'frozen'",
        reasons,
        seq
    )
    .execute(&mut *transaction)
    .await?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    allowance::give_back(&mut transaction, seq).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Holds an entry in review until a vendor fixes the hole, and guards its account.
/// False if it wasn't in review.
pub async fn hold(pool: &SqlitePool, seq: i64) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let done = sqlx::query!(
        "UPDATE entries SET status = 'held' WHERE seq = ? AND status = 'review'",
        seq
    )
    .execute(&mut *transaction)
    .await?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    guard_author(&mut transaction, seq).await?;
    transaction.commit().await?;
    Ok(true)
}

/// The owner passes an entry in review or held: it goes back to `screening` with a
/// pass, for the queue to place with its batch. False if it was in neither.
pub async fn owner_pass(pool: &SqlitePool, seq: i64) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE entries SET status = 'screening', screen_verdict = 'pass'
         WHERE seq = ? AND status IN ('review', 'held')",
        seq
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// The owner fails an entry in review or held with `reasons` (a JSON array): it goes
/// back to `screening` with a rejection, for the queue to send its whole batch back,
/// and its account is guarded. False if it was in neither.
pub async fn owner_reject(pool: &SqlitePool, seq: i64, reasons: &str) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let done = sqlx::query!(
        "UPDATE entries SET status = 'screening', screen_verdict = 'reject',
             rejection_reasons = ?
         WHERE seq = ? AND status IN ('review', 'held')",
        reasons,
        seq
    )
    .execute(&mut *transaction)
    .await?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    guard_author(&mut transaction, seq).await?;
    transaction.commit().await?;
    Ok(true)
}

/// An entry that passed although the screen doubted it holds a hazard.
pub struct Doubted {
    pub id: String,
    pub status: String,
    pub source: String,
    pub doubt: String,
}

/// Every entry that passed with a hazard doubt the owner hasn't marked seen, oldest
/// first.
pub async fn doubted(pool: &SqlitePool) -> Result<Vec<Doubted>, sqlx::Error> {
    sqlx::query_as!(
        Doubted,
        r#"SELECT id, status, source, hazard_doubt AS "doubt!"
           FROM entries
           WHERE hazard_doubt IS NOT NULL AND hazard_doubt_seen_at IS NULL
             AND status NOT IN ('withdrawn', 'removed')
           ORDER BY seq"#
    )
    .fetch_all(pool)
    .await
}

/// Takes entry `id` off the owner's list of doubted entries. False if it wasn't on it.
pub async fn mark_doubt_seen(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let at = now();
    let done = sqlx::query!(
        "UPDATE entries SET hazard_doubt_seen_at = ?
         WHERE id = ? AND hazard_doubt IS NOT NULL AND hazard_doubt_seen_at IS NULL",
        at,
        id
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// An entry waiting on the owner, in `review`, `held` or `frozen`.
pub struct Undecided {
    pub seq: i64,
    pub id: String,
    pub status: String,
    pub source: String,
    pub account_id: String,
    pub display_name: Option<String>,
    pub submitted_at: String,
    pub batch_id: Option<String>,
    /// Why the screen sent it to review; `None` for entries from before reasons
    /// were kept.
    pub review_reason: Option<String>,
}

/// Every entry in `review`, `held` or `frozen`, oldest submission first, or only
/// entry `id`.
pub async fn undecided(pool: &SqlitePool, id: Option<&str>) -> Result<Vec<Undecided>, sqlx::Error> {
    sqlx::query_as!(
        Undecided,
        r#"SELECT e.seq AS "seq!: i64", e.id, e.status, e.source, e.account_id,
                  a.display_name, e.submitted_at, b.id AS "batch_id?", e.review_reason
           FROM entries e JOIN authors a ON a.id = e.account_id
           LEFT JOIN batches b ON b.seq = e.batch_seq
           WHERE e.status IN ('review', 'held', 'frozen') AND (?1 IS NULL OR e.id = ?1)
           ORDER BY e.queue_position, e.submitted_at, e.seq"#,
        id
    )
    .fetch_all(pool)
    .await
}

/// Whether another member of entry `seq`'s batch has been rejected, so publishing
/// `seq` would send it back with its batch.
pub async fn batch_failed(pool: &SqlitePool, seq: i64) -> Result<bool, sqlx::Error> {
    let failed = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM entries e JOIN entries m ON m.batch_seq = e.batch_seq
               WHERE e.seq = ? AND m.seq != e.seq AND m.screen_verdict = 'reject'
           ) AS "failed!: bool""#,
        seq
    )
    .fetch_one(pool)
    .await?;
    Ok(failed)
}

/// Whether entry `seq` cites an attachment set.
pub async fn has_attachments(pool: &SqlitePool, seq: i64) -> Result<bool, sqlx::Error> {
    let set = sqlx::query_scalar!("SELECT attachment_set_seq FROM entries WHERE seq = ?", seq)
        .fetch_optional(pool)
        .await?;
    Ok(set.flatten().is_some())
}

/// The seq of entry `id`, whatever its status.
pub async fn seq_of(pool: &SqlitePool, id: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT seq AS "seq!: i64" FROM entries WHERE id = ?"#, id)
        .fetch_optional(pool)
        .await
}
