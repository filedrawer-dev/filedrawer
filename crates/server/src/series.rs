//! Storing series and reading them back (design.md, Series): versions, members, the
//! version each entry cites, and when a version may be read.

use analyzer::{SeriesCite, SeriesText};
use sqlx::SqlitePool;

use crate::{ids, time::now};

/// A series as a reader may see it.
pub struct Series {
    pub seq: i64,
    pub id: String,
    /// The account that made it, which alone makes versions and makes it public.
    pub creator: String,
    pub public: bool,
    /// The newest version's number.
    pub latest: i64,
}

/// One version of a series.
pub struct Version {
    pub version: i64,
    pub title: String,
    pub description: String,
    /// Whether anyone may read it (see `mark_readable`).
    /// version is.
    pub readable: bool,
}

/// The series version an accepted entry cites, checked against the database.
#[derive(Debug, Clone)]
pub struct Cited {
    pub series_seq: i64,
    pub version: i64,
    pub section: Option<String>,
}

/// A new series: its ID and the join key, which is shown once.
pub async fn create(
    pool: &SqlitePool,
    account_id: &str,
    text: &SeriesText,
    public: bool,
) -> Result<(String, String), sqlx::Error> {
    let id = format!("s-{}", ids::new_id());
    let key = ids::new_join_key();
    let hash = ids::token_hash(&key);
    let created_at = now();
    let public_at = public.then_some(created_at.as_str());
    let mut transaction = crate::db::write_tx(pool).await?;
    let seq = sqlx::query_scalar!(
        r#"INSERT INTO series (id, account_id, key_hash, created_at, public_at)
           VALUES (?, ?, ?, ?, ?)
           RETURNING seq AS "seq!: i64""#,
        id,
        account_id,
        hash,
        created_at,
        public_at
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query!(
        "INSERT INTO series_members (series_seq, account_id, joined_at) VALUES (?, ?, ?)",
        seq,
        account_id,
        created_at
    )
    .execute(&mut *transaction)
    .await?;
    insert_version(&mut transaction, seq, 1, text).await?;
    transaction.commit().await?;
    Ok((id, key))
}

async fn insert_version(
    transaction: &mut sqlx::SqliteConnection,
    seq: i64,
    version: i64,
    text: &SeriesText,
) -> Result<(), sqlx::Error> {
    let created_at = now();
    sqlx::query!(
        "INSERT INTO series_versions (series_seq, version, title, description, created_at)
         VALUES (?, ?, ?, ?, ?)",
        seq,
        version,
        text.title,
        text.description,
        created_at
    )
    .execute(&mut *transaction)
    .await?;
    Ok(())
}

/// Adds the next version and returns its number.
pub async fn add_version(
    pool: &SqlitePool,
    seq: i64,
    text: &SeriesText,
) -> Result<i64, sqlx::Error> {
    let mut transaction = crate::db::write_tx(pool).await?;
    let version = sqlx::query_scalar!(
        r#"SELECT COALESCE(MAX(version), 0) + 1 AS "next!: i64" FROM series_versions
           WHERE series_seq = ?"#,
        seq
    )
    .fetch_one(&mut *transaction)
    .await?;
    insert_version(&mut transaction, seq, version, text).await?;
    transaction.commit().await?;
    Ok(version)
}

/// The series `id`, unless the owner removed it: every route and cite check finds a
/// removed series missing (`removed_series`).
pub async fn find(pool: &SqlitePool, id: &str) -> Result<Option<Series>, sqlx::Error> {
    sqlx::query_as!(
        Series,
        r#"SELECT s.seq AS "seq!: i64", s.id, s.account_id AS creator,
                  s.public_at IS NOT NULL AS "public!: bool",
                  (SELECT MAX(version) FROM series_versions v WHERE v.series_seq = s.seq) AS "latest!: i64"
           FROM series s WHERE s.id = ? AND s.removed_at IS NULL"#,
        id
    )
    .fetch_optional(pool)
    .await
}

/// Whether `account_id` is a member of series `seq`.
pub async fn is_member(pool: &SqlitePool, seq: i64, account_id: &str) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar!(
        "SELECT 1 AS found FROM series_members WHERE series_seq = ? AND account_id = ?",
        seq,
        account_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Makes `account_id` a member of series `id` if `key` is its join key. False when
/// the series doesn't exist or the key is wrong. Joining twice is harmless.
pub async fn join(
    pool: &SqlitePool,
    id: &str,
    key: &str,
    account_id: &str,
) -> Result<bool, sqlx::Error> {
    let hash = ids::token_hash(key);
    let Some(seq) = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM series
           WHERE id = ? AND key_hash = ? AND removed_at IS NULL"#,
        id,
        hash
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(false);
    };
    let joined_at = now();
    sqlx::query!(
        "INSERT INTO series_members (series_seq, account_id, joined_at) VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
        seq,
        account_id,
        joined_at
    )
    .execute(pool)
    .await?;
    Ok(true)
}

/// Version `version` of series `seq`.
pub async fn version(
    pool: &SqlitePool,
    seq: i64,
    version: i64,
) -> Result<Option<Version>, sqlx::Error> {
    sqlx::query_as!(
        Version,
        r#"SELECT version AS "version!: i64", title, description,
                  readable_at IS NOT NULL AS "readable!: bool"
           FROM series_versions WHERE series_seq = ? AND version = ?"#,
        seq,
        version
    )
    .fetch_optional(pool)
    .await
}

/// The newest version anyone may read, if one is.
pub async fn latest_readable(pool: &SqlitePool, seq: i64) -> Result<Option<Version>, sqlx::Error> {
    sqlx::query_as!(
        Version,
        r#"SELECT version AS "version!: i64", title, description, TRUE AS "readable!: bool"
           FROM series_versions WHERE series_seq = ? AND readable_at IS NOT NULL
           ORDER BY version DESC LIMIT 1"#,
        seq
    )
    .fetch_optional(pool)
    .await
}

/// Makes readable, for good, every version whose series is public, that a public
/// entry cites, and that no entry still on its way to public cites (design.md,
/// Series: a new version "stays unreadable ... until every entry citing it is
/// public"). An entry taken out of the series counts neither way, and a removed
/// series' versions stay as they are. Run whenever an entry becomes public, leaves a
/// series, or a series becomes public or is restored.
pub async fn mark_readable(connection: &mut sqlx::SqliteConnection) -> Result<(), sqlx::Error> {
    let readable_at = now();
    sqlx::query!(
        "UPDATE series_versions SET readable_at = ?
         WHERE readable_at IS NULL
           AND EXISTS (
               SELECT 1 FROM series s WHERE s.seq = series_versions.series_seq
                 AND s.public_at IS NOT NULL AND s.removed_at IS NULL)
           AND EXISTS (
               SELECT 1 FROM entries x
               WHERE x.series_seq = series_versions.series_seq
                 AND x.series_version = series_versions.version AND x.status = 'public'
                 AND x.series_removed_at IS NULL)
           AND NOT EXISTS (
               SELECT 1 FROM entries x
               WHERE x.series_seq = series_versions.series_seq
                 AND x.series_version = series_versions.version
                 AND x.series_removed_at IS NULL
                 AND x.status NOT IN ('public', 'rejected', 'withdrawn', 'removed'))",
        readable_at
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// A public entry listed on its series' page.
pub struct Listed {
    pub id: String,
    pub account_id: String,
    /// `None` once the account is deleted.
    pub display_name: Option<String>,
}

/// The public entries in a series, oldest first: the latest public version in each
/// chain, when it cites the series and hasn't been taken out of it.
pub async fn public_entries(pool: &SqlitePool, seq: i64) -> Result<Vec<Listed>, sqlx::Error> {
    sqlx::query_as!(
        Listed,
        r#"SELECT e.id, e.account_id,
                  a.display_name AS "display_name?: String"
           FROM entries e JOIN authors a ON a.id = e.account_id
           WHERE e.series_seq = ? AND e.status = 'public' AND e.series_removed_at IS NULL
             AND NOT EXISTS (SELECT 1 FROM entries n WHERE n.previous_seq = e.seq AND n.status = 'public')
           ORDER BY e.published_at, e.seq"#,
        seq
    )
    .fetch_all(pool)
    .await
}

/// Checks what an entry by `account_id` cites: the series exists and the account is a
/// member, the version exists, and the section is one of its headings. `Err` holds
/// the reason to refuse the entry.
pub async fn check_cite(
    pool: &SqlitePool,
    account_id: &str,
    cite: &SeriesCite,
) -> Result<Result<Cited, String>, sqlx::Error> {
    let series = match find(pool, &cite.id).await? {
        Some(series) if is_member(pool, series.seq, account_id).await? => series,
        // A series this account doesn't belong to looks the same as none.
        _ => {
            return Ok(Err(format!(
                "`series` names {}, which this account isn't a member of; join it first with a POST of its key to /series/{}/join",
                cite.id, cite.id
            )));
        }
    };
    let Some(found) = version(pool, series.seq, cite.version).await? else {
        return Ok(Err(format!(
            "series {} has no version {}; its latest is {}",
            cite.id, cite.version, series.latest
        )));
    };
    if let Some(section) = &cite.section
        && analyzer::description_section(&found.description, section).is_none()
    {
        return Ok(Err(format!(
            "version {} of series {} has no heading whose anchor is `{section}`",
            cite.version, cite.id
        )));
    }
    Ok(Ok(Cited {
        series_seq: series.seq,
        version: cite.version,
        section: cite.section.clone(),
    }))
}

/// The series version a stored entry cites.
pub struct EntryCite {
    pub series_id: String,
    pub series_seq: i64,
    pub version: i64,
    pub section: Option<String>,
    /// The owner removed the series, so its text shows on no entry.
    pub series_removed: bool,
}

impl EntryCite {
    /// The cite as the entry's `series` field writes it.
    pub fn text(&self) -> String {
        match &self.section {
            Some(section) => format!("{}@{}#{section}", self.series_id, self.version),
            None => format!("{}@{}", self.series_id, self.version),
        }
    }
}

/// What entry `seq` cites, if anything.
pub async fn entry_cite(pool: &SqlitePool, seq: i64) -> Result<Option<EntryCite>, sqlx::Error> {
    sqlx::query_as!(
        EntryCite,
        r#"SELECT s.id AS series_id, s.seq AS "series_seq!: i64",
                  e.series_version AS "version!: i64", e.series_section AS section,
                  s.removed_at IS NOT NULL AS "series_removed!: bool"
           FROM entries e JOIN series s ON s.seq = e.series_seq
           WHERE e.seq = ? AND e.series_version IS NOT NULL"#,
        seq
    )
    .fetch_optional(pool)
    .await
}

/// Whether entry `seq` must wait for a series: its own, or one that another live
/// member of its batch cites, is still hidden. A removed series holds nothing back.
pub async fn waits(pool: &SqlitePool, seq: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM entries m JOIN series s ON s.seq = m.series_seq
               WHERE s.public_at IS NULL AND s.removed_at IS NULL
                 AND m.series_removed_at IS NULL
                 AND m.status NOT IN ('rejected', 'withdrawn', 'removed')
                 AND (m.seq = ?1 OR m.batch_seq = (SELECT batch_seq FROM entries WHERE seq = ?1))
           ) AS "waits!: bool""#,
        seq
    )
    .fetch_one(pool)
    .await
}

/// Gives series `seq` a new join key and returns it; the old key stops working.
pub async fn rotate_key(pool: &SqlitePool, seq: i64) -> Result<String, sqlx::Error> {
    let key = ids::new_join_key();
    let hash = ids::token_hash(&key);
    sqlx::query!("UPDATE series SET key_hash = ? WHERE seq = ?", hash, seq)
        .execute(pool)
        .await?;
    Ok(key)
}

/// A member of a series, for its creator.
pub struct Member {
    pub account_id: String,
    /// `None` once the account is deleted.
    pub display_name: Option<String>,
}

/// The members of series `seq`, in the order they joined.
pub async fn members(pool: &SqlitePool, seq: i64) -> Result<Vec<Member>, sqlx::Error> {
    sqlx::query_as!(
        Member,
        r#"SELECT m.account_id,
                  a.display_name AS "display_name?: String"
           FROM series_members m JOIN authors a ON a.id = m.account_id
           WHERE m.series_seq = ? ORDER BY m.joined_at, m.account_id"#,
        seq
    )
    .fetch_all(pool)
    .await
}

/// Takes `account_id` out of series `seq`'s members. False if it wasn't one.
pub async fn remove_member(
    pool: &SqlitePool,
    seq: i64,
    account_id: &str,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM series_members WHERE series_seq = ? AND account_id = ?",
        seq,
        account_id
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Takes entry `entry_id` out of series `seq`. False if it doesn't cite the series.
/// Taking it out again changes nothing.
pub async fn remove_entry(
    pool: &SqlitePool,
    seq: i64,
    entry_id: &str,
) -> Result<bool, sqlx::Error> {
    let removed_at = now();
    let mut transaction = crate::db::write_tx(pool).await?;
    let done = sqlx::query!(
        "UPDATE entries SET series_removed_at = COALESCE(series_removed_at, ?)
         WHERE id = ? AND series_seq = ?",
        removed_at,
        entry_id,
        seq
    )
    .execute(&mut *transaction)
    .await?;
    mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(done.rows_affected() == 1)
}

/// A series on its creator's account page.
pub struct Owned {
    pub id: String,
    pub title: String,
    pub public: bool,
    /// The owner removed it.
    pub removed: bool,
}

/// The series `account_id` made, newest first, each with its latest title.
pub async fn created_by(pool: &SqlitePool, account_id: &str) -> Result<Vec<Owned>, sqlx::Error> {
    sqlx::query_as!(
        Owned,
        r#"SELECT s.id, v.title, s.public_at IS NOT NULL AS "public!: bool",
                  s.removed_at IS NOT NULL AS "removed!: bool"
           FROM series s JOIN series_versions v ON v.series_seq = s.seq
           WHERE s.account_id = ?
             AND v.version = (SELECT MAX(version) FROM series_versions WHERE series_seq = s.seq)
           ORDER BY s.seq DESC"#,
        account_id
    )
    .fetch_all(pool)
    .await
}

/// Makes series `id` public if `account_id` made it. False when it didn't. Making a
/// public series public again changes nothing.
pub async fn make_public(
    pool: &SqlitePool,
    id: &str,
    account_id: &str,
) -> Result<bool, sqlx::Error> {
    let public_at = now();
    let mut transaction = crate::db::write_tx(pool).await?;
    let found = sqlx::query_scalar!(
        r#"UPDATE series SET public_at = COALESCE(public_at, ?)
           WHERE id = ? AND account_id = ? AND removed_at IS NULL
           RETURNING seq AS "seq!: i64""#,
        public_at,
        id,
        account_id
    )
    .fetch_optional(&mut *transaction)
    .await?;
    mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(found.is_some())
}

/// A series the screen refused as a likely hazard, waiting for the owner.
pub struct Refused {
    pub id: String,
    pub account_id: String,
    pub display_name: Option<String>,
    pub series_id: Option<String>,
    pub title: String,
    pub description: String,
    pub why: String,
    pub refused_at: String,
}

/// Keeps a series the screen refused as a likely hazard for the owner (the owner's
/// decision, 2026-11-06). `series_id` is the series it would have versioned.
pub async fn record_refusal(
    pool: &SqlitePool,
    account_id: &str,
    series_id: Option<&str>,
    text: &SeriesText,
    why: &str,
) -> Result<(), sqlx::Error> {
    let id = ids::new_id();
    let at = now();
    sqlx::query!(
        "INSERT INTO refused_series (id, account_id, series_id, title, description, why, refused_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        id,
        account_id,
        series_id,
        text.title,
        text.description,
        why,
        at
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// The refused series the owner hasn't decided, oldest first.
pub async fn refusals(pool: &SqlitePool) -> Result<Vec<Refused>, sqlx::Error> {
    sqlx::query_as!(
        Refused,
        "SELECT r.id, r.account_id, a.display_name, r.series_id, r.title, r.description,
                r.why, r.refused_at
         FROM refused_series r JOIN authors a ON a.id = r.account_id
         WHERE r.decision IS NULL ORDER BY r.refused_at, r.id"
    )
    .fetch_all(pool)
    .await
}

/// The owner's call on a refused series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCall {
    /// A hazard: its account goes under the review guard.
    Confirm,
    /// A false flag.
    Dismiss,
}

impl RefusalCall {
    /// As `refused_series.decision` stores it.
    fn as_str(self) -> &'static str {
        match self {
            RefusalCall::Confirm => "confirmed",
            RefusalCall::Dismiss => "dismissed",
        }
    }
}

/// Records the owner's call on a refused series. False if there's no such
/// undecided refusal.
pub async fn decide_refusal(
    pool: &SqlitePool,
    id: &str,
    call: RefusalCall,
) -> Result<bool, sqlx::Error> {
    let decision = call.as_str();
    let at = now();
    let mut transaction = crate::db::write_tx(pool).await?;
    let account = sqlx::query_scalar!(
        "UPDATE refused_series SET decision = ?, decided_at = ?
         WHERE id = ? AND decision IS NULL RETURNING account_id",
        decision,
        at,
        id
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(account) = account else {
        return Ok(false);
    };
    // A confirmed hazard guards the account, as the owner's reject or hold of an
    // entry does (the owner's decision, 2026-11-06).
    if call == RefusalCall::Confirm {
        crate::accounts::guard(&mut transaction, &account).await?;
    }
    transaction.commit().await?;
    Ok(true)
}
