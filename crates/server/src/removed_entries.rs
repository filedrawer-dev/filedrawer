//! The owner removing an entry and restoring it (design.md, Admin; #43): a removed
//! entry is served nowhere, with no public notice, and stays in the database for
//! analysis. A restore puts back the status it had.
//!
//! Every other change of status is guarded on the status it changes from, so once an
//! entry is `removed`, no job, screen verdict or review decision moves it.

use sqlx::{SqliteConnection, SqlitePool};

use crate::{db, series, time::now};

/// What removing an entry did.
#[derive(Debug, PartialEq, Eq)]
pub enum Removed {
    Done,
    NoEntry,
    Already,
}

/// Removes entry `id`, whatever its status, keeping that status for a restore.
pub async fn remove(pool: &SqlitePool, id: &str) -> Result<Removed, sqlx::Error> {
    let mut transaction = db::write_tx(pool).await?;
    let Some(status) = sqlx::query_scalar!("SELECT status FROM entries WHERE id = ?", id)
        .fetch_optional(&mut *transaction)
        .await?
    else {
        return Ok(Removed::NoEntry);
    };
    if status == "removed" {
        return Ok(Removed::Already);
    }
    let at = now();
    sqlx::query!(
        "UPDATE entries SET removed_from = status, removed_at = ?, status = 'removed'
         WHERE id = ? AND status != 'removed'",
        at,
        id
    )
    .execute(&mut *transaction)
    .await?;
    // A series version it alone held back is readable now.
    series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(Removed::Done)
}

/// What restoring an entry did.
#[derive(Debug, PartialEq, Eq)]
pub enum Restored {
    /// Back in the status held.
    Done(String),
    NoEntry,
    NotRemoved,
    Refused(Refusal),
}

/// Why a removed entry can't be restored.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Its account is deleted, and removed entries outlive it only for analysis.
    AccountDeleted,
    /// Version `by` has replaced the version this one replaced, so restoring it would
    /// give that version two current successors.
    Replaced { by: String },
    /// It was removed from a batch that is decided as one, and went on without it.
    BatchWentOn,
}

impl Refusal {
    /// The refusal as the owner reads it, of entry `id`.
    pub fn text(&self, id: &str) -> String {
        match self {
            Refusal::AccountDeleted => {
                format!("entry {id}'s account is deleted, so it stays removed")
            }
            Refusal::Replaced { by } => format!(
                "entry {by} has replaced the version {id} replaced, so restoring {id} would give that version two current successors"
            ),
            Refusal::BatchWentOn => format!(
                "entry {id} was removed from its batch before the batch was decided, and a batch is decided as one, so it can't come back alone"
            ),
        }
    }
}

/// Where an entry stands for the owner's Remove and Restore, as its admin page offers
/// them.
pub enum Standing {
    /// Not removed. `is_final` when removing it can't be undone, since its batch
    /// would go on without it.
    Removable { is_final: bool },
    /// Removed, and a restore puts it back as `to`.
    Restorable { to: String },
    /// Removed, and a restore is refused.
    Refused(Refusal),
}

/// The statuses a batch member passes through before its batch is decided. A member
/// removed from one of them can't come back alone: the batch was decided, or is
/// being decided, without it.
const UNSETTLED: [&str; 7] = [
    "pending",
    "screening",
    "review",
    "held",
    "frozen",
    "embargoed",
    "waiting_on_series",
];

struct Row {
    seq: i64,
    status: String,
    removed_from: Option<String>,
    previous_seq: Option<i64>,
    batch_seq: Option<i64>,
    account_deleted: bool,
}

async fn row(connection: &mut SqliteConnection, id: &str) -> Result<Option<Row>, sqlx::Error> {
    sqlx::query_as!(
        Row,
        r#"SELECT e.seq AS "seq!: i64", e.status, e.removed_from, e.previous_seq, e.batch_seq,
                  a.deleted_at IS NOT NULL AS "account_deleted!: bool"
           FROM entries e JOIN accounts a ON a.id = e.account_id
           WHERE e.id = ?"#,
        id
    )
    .fetch_optional(connection)
    .await
}

/// Whether `row`'s batch goes on without it once it is removed while in `status`:
/// the batch isn't decided yet, and another member is still in it.
async fn batch_goes_on(
    connection: &mut SqliteConnection,
    row: &Row,
    status: &str,
) -> Result<bool, sqlx::Error> {
    let Some(batch) = row.batch_seq.filter(|_| UNSETTLED.contains(&status)) else {
        return Ok(false);
    };
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM entries
               WHERE batch_seq = ? AND seq != ? AND status NOT IN ('withdrawn', 'removed')
           ) AS "others!: bool""#,
        batch,
        row.seq
    )
    .fetch_one(connection)
    .await
}

/// Why removed entry `row` can't go back to `status`, if it can't.
async fn refusal(
    connection: &mut SqliteConnection,
    row: &Row,
    status: &str,
) -> Result<Option<Refusal>, sqlx::Error> {
    if row.account_deleted {
        return Ok(Some(Refusal::AccountDeleted));
    }
    // A rejected or withdrawn version is no successor, so it forks nothing.
    if let Some(previous) = row.previous_seq
        && !matches!(status, "rejected" | "withdrawn")
        && let Some(by) = sqlx::query_scalar!(
            "SELECT id FROM entries
             WHERE previous_seq = ? AND seq != ?
               AND status NOT IN ('rejected', 'withdrawn', 'removed')",
            previous,
            row.seq
        )
        .fetch_optional(&mut *connection)
        .await?
    {
        return Ok(Some(Refusal::Replaced { by }));
    }
    if batch_goes_on(connection, row, status).await? {
        return Ok(Some(Refusal::BatchWentOn));
    }
    Ok(None)
}

/// Where entry `id` stands for removing and restoring; `None` if there is no such
/// entry.
pub async fn standing(pool: &SqlitePool, id: &str) -> Result<Option<Standing>, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let Some(row) = row(&mut connection, id).await? else {
        return Ok(None);
    };
    let Some(status) = row.removed_from.clone().filter(|_| row.status == "removed") else {
        let is_final = batch_goes_on(&mut connection, &row, &row.status).await?;
        return Ok(Some(Standing::Removable { is_final }));
    };
    Ok(Some(
        match refusal(&mut connection, &row, &status).await? {
            Some(refusal) => Standing::Refused(refusal),
            None => Standing::Restorable { to: status },
        },
    ))
}

/// Restores removed entry `id` to the status it had, unless `refusal` says why not.
pub async fn restore(pool: &SqlitePool, id: &str) -> Result<Restored, sqlx::Error> {
    let mut transaction = db::write_tx(pool).await?;
    let Some(row) = row(&mut transaction, id).await? else {
        return Ok(Restored::NoEntry);
    };
    let Some(status) = row.removed_from.clone().filter(|_| row.status == "removed") else {
        return Ok(Restored::NotRemoved);
    };
    if let Some(refusal) = refusal(&mut transaction, &row, &status).await? {
        return Ok(Restored::Refused(refusal));
    }
    sqlx::query!(
        "UPDATE entries SET status = removed_from, removed_from = NULL, removed_at = NULL
         WHERE seq = ? AND status = 'removed'",
        row.seq
    )
    .execute(&mut *transaction)
    .await?;
    series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    Ok(Restored::Done(status))
}
