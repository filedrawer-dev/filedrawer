//! Storing entries and reading them back (design.md, Entry status; Agent interface).

use analyzer::{
    Analysis, Author, CardContext, Choice, Claim, Kind, Outcome, REFERENCE_KEYS_VERSION, Reference,
    Results, Role, Stage, Vocabulary, analyze, render_card,
};
use sqlx::SqlitePool;

use crate::embargo::ChainDate;
use crate::link_checks::{self, LinkFlags};
use crate::provenance::Provenance;
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
    /// The link checks' flags (`link_checks::LinkFlags`).
    pub code_link_dead: bool,
    pub logs_link_dead: bool,
    /// The `link_flags` view's `pin_flag` (`link_checks::pin_flag`).
    pub pin_flag: Option<String>,
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
        self.card_from(&analyze(&self.source), self.superseded_by.clone())
    }

    /// The card and the outcome its stamp shows, from one reading of the entry.
    pub fn card_and_outcome(&self) -> (String, Choice<Outcome>) {
        let analysis = analyze(&self.source);
        let card = self.card_from(&analysis, self.superseded_by.clone());
        (card, analysis.fields.outcome)
    }

    /// The card from `analysis`, this entry's, with `superseded_by` as the server's
    /// record of what replaced it.
    pub fn card_from(&self, analysis: &Analysis, superseded_by: Option<String>) -> String {
        let flags = LinkFlags {
            code_link_dead: self.code_link_dead,
            logs_link_dead: self.logs_link_dead,
            pin: link_checks::pin_flag(self.pin_flag.as_deref()),
        };
        let context = flags.on(CardContext {
            superseded_by_version: superseded_by,
            screening_note: self.screening_note.clone(),
            code_attached: self.code_attached,
            ..card_context(&self.id, &self.account_id, self.display_name.as_deref())
        });
        render_card(analysis, &context)
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
    /// The token that submitted it, the only one that may delete it while pending.
    pub token_id: &'a str,
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
    /// The skill and User-Agent of the request that sent it.
    pub provenance: &'a Provenance,
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
        token_id,
        source,
        analysis,
        hold_until,
        ref previous,
        batch_seq,
        register_slug,
        series,
        attachments,
        provenance,
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
               batch_seq, series_seq, series_version, series_section, attachment_set_seq,
               token_id, skill, user_agent)
           VALUES (?, ?, 'pending', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        attachments,
        token_id,
        provenance.skill,
        provenance.user_agent
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

/// What a PUT sends in place of a pending entry: the accepted text and what was read
/// from it, the end of its new hold, and the token that sent it.
pub struct Replacement<'a> {
    /// The token that sent it, which becomes the one that may delete the entry.
    pub token_id: &'a str,
    pub source: &'a str,
    pub analysis: &'a Analysis,
    pub hold_until: &'a str,
    /// The series version it cites, checked beforehand.
    pub series: Option<&'a Cited>,
    /// The attachment set it cites, checked beforehand to be its account's.
    pub attachments: Option<i64>,
    /// The skill and User-Agent of the request that sent the new text.
    pub provenance: &'a Provenance,
}

/// Replaces pending entry `seq`'s text and everything read from it with
/// `replacement`, and restarts its hold, in the caller's transaction. Its ID and
/// first submission date stay. Returns false if it is no longer pending.
pub async fn replace_on(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
    replacement: &Replacement<'_>,
) -> Result<bool, sqlx::Error> {
    let Replacement {
        token_id,
        source,
        analysis,
        hold_until,
        series,
        attachments,
        provenance,
    } = *replacement;
    let submitted_at = now();
    let columns = columns(analysis);
    let (series_seq, series_version, series_section) = series_columns(series);
    let done = sqlx::query!(
        "UPDATE entries SET source = ?, submitted_at = ?, embargo_until = ?,
             analyzer_version = ?, outcome = ?, kind = ?, stage = ?, hold_until = ?,
             series_seq = ?, series_version = ?, series_section = ?, series_removed_at = NULL,
             attachment_set_seq = ?, token_id = ?, skill = ?, user_agent = ?
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
        token_id,
        provenance.skill,
        provenance.user_agent,
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
    sqlx::query!("DELETE FROM entry_references WHERE entry_seq = ?", seq)
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
    write_references(&mut *transaction, seq, analysis, previous_seq).await
}

/// Stores the keys of what an entry's `replicates` and `reanalyzes` name, for search's
/// `cites` filter. A version that points to a paper and names nothing itself carries
/// the version before it's, as its rows are carried (see `write_derived`).
async fn write_references(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
    analysis: &Analysis,
    previous_seq: Option<i64>,
) -> Result<(), sqlx::Error> {
    let fields = &analysis.fields;
    let relations = fields.follow_ups();
    let names_nothing = relations.iter().all(|(_, items)| items.is_empty());
    if let (true, Some(previous), Some(_)) = (names_nothing, previous_seq, &fields.published_in) {
        sqlx::query!(
            "INSERT INTO entry_references (entry_seq, relation, position, key)
             SELECT ?, relation, position, key FROM entry_references WHERE entry_seq = ?",
            seq,
            previous
        )
        .execute(&mut *transaction)
        .await?;
        return Ok(());
    }
    for (relation, items) in relations {
        for (position, item) in (1_i64..).zip(items) {
            let Some(key) = Reference::parse(item).map(|reference| reference.key()) else {
                continue;
            };
            sqlx::query!(
                "INSERT INTO entry_references (entry_seq, relation, position, key)
                 VALUES (?, ?, ?, ?)",
                seq,
                relation,
                position,
                key
            )
            .execute(&mut *transaction)
            .await?;
        }
    }
    Ok(())
}

/// Rebuilds `entry_references` from every entry's text when the keys it holds are of
/// another format than the analyzer's, or the table was never filled: entries stored
/// before it existed get theirs. In seq order, so a version that carries its
/// predecessor's keys finds them.
pub async fn rebuild_stale_references(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let current = i64::from(REFERENCE_KEYS_VERSION);
    let mut transaction = crate::db::write_tx(pool).await?;
    let stored =
        sqlx::query_scalar!("SELECT version FROM derived_versions WHERE name = 'entry_references'")
            .fetch_optional(&mut *transaction)
            .await?;
    if stored == Some(current) {
        return Ok(());
    }
    sqlx::query!("DELETE FROM entry_references")
        .execute(&mut *transaction)
        .await?;
    let entries = sqlx::query!(
        r#"SELECT seq AS "seq!: i64", source, previous_seq FROM entries ORDER BY seq"#
    )
    .fetch_all(&mut *transaction)
    .await?;
    for entry in entries {
        let analysis = analyze(&entry.source);
        write_references(&mut transaction, entry.seq, &analysis, entry.previous_seq).await?;
    }
    sqlx::query!(
        "INSERT INTO derived_versions (name, version) VALUES ('entry_references', ?)
         ON CONFLICT (name) DO UPDATE SET version = excluded.version",
        current
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
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
    let mut transaction = crate::db::write_tx(pool).await?;
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
    index(&mut transaction, seq, fields, &source).await?;
    crate::series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(Some(Placed::Public))
}

/// Adds public entry `seq` to the search index.
async fn index(
    connection: &mut sqlx::SqliteConnection,
    seq: i64,
    fields: &analyzer::Fields,
    source: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO entries_fts (rowid, title, hypothesis, safety_motivation, body)
         VALUES (?, ?, ?, ?, ?)",
        seq,
        fields.title,
        fields.hypothesis,
        fields.safety_motivation,
        source
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// An entry held back that a release may move (design.md, Embargo; Series).
pub struct HeldBack {
    pub seq: i64,
    /// `embargoed` or `waiting_on_series`.
    pub status: String,
    /// The 12-month cap counts from here.
    pub first_submitted_at: String,
    /// The date its chain releases on ([`chain_date`]).
    pub chain_date: ChainDate,
    /// A later version of its chain still waits for a verdict ([`chain_undecided`]).
    pub chain_undecided: bool,
}

/// Why a held-back entry is due for release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Its chain's date has come.
    Embargo,
    /// The 12-month cap from its first submission has come, which releases it even
    /// while a hidden series holds it back (design.md, Series).
    Cap,
}

impl HeldBack {
    /// Why it is due at `now`, if it is. Never while a later version of its chain
    /// waits for a verdict, even at the cap: the versions release together, and an
    /// owner's hold beats the author's cap (the owner's decision, 2026-11-08).
    pub fn due(&self, now: time::OffsetDateTime) -> Option<Due> {
        if self.chain_undecided {
            return None;
        }
        let at_cap = crate::time::parse(&self.first_submitted_at)
            .is_some_and(|first| now >= crate::embargo::cap_time(first));
        let embargo_over = self.status == "embargoed" && !self.chain_date.holds(now);
        if at_cap {
            Some(Due::Cap)
        } else {
            embargo_over.then_some(Due::Embargo)
        }
    }

    /// Whether it is embargoed under a stored date that isn't one, which holds it
    /// until the 12-month cap.
    pub fn date_unreadable(&self) -> bool {
        self.status == "embargoed" && self.chain_date == ChainDate::Unreadable
    }
}

/// Every entry in `embargoed` or `waiting_on_series`, oldest first.
pub async fn held_back(pool: &SqlitePool) -> Result<Vec<HeldBack>, sqlx::Error> {
    let seqs = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries
           WHERE status IN ('embargoed', 'waiting_on_series') ORDER BY seq"#
    )
    .fetch_all(pool)
    .await?;
    let mut connection = pool.acquire().await?;
    let mut held = Vec::with_capacity(seqs.len());
    for seq in seqs {
        held.extend(held_back_one(&mut connection, seq).await?);
    }
    Ok(held)
}

/// Entry `seq`, if it is `embargoed` or `waiting_on_series`.
async fn held_back_one(
    connection: &mut sqlx::SqliteConnection,
    seq: i64,
) -> Result<Option<HeldBack>, sqlx::Error> {
    let Some(row) = sqlx::query!(
        "SELECT status, first_submitted_at FROM entries
         WHERE seq = ? AND status IN ('embargoed', 'waiting_on_series')",
        seq
    )
    .fetch_optional(&mut *connection)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(HeldBack {
        seq,
        status: row.status,
        first_submitted_at: row.first_submitted_at,
        chain_date: chain_date(&mut *connection, seq).await?,
        chain_undecided: chain_undecided(&mut *connection, seq).await?,
    }))
}

/// Whether a later version of entry `seq`'s chain is still pending, being screened,
/// or with the owner (`review`, `held`, `frozen`).
async fn chain_undecided(
    connection: &mut sqlx::SqliteConnection,
    seq: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"WITH RECURSIVE later(seq) AS (
               SELECT seq FROM entries WHERE previous_seq = ?1
               UNION ALL
               SELECT n.seq FROM entries n JOIN later l ON n.previous_seq = l.seq
           )
           SELECT EXISTS (
               SELECT 1 FROM entries e JOIN later l ON l.seq = e.seq
               WHERE e.status IN ('pending', 'screening', 'review', 'held', 'frozen')
           ) AS "undecided!: bool""#,
        seq
    )
    .fetch_one(&mut *connection)
    .await
}

/// An earlier version of a chain, held back while a later one is decided.
pub struct EarlierVersion {
    pub seq: i64,
    pub id: String,
    pub status: String,
}

/// The earlier versions of entry `seq`'s chain that are `embargoed` or
/// `waiting_on_series`, oldest first: the ones that wait on its verdict, which the
/// owner's "reject the chain" rejects with it.
pub async fn held_back_before<'e>(
    executor: impl sqlx::SqliteExecutor<'e>,
    seq: i64,
) -> Result<Vec<EarlierVersion>, sqlx::Error> {
    sqlx::query_as!(
        EarlierVersion,
        r#"WITH RECURSIVE earlier(seq) AS (
               SELECT previous_seq FROM entries WHERE seq = ?1 AND previous_seq IS NOT NULL
               UNION ALL
               SELECT e.previous_seq FROM entries e JOIN earlier p ON e.seq = p.seq
               WHERE e.previous_seq IS NOT NULL
           )
           SELECT e.seq AS "seq!: i64", e.id, e.status FROM entries e JOIN earlier p ON p.seq = e.seq
           WHERE e.status IN ('embargoed', 'waiting_on_series')
           ORDER BY e.seq"#,
        seq
    )
    .fetch_all(executor)
    .await
}

/// Entry `seq`'s chain's release date ([`chain_embargo`]), read.
pub async fn chain_date<'e>(
    executor: impl sqlx::SqliteExecutor<'e>,
    seq: i64,
) -> Result<ChainDate, sqlx::Error> {
    Ok(ChainDate::read(
        chain_embargo(executor, seq).await?.as_deref(),
    ))
}

/// The date entry `seq`'s chain releases on (design.md, Entry lifecycle: the
/// versions "release together"): the `embargo_until` of the latest version reached
/// from it through versions that aren't rejected, withdrawn or removed.
pub async fn chain_embargo<'e>(
    executor: impl sqlx::SqliteExecutor<'e>,
    seq: i64,
) -> Result<Option<String>, sqlx::Error> {
    let date = sqlx::query_scalar!(
        r#"WITH RECURSIVE chain(seq) AS (
               SELECT ?1
               UNION ALL
               SELECT n.seq FROM entries n JOIN chain c ON n.previous_seq = c.seq
               WHERE n.status NOT IN ('rejected', 'withdrawn', 'removed')
           )
           SELECT embargo_until AS "embargo_until?: String" FROM entries
           WHERE seq = (SELECT MAX(seq) FROM chain)"#,
        seq
    )
    .fetch_optional(executor)
    .await?;
    Ok(date.flatten())
}

/// Where a release moves an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Released {
    Public,
    /// Its embargo is over, but a series it waits for is hidden.
    WaitingOnSeries,
}

/// Moves each entry in `releases` that is still due at `now` from `embargoed` or
/// `waiting_on_series` to where it says, all in one transaction, so no read sees one
/// version of a chain public while another is held back. Every stored link of each
/// is due for a recheck at `now` (design.md, Embargo: "At release the server checks
/// the `code` and `logs` links again"). Returns how many it moved.
pub async fn release(
    pool: &SqlitePool,
    releases: &[(i64, Released)],
    now: time::OffsetDateTime,
) -> Result<usize, sqlx::Error> {
    // Nothing is due most minutes: don't take the write lock for nothing.
    if releases.is_empty() {
        return Ok(0);
    }
    let at = crate::time::stamp(now);
    let mut transaction = crate::db::write_tx(pool).await?;
    let mut moved = 0;
    for (seq, to) in releases {
        // Read again under the write lock: a date moved since the caller read it wins.
        let still_due = held_back_one(&mut transaction, *seq)
            .await?
            .and_then(|held| held.due(now))
            .is_some();
        if !still_due {
            continue;
        }
        let moved_now = match to {
            Released::Public => {
                let source = sqlx::query_scalar!(
                    "UPDATE entries SET status = 'public', published_at = ?
                     WHERE seq = ? AND status IN ('embargoed', 'waiting_on_series')
                     RETURNING source",
                    at,
                    seq
                )
                .fetch_optional(&mut *transaction)
                .await?;
                if let Some(source) = &source {
                    let fields = analyze(source).fields;
                    index(&mut transaction, *seq, &fields, source).await?;
                }
                source.is_some()
            }
            Released::WaitingOnSeries => {
                sqlx::query!(
                    "UPDATE entries SET status = 'waiting_on_series'
                     WHERE seq = ? AND status = 'embargoed'",
                    seq
                )
                .execute(&mut *transaction)
                .await?
                .rows_affected()
                    == 1
            }
        };
        if moved_now {
            link_checks::make_due(&mut transaction, *seq, now).await?;
            moved += 1;
        }
    }
    crate::series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(moved)
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
                  e.attachment_set_seq IS NOT NULL AS "code_attached!: bool",
                  IFNULL(f.code_link_dead, 0) AS "code_link_dead!: bool",
                  IFNULL(f.logs_link_dead, 0) AS "logs_link_dead!: bool",
                  f.pin_flag AS "pin_flag?: String"
         FROM entries e JOIN authors a ON a.id = e.account_id
         LEFT JOIN link_flags f ON f.entry_seq = e.seq
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
    /// The key of a work entries name in `replicates` or `reanalyzes`, never a slug's.
    /// A public entry's ID stands for every public version of its chain, and for the
    /// chain's slug among its own account's entries.
    pub cites: Option<String>,
}

/// A page of search results, and how many public entries match in all.
pub struct Found {
    pub entries: Vec<PublicEntry>,
    /// Every match, across pages; 0 when the page is past the last.
    pub total: i64,
}

/// Public entries matching `filters`: best match first when there are search words
/// (bm25, with the title weighted most), otherwise newest first.
pub async fn search(
    pool: &SqlitePool,
    filters: &Filters,
    limit: i64,
    offset: i64,
) -> Result<Found, sqlx::Error> {
    let outcome = filters.outcome.map(Outcome::name);
    let kind = filters.kind.map(Kind::name);
    let stage = filters.stage.map(Stage::name);
    let row_claim = filters.row_claim.map(Claim::name);
    let rows = sqlx::query!(
        r#"WITH RECURSIVE
             -- `cites` names a public entry: the public versions of its chain, and
             -- its slug if a public version registered it, stand for it too. A
             -- version that isn't public never widens the match (design.md, Agent
             -- interface).
             named(seq) AS (SELECT seq FROM entries WHERE id = ?11 AND status = 'public'),
             back(seq, previous_seq) AS (
               SELECT n.seq, p.previous_seq FROM named n JOIN entries p ON p.seq = n.seq
               UNION
               SELECT p.seq, p.previous_seq FROM back b JOIN entries p ON p.seq = b.previous_seq),
             chain(seq) AS (
               SELECT seq FROM back WHERE previous_seq IS NULL
               UNION
               SELECT n.seq FROM chain c JOIN entries n ON n.previous_seq = c.seq),
             cited(key, account_id) AS (
               SELECT ?11, NULL WHERE ?11 IS NOT NULL
               UNION
               SELECT v.id, NULL FROM chain c JOIN entries v ON v.seq = c.seq
               WHERE v.status = 'public'
               UNION
               SELECT 'slug:' || s.slug, s.account_id FROM slugs s
               JOIN chain c ON c.seq = s.entry_seq
               JOIN entries r ON r.seq = s.entry_seq
               WHERE r.status = 'public')
           SELECT e.seq AS "seq!: i64", e.id, e.source, e.account_id,
                  a.display_name AS "display_name?: String", e.submitted_at, e.published_at,
                  (SELECT p.id FROM entries p WHERE p.seq = e.previous_seq AND p.status = 'public') AS "previous_id?",
                  (SELECT n.id FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public') AS "superseded_by?",
                  e.screening_note,
                  e.attachment_set_seq IS NOT NULL AS "code_attached!: bool",
                  IFNULL(f.code_link_dead, 0) AS "code_link_dead!: bool",
                  IFNULL(f.logs_link_dead, 0) AS "logs_link_dead!: bool",
                  f.pin_flag AS "pin_flag?: String",
                  -- Every match, before LIMIT: the sort visits them all anyway.
                  COUNT(*) OVER () AS "total!: i64"
           FROM entries e JOIN authors a ON a.id = e.account_id
           LEFT JOIN link_flags f ON f.entry_seq = e.seq
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
             -- A slug names an entry only among its own account's.
             AND (?11 IS NULL OR EXISTS (
                    SELECT 1 FROM entry_references r JOIN cited k ON k.key = r.key
                    WHERE r.entry_seq = e.seq
                      AND (k.account_id IS NULL OR k.account_id = e.account_id)))
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
        offset,
        filters.cites
    )
    .fetch_all(pool)
    .await?;
    let total = rows.first().map_or(0, |row| row.total);
    let entries = rows
        .into_iter()
        .map(|row| PublicEntry {
            seq: row.seq,
            id: row.id,
            source: row.source,
            account_id: row.account_id,
            display_name: row.display_name,
            submitted_at: row.submitted_at,
            published_at: row.published_at,
            previous_id: row.previous_id,
            superseded_by: row.superseded_by,
            screening_note: row.screening_note,
            code_attached: row.code_attached,
            code_link_dead: row.code_link_dead,
            logs_link_dead: row.logs_link_dead,
            pin_flag: row.pin_flag,
        })
        .collect();
    Ok(Found { entries, total })
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
                  e.attachment_set_seq IS NOT NULL AS "code_attached!: bool",
                  IFNULL(f.code_link_dead, 0) AS "code_link_dead!: bool",
                  IFNULL(f.logs_link_dead, 0) AS "logs_link_dead!: bool",
                  f.pin_flag AS "pin_flag?: String"
         FROM entries e JOIN authors a ON a.id = e.account_id
         LEFT JOIN link_flags f ON f.entry_seq = e.seq
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

/// Whether `id` names a public entry, or any entry of `account_id`'s.
pub async fn public_or_own(
    pool: &SqlitePool,
    account_id: &str,
    id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM entries
               WHERE id = ? AND (status = 'public' OR account_id = ?)) AS "found!: bool""#,
        id,
        account_id
    )
    .fetch_one(pool)
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
/// public ([`run_description`]), or `None` before then.
pub async fn entry_runs(
    pool: &SqlitePool,
    id: &str,
) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let runs = sqlx::query!(
        "SELECT account_id, run_id FROM entry_runs
         WHERE entry_seq = (SELECT seq FROM entries WHERE id = ?) ORDER BY run_id",
        id
    )
    .fetch_all(&mut *connection)
    .await?;
    let mut described = Vec::new();
    for run in runs {
        let description = run_description(&mut connection, &run.account_id, &run.run_id).await?;
        described.push((run.run_id, description));
    }
    Ok(described)
}

/// The description of `account_id`'s run `run_id` once every entry using it is
/// public (design.md, Agent interface), and `None` before then or with no such run.
/// A run's description is author text, shown only once no hidden entry uses it.
pub async fn run_description(
    connection: &mut sqlx::SqliteConnection,
    account_id: &str,
    run_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let declaration = sqlx::query_scalar!(
        "SELECT r.declaration FROM runs r WHERE r.account_id = ?1 AND r.id = ?2
           AND NOT EXISTS (
               SELECT 1 FROM entry_runs other JOIN entries x ON x.seq = other.entry_seq
               WHERE other.account_id = ?1 AND other.run_id = ?2 AND x.status != 'public')",
        account_id,
        run_id
    )
    .fetch_optional(&mut *connection)
    .await?;
    Ok(declaration.map(|declaration| {
        serde_json::from_str::<serde_json::Value>(&declaration)
            .ok()
            .and_then(|run| run["description"].as_str().map(str::to_string))
            .unwrap_or_default()
    }))
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
    let mut transaction = crate::db::write_tx(pool).await?;
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
    let mut transaction = crate::db::write_tx(pool).await?;
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
    let mut transaction = crate::db::write_tx(pool).await?;
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
pub async fn owner_reject(
    pool: &SqlitePool,
    seq: i64,
    reasons: &str,
    earlier: Earlier,
) -> Result<bool, sqlx::Error> {
    let mut transaction = crate::db::write_tx(pool).await?;
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
    if earlier == Earlier::Reject {
        for version in held_back_before(&mut *transaction, seq).await? {
            sqlx::query!(
                "UPDATE entries SET status = 'rejected', rejection_reasons = ?
                 WHERE seq = ? AND status IN ('embargoed', 'waiting_on_series')",
                reasons,
                version.seq
            )
            .execute(&mut *transaction)
            .await?;
            // As any rejection does, so its author can submit again with the slug.
            sqlx::query!("DELETE FROM slugs WHERE entry_seq = ?", version.seq)
                .execute(&mut *transaction)
                .await?;
        }
        // A rejected version no longer holds back the series version it cites.
        crate::series::mark_readable(&mut transaction).await?;
    }
    transaction.commit().await?;
    Ok(true)
}

/// What the owner's reject of a version does to the earlier versions of its chain
/// that are held back with it ([`held_back_before`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Earlier {
    /// They release once it is decided, on their date.
    Keep,
    /// They are rejected with it (the owner's "reject the chain", 2026-11-08).
    Reject,
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

/// An entry as the owner's pages show it: one waiting on the owner, in `review`,
/// `held` or `frozen`, or, on its own admin page, any entry.
pub struct OwnerEntry {
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

/// Every entry in `review`, `held` or `frozen`, oldest submission first.
pub async fn undecided(pool: &SqlitePool) -> Result<Vec<OwnerEntry>, sqlx::Error> {
    owner_entries(pool, None).await
}

/// Entry `id`, whatever its status, for its admin page.
pub async fn for_owner(pool: &SqlitePool, id: &str) -> Result<Option<OwnerEntry>, sqlx::Error> {
    Ok(owner_entries(pool, Some(id)).await?.pop())
}

/// Entry `id`, or with `None` every entry in `review`, `held` or `frozen`, in the
/// queue's order.
async fn owner_entries(
    pool: &SqlitePool,
    id: Option<&str>,
) -> Result<Vec<OwnerEntry>, sqlx::Error> {
    sqlx::query_as!(
        OwnerEntry,
        r#"SELECT e.seq AS "seq!: i64", e.id, e.status, e.source, e.account_id,
                  a.display_name, e.submitted_at, b.id AS "batch_id?", e.review_reason
           FROM entries e JOIN authors a ON a.id = e.account_id
           LEFT JOIN batches b ON b.seq = e.batch_seq
           WHERE (?1 IS NULL AND e.status IN ('review', 'held', 'frozen')) OR e.id = ?1
           ORDER BY e.queue_position, e.submitted_at, e.seq"#,
        id
    )
    .fetch_all(pool)
    .await
}

/// Another version of an entry's chain, as its admin page lists it.
pub struct ChainVersion {
    pub id: String,
    pub status: String,
}

/// Every other version of entry `seq`'s chain, oldest first: those it descends from,
/// and every version descending from the chain's first, forks included, of any
/// status.
pub async fn other_versions(pool: &SqlitePool, seq: i64) -> Result<Vec<ChainVersion>, sqlx::Error> {
    sqlx::query_as!(
        ChainVersion,
        r#"WITH RECURSIVE
             up(seq, previous_seq) AS (
               SELECT seq, previous_seq FROM entries WHERE seq = ?1
               UNION ALL
               SELECT e.seq, e.previous_seq FROM entries e JOIN up ON e.seq = up.previous_seq),
             down(seq) AS (
               SELECT seq FROM up WHERE previous_seq IS NULL
               UNION
               SELECT e.seq FROM entries e JOIN down ON e.previous_seq = down.seq)
           SELECT id, status FROM entries
           WHERE seq IN (SELECT seq FROM down) AND seq != ?1
           ORDER BY seq"#,
        seq
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

/// How many entries wait in `screening` without a verdict, and when the oldest of them
/// was submitted.
pub async fn waiting_to_screen(pool: &SqlitePool) -> Result<(i64, Option<String>), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!: i64", MIN(submitted_at) AS "oldest: String"
           FROM entries WHERE status = 'screening' AND screen_verdict IS NULL"#
    )
    .fetch_one(pool)
    .await?;
    Ok((row.count, row.oldest))
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

/// One of an account's entries, as `GET /entries/mine` lists it.
pub struct Mine {
    pub id: String,
    pub status: String,
    /// Its chain's slug, while the account holds it.
    pub slug: Option<String>,
    pub title: Option<String>,
    pub code: Option<String>,
}

/// Whether the `code` link `code` lies in the repo at `repo`: it is that URL, or goes
/// on from it with a path, query or fragment, ignoring case.
fn in_repo(code: &str, repo: &str) -> bool {
    let code = code.to_lowercase();
    let repo = repo.to_lowercase();
    code.strip_prefix(&repo)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// `account_id`'s entries, newest first, those the owner removed included, at
/// `offset` and at most `limit` of them; with `repo`, only those whose `code` link
/// lies in that repo. The link is read from each entry's text, so a repo filter
/// reads every entry whose text holds the repo's URL, and pages after.
pub async fn mine(
    pool: &SqlitePool,
    account_id: &str,
    repo: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Mine>, sqlx::Error> {
    // SQLite reads a negative LIMIT as none.
    let (rows_limit, rows_offset) = if repo.is_some() {
        (-1, 0)
    } else {
        (limit, offset)
    };
    let needle = repo.map(str::to_lowercase);
    let rows = sqlx::query!(
        r#"SELECT seq AS "seq!: i64", id, status, source FROM entries
           WHERE account_id = ?1
             AND (?2 IS NULL OR instr(lower(source), ?2) > 0)
           ORDER BY seq DESC LIMIT ?3 OFFSET ?4"#,
        account_id,
        needle,
        rows_limit,
        rows_offset
    )
    .fetch_all(pool)
    .await?;
    let mut listed = Vec::new();
    for row in rows {
        let fields = analyze(&row.source).fields;
        if let Some(repo) = repo
            && !fields
                .code
                .as_deref()
                .is_some_and(|code| in_repo(code, repo))
        {
            continue;
        }
        listed.push(Mine {
            slug: chain_slug(pool, row.seq).await?,
            id: row.id,
            status: row.status,
            title: fields.title,
            code: fields.code,
        });
    }
    if repo.is_some() {
        let skip = usize::try_from(offset).unwrap_or(usize::MAX);
        let take = usize::try_from(limit).unwrap_or(usize::MAX);
        listed = listed.into_iter().skip(skip).take(take).collect();
    }
    Ok(listed)
}
