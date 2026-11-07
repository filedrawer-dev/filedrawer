//! Deleting an account (design.md, Accounts): everything not yet public goes with
//! it, and its public entries stay without its name.

use sqlx::{SqliteConnection, SqlitePool};

use crate::time::now;
use crate::{AppState, allowance, attachments, series};

/// What deleting an account would do, for the confirm page.
pub struct Preview {
    /// Entries that would be deleted: every one that isn't public.
    pub unpublished: i64,
    /// Public entries that would stay, under "account deleted".
    pub public: i64,
    /// Hidden series the account made that hold another account's entries, which
    /// stop the deletion.
    pub blocking: Vec<String>,
}

/// What `delete` did.
#[derive(Debug)]
pub enum Outcome {
    Deleted,
    /// Nothing was deleted: hidden series it made hold other accounts' entries, as
    /// `preview` lists.
    Blocked,
}

// Deletion keeps public entries, under "account deleted", and the ones the owner
// removed, for analysis (design.md, Admin). Every query below spells those two
// statuses out.

pub async fn preview(pool: &SqlitePool, account_id: &str) -> Result<Preview, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let counts = sqlx::query!(
        r#"SELECT COALESCE(SUM(status NOT IN ('public', 'removed')), 0) AS "unpublished!: i64",
                  COALESCE(SUM(status = 'public'), 0) AS "public!: i64"
           FROM entries WHERE account_id = ?"#,
        account_id
    )
    .fetch_one(&mut *connection)
    .await?;
    Ok(Preview {
        unpublished: counts.unpublished,
        public: counts.public,
        blocking: blocking(&mut connection, account_id).await?,
    })
}

/// The hidden series `account_id` made that another account's live entry cites:
/// nobody else could make them public (the owner's decision, 2026-11-05).
async fn blocking(
    connection: &mut SqliteConnection,
    account_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT s.id FROM series s
         WHERE s.account_id = ?1 AND s.public_at IS NULL
           AND EXISTS (
               SELECT 1 FROM entries e
               WHERE e.series_seq = s.seq AND e.account_id != ?1
                 AND e.series_removed_at IS NULL
                 AND e.status NOT IN ('rejected', 'withdrawn', 'removed'))
         ORDER BY s.seq",
        account_id
    )
    .fetch_all(&mut *connection)
    .await
}

/// The account's password hash, to confirm a deletion.
pub async fn password_hash(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let hash = sqlx::query_scalar!(
        "SELECT password_hash FROM accounts WHERE id = ? AND deleted_at IS NULL",
        account_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(hash.flatten())
}

/// Deletes the account in one transaction, or names the series that stop it.
/// Deleted: its email, login, name, sessions, tokens, email links, private feedback
/// and series memberships; every entry of its that isn't public or removed, with what
/// was read from it; runs and batches nothing left uses; the hidden series it made;
/// the series the screen refused that the owner hasn't confirmed as a hazard; the
/// attachment sets no kept entry cites, and the files no remaining set holds, whose
/// bytes leave `files` once the rest has committed.
/// Kept: the account ID, its public entries, the public series it made, refused
/// series the owner confirmed, the attachment sets kept entries cite, and the
/// invite code it signed up with, so it still counts as one of the code's uses.
pub async fn delete(state: &AppState, account_id: &str) -> Result<Outcome, sqlx::Error> {
    let writer = state.files.write().await;
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    let blocked = blocking(&mut transaction, account_id).await?;
    if !blocked.is_empty() {
        return Ok(Outcome::Blocked);
    }
    // Its allowance goes with it, before the entries its units name.
    allowance::delete_account(&mut transaction, account_id).await?;
    delete_entries(&mut transaction, account_id).await?;
    delete_hidden_series(&mut transaction, account_id).await?;
    let deleted_at = now();
    for statement in [
        sqlx::query!("DELETE FROM sessions WHERE account_id = ?", account_id),
        sqlx::query!("DELETE FROM email_tokens WHERE account_id = ?", account_id),
        sqlx::query!("DELETE FROM tokens WHERE account_id = ?", account_id),
        sqlx::query!("DELETE FROM feedback WHERE account_id = ?", account_id),
        sqlx::query!(
            "DELETE FROM series_members WHERE account_id = ?",
            account_id
        ),
        // A series the owner confirmed as a hazard stays, like a removed entry.
        sqlx::query!(
            "DELETE FROM refused_series
             WHERE account_id = ? AND (decision IS NULL OR decision = 'dismissed')",
            account_id
        ),
    ] {
        statement.execute(&mut *transaction).await?;
    }
    // Attachment sets no kept entry cites go, and then files no set holds.
    attachments::delete_uncited_sets(&mut transaction, Some(account_id), None).await?;
    let lone = attachments::delete_lone_files(&writer, &mut transaction).await?;
    // The name column can't be empty; this placeholder is never shown, since every
    // read checks `deleted_at` first.
    sqlx::query!(
        "UPDATE accounts SET email = NULL, password_hash = NULL, email_verified_at = NULL,
             display_name = 'deleted', deleted_at = ?
         WHERE id = ?",
        deleted_at,
        account_id
    )
    .execute(&mut *transaction)
    .await?;
    // Its entries may have been all that held a version back.
    series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    attachments::remove_bytes(&writer, &lone).await;
    Ok(Outcome::Deleted)
}

/// Deletes every entry of the account that isn't public or removed, and the runs,
/// slugs and batches only those entries used.
async fn delete_entries(
    transaction: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    // Each statement names the doomed entries the same way; the last deletes them.
    for statement in [
        sqlx::query!(
            "DELETE FROM entry_models WHERE entry_seq IN (SELECT seq FROM entries
                 WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "DELETE FROM entry_tags WHERE entry_seq IN (SELECT seq FROM entries
                 WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "DELETE FROM entry_rows WHERE entry_seq IN (SELECT seq FROM entries
                 WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "DELETE FROM entry_unknown_fields WHERE entry_seq IN (SELECT seq FROM entries
                 WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "DELETE FROM entry_runs WHERE entry_seq IN (SELECT seq FROM entries
                 WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "DELETE FROM slugs WHERE entry_seq IN (SELECT seq FROM entries
                 WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "DELETE FROM entries_fts WHERE rowid IN (SELECT seq FROM entries
                 WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "UPDATE entries SET previous_seq = NULL WHERE previous_seq IN (SELECT seq
                 FROM entries WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        // The cost log keeps the spend, which still counts against the day's cap.
        sqlx::query!(
            "UPDATE screenings SET entry_seq = NULL WHERE entry_seq IN (SELECT seq
                 FROM entries WHERE account_id = ? AND status NOT IN ('public', 'removed'))",
            account_id
        ),
        sqlx::query!(
            "DELETE FROM entries WHERE account_id = ? AND status NOT IN ('public', 'removed')",
            account_id
        ),
    ] {
        statement.execute(&mut *transaction).await?;
    }
    sqlx::query!(
        "DELETE FROM runs WHERE account_id = ?1
           AND NOT EXISTS (SELECT 1 FROM entry_runs er
                           WHERE er.account_id = runs.account_id AND er.run_id = runs.id)",
        account_id
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!(
        "DELETE FROM batches WHERE account_id = ?
           AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.batch_seq = batches.seq)",
        account_id
    )
    .execute(&mut *transaction)
    .await?;
    Ok(())
}

/// Deletes the hidden series the account made, which no other account's live entry
/// cites by now. An entry that still names one (taken out of it, or rejected) loses
/// the cite.
async fn delete_hidden_series(
    transaction: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    let hidden = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM series WHERE account_id = ? AND public_at IS NULL"#,
        account_id
    )
    .fetch_all(&mut *transaction)
    .await?;
    for seq in hidden {
        for statement in [
            sqlx::query!(
                "UPDATE entries SET series_seq = NULL, series_version = NULL,
                     series_section = NULL, series_removed_at = NULL
                 WHERE series_seq = ?",
                seq
            ),
            sqlx::query!("DELETE FROM series_members WHERE series_seq = ?", seq),
            sqlx::query!("DELETE FROM series_versions WHERE series_seq = ?", seq),
            sqlx::query!("DELETE FROM series WHERE seq = ?", seq),
        ] {
            statement.execute(&mut *transaction).await?;
        }
    }
    Ok(())
}
