//! The owner removing a series and restoring it (design.md, Admin; #57): a removed
//! series answers 404 on every route, to its creator and members too, its title and
//! description show on no entry, and it stays in the database. Its members' entries
//! stay as they are.
//!
//! `series::find` leaves removed series out, so every route that looks a series up
//! through it, and every cite checked against it, treats a removed one as missing.

use sqlx::{SqliteConnection, SqlitePool};

use crate::{db, series, time::now};

/// A series as its admin page shows it.
pub struct OwnersSeries {
    pub id: String,
    /// The account that made it.
    pub creator: String,
    pub public: bool,
    /// When the owner removed it, while it is removed.
    pub removed_at: Option<String>,
}

/// One version, whatever anyone may read.
pub struct OwnersVersion {
    pub version: i64,
    pub title: String,
    pub description: String,
}

/// An entry that cites the series.
pub struct Citing {
    pub id: String,
    pub status: String,
}

/// Where a series stands for the owner's Remove and Restore.
pub enum SeriesStanding {
    Removable,
    Restorable,
    /// Removed, and a restore is refused while its creator is suspended.
    Refused,
}

/// What the admin page of series `id` shows; `None` if there is no such series.
pub struct OwnersView {
    pub series: OwnersSeries,
    pub versions: Vec<OwnersVersion>,
    pub entries: Vec<Citing>,
    pub standing: SeriesStanding,
}

struct Row {
    seq: i64,
    series: OwnersSeries,
    creator_suspended: bool,
}

async fn row(connection: &mut SqliteConnection, id: &str) -> Result<Option<Row>, sqlx::Error> {
    let found = sqlx::query!(
        r#"SELECT s.seq AS "seq!: i64", s.id, s.account_id, s.removed_at,
                  s.public_at IS NOT NULL AS "public!: bool",
                  a.suspended_at IS NOT NULL AS "suspended!: bool"
           FROM series s JOIN accounts a ON a.id = s.account_id
           WHERE s.id = ?"#,
        id
    )
    .fetch_optional(connection)
    .await?;
    Ok(found.map(|found| Row {
        seq: found.seq,
        series: OwnersSeries {
            id: found.id,
            creator: found.account_id,
            public: found.public,
            removed_at: found.removed_at,
        },
        creator_suspended: found.suspended,
    }))
}

fn standing(row: &Row) -> SeriesStanding {
    match (&row.series.removed_at, row.creator_suspended) {
        (None, _) => SeriesStanding::Removable,
        (Some(_), false) => SeriesStanding::Restorable,
        (Some(_), true) => SeriesStanding::Refused,
    }
}

/// Series `id` with every version and every entry citing it, oldest first.
pub async fn owners_view(pool: &SqlitePool, id: &str) -> Result<Option<OwnersView>, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let Some(row) = row(&mut connection, id).await? else {
        return Ok(None);
    };
    let versions = sqlx::query_as!(
        OwnersVersion,
        r#"SELECT version AS "version!: i64", title, description FROM series_versions
           WHERE series_seq = ? ORDER BY version"#,
        row.seq
    )
    .fetch_all(&mut *connection)
    .await?;
    let entries = sqlx::query_as!(
        Citing,
        "SELECT id, status FROM entries WHERE series_seq = ? ORDER BY seq",
        row.seq
    )
    .fetch_all(&mut *connection)
    .await?;
    let standing = standing(&row);
    Ok(Some(OwnersView {
        series: row.series,
        versions,
        entries,
        standing,
    }))
}

/// What removing or restoring did.
#[derive(Debug, PartialEq, Eq)]
pub enum Changed {
    Done,
    NoSeries,
    /// Already in the state asked for.
    Already,
    /// A restore while its creator is suspended.
    CreatorSuspended,
}

/// Removes series `id`. The caller places the entries that waited on it.
pub async fn remove(pool: &SqlitePool, id: &str) -> Result<Changed, sqlx::Error> {
    let mut transaction = db::write_tx(pool).await?;
    let Some(row) = row(&mut transaction, id).await? else {
        return Ok(Changed::NoSeries);
    };
    if row.series.removed_at.is_some() {
        return Ok(Changed::Already);
    }
    let at = now();
    sqlx::query!(
        "UPDATE series SET removed_at = ? WHERE seq = ?",
        at,
        row.seq
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Changed::Done)
}

/// Restores removed series `id`, unless its creator is suspended. Versions that
/// would have become readable while it was removed become readable now.
pub async fn restore(pool: &SqlitePool, id: &str) -> Result<Changed, sqlx::Error> {
    let mut transaction = db::write_tx(pool).await?;
    let Some(row) = row(&mut transaction, id).await? else {
        return Ok(Changed::NoSeries);
    };
    match standing(&row) {
        SeriesStanding::Removable => return Ok(Changed::Already),
        SeriesStanding::Refused => return Ok(Changed::CreatorSuspended),
        SeriesStanding::Restorable => {}
    }
    sqlx::query!("UPDATE series SET removed_at = NULL WHERE seq = ?", row.seq)
        .execute(&mut *transaction)
        .await?;
    series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(Changed::Done)
}
