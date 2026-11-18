//! Deleting an account (design.md, Accounts): everything not yet public goes with
//! it, and its public entries stay without its name. Also the deletion of entries
//! that an account's deletion shares with a token's `DELETE` and the account page's
//! withdrawal (`withdraw`), and the record of the entries deleted those two ways.

use std::collections::BTreeSet;

use sqlx::{SqliteConnection, SqlitePool};
use time::Duration;

use crate::time::{now, now_utc, stamp};
use crate::{AppState, allowance, attachments, series};

/// How long another account's rejected entry keeps a series it cites alive when the
/// series' creator deletes the account. Reason: a rejection may be the owner's
/// mistake, and its author fixes the entry and resubmits.
const REJECTION_KEEPS_SERIES_DAYS: i64 = 60;

/// What deleting an account would do, for the confirm page.
pub struct Preview {
    /// Entries that would be deleted: every one that isn't public.
    pub unpublished: i64,
    /// Public entries that would stay, under "account deleted".
    pub public: i64,
}

// Deletion keeps public entries, under "account deleted", and the ones the owner
// removed, for analysis (design.md, Admin). Every query below spells those two
// statuses out.

pub async fn preview(pool: &SqlitePool, account_id: &str) -> Result<Preview, sqlx::Error> {
    let counts = sqlx::query!(
        r#"SELECT COALESCE(SUM(status NOT IN ('public', 'removed')), 0) AS "unpublished!: i64",
                  COALESCE(SUM(status = 'public'), 0) AS "public!: i64"
           FROM entries WHERE account_id = ?"#,
        account_id
    )
    .fetch_one(pool)
    .await?;
    Ok(Preview {
        unpublished: counts.unpublished,
        public: counts.public,
    })
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

/// Deletes the account in one transaction.
/// Deleted: its email, login, name, sessions, tokens, email links, private feedback
/// and series memberships; every entry of its that isn't public or removed, with what
/// was read from it; runs and batches nothing left uses; the series it made that have
/// no readable version and no other account's entry cites; the series the screen
/// refused that the owner hasn't confirmed as a hazard; the attachment sets no kept
/// entry cites, and the files no remaining set holds, whose bytes leave `files` once
/// the rest has committed.
/// Kept: the account ID, its public entries, the other series it made, refused
/// series the owner confirmed, the attachment sets kept entries cite, and the
/// invite code it signed up with, so it still counts as one of the code's uses.
pub async fn delete(state: &AppState, account_id: &str) -> Result<(), sqlx::Error> {
    let writer = state.files.write().await;
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    // Its allowance goes with it, before the entries its units name.
    allowance::delete_account(&mut transaction, account_id).await?;
    let doomed = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries
           WHERE account_id = ? AND status NOT IN ('public', 'removed')"#,
        account_id
    )
    .fetch_all(&mut *transaction)
    .await?;
    delete_entries(&mut transaction, &doomed).await?;
    delete_unused(&mut transaction, account_id).await?;
    // Its entries may have been all that held a version back, so a series they
    // cited may be readable by now, and stays.
    series::mark_readable(&mut transaction).await?;
    delete_unread_series(&mut transaction, account_id).await?;
    let deleted_at = now();
    for statement in [
        sqlx::query!("DELETE FROM sessions WHERE account_id = ?", account_id),
        sqlx::query!("DELETE FROM email_tokens WHERE account_id = ?", account_id),
        // The kept entries forget the tokens that submitted them, which go next.
        sqlx::query!(
            "UPDATE entries SET token_id = NULL WHERE account_id = ?",
            account_id
        ),
        sqlx::query!("DELETE FROM tokens WHERE account_id = ?", account_id),
        sqlx::query!(
            "DELETE FROM deleted_entries WHERE account_id = ?",
            account_id
        ),
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
    transaction.commit().await?;
    attachments::remove_bytes(&writer, &lone).await;
    Ok(())
}

/// Deletes the account's runs and batches that nothing left uses, whichever entries
/// used them: the account-wide cleanup only its own deletion does.
async fn delete_unused(
    transaction: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
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

/// Deletes the series the account made that no one but their members could read: no
/// version is readable, and no other account's live entry cites one, nor one rejected
/// within [`REJECTION_KEEPS_SERIES_DAYS`] (the owner's decisions, 2026-11-17); any
/// other series outlives the account. An entry that still names one (taken out of it,
/// or rejected earlier) loses the cite. A series the owner removed stays, as removed
/// data does.
async fn delete_unread_series(
    transaction: &mut SqliteConnection,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    // No rejection time is stored; the latest submission stands in, since a
    // rejection follows it within the screening hold or the review queue.
    let rejected_since = stamp(now_utc() - Duration::days(REJECTION_KEEPS_SERIES_DAYS));
    let unread = sqlx::query_scalar!(
        r#"SELECT s.seq AS "seq!: i64" FROM series s
           WHERE s.account_id = ?1 AND s.removed_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM series_versions v
                 WHERE v.series_seq = s.seq AND v.readable_at IS NOT NULL)
             AND NOT EXISTS (
                 SELECT 1 FROM entries e
                 WHERE e.series_seq = s.seq AND e.account_id != ?1
                   AND e.series_removed_at IS NULL
                   AND (e.status NOT IN ('rejected', 'withdrawn', 'removed')
                        OR (e.status = 'rejected' AND e.submitted_at >= ?2)))"#,
        account_id,
        rejected_since
    )
    .fetch_all(&mut *transaction)
    .await?;
    for seq in unread {
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

/// Deletes entries `seqs`, none public or removed, in the caller's transaction, with
/// what was read from them, and then what only they used: their attachment sets, runs
/// and batches that no remaining entry cites. A set's files leave it, and its row is
/// marked deleted rather than dropped, so its bytes still count toward the day's
/// uploads (`attachments::delete_uncited_set`); the files no set holds any more go
/// with `attachments::delete_lone_files`, which the caller runs. Each entry's
/// allowance unit stays, with no entry, and the cost log keeps what its screening
/// spent.
pub async fn delete_entries(
    transaction: &mut SqliteConnection,
    seqs: &[i64],
) -> Result<(), sqlx::Error> {
    let mut sets = BTreeSet::new();
    let mut batches = BTreeSet::new();
    let mut runs = BTreeSet::new();
    for &seq in seqs {
        let used = sqlx::query!(
            "SELECT attachment_set_seq, batch_seq FROM entries WHERE seq = ?",
            seq
        )
        .fetch_one(&mut *transaction)
        .await?;
        sets.extend(used.attachment_set_seq);
        batches.extend(used.batch_seq);
        let cited = sqlx::query!(
            "SELECT account_id, run_id FROM entry_runs WHERE entry_seq = ?",
            seq
        )
        .fetch_all(&mut *transaction)
        .await?;
        runs.extend(cited.into_iter().map(|run| (run.account_id, run.run_id)));
        delete_entry(transaction, seq).await?;
    }
    let deleted_at = now();
    for set in sets {
        attachments::delete_uncited_set(transaction, set, &deleted_at).await?;
    }
    for (account_id, run_id) in runs {
        sqlx::query!(
            "DELETE FROM runs WHERE account_id = ?1 AND id = ?2
               AND NOT EXISTS (SELECT 1 FROM entry_runs er
                               WHERE er.account_id = ?1 AND er.run_id = ?2)",
            account_id,
            run_id
        )
        .execute(&mut *transaction)
        .await?;
    }
    for batch in batches {
        sqlx::query!(
            "DELETE FROM batches WHERE seq = ?1
               AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.batch_seq = ?1)",
            batch
        )
        .execute(&mut *transaction)
        .await?;
    }
    Ok(())
}

/// Deletes entry `seq` and what was read from it. A later version that names it
/// loses the link; its allowance unit and its screenings' costs stay, with no entry.
async fn delete_entry(transaction: &mut SqliteConnection, seq: i64) -> Result<(), sqlx::Error> {
    for statement in [
        sqlx::query!("DELETE FROM entry_models WHERE entry_seq = ?", seq),
        sqlx::query!("DELETE FROM entry_tags WHERE entry_seq = ?", seq),
        sqlx::query!("DELETE FROM entry_rows WHERE entry_seq = ?", seq),
        sqlx::query!("DELETE FROM entry_references WHERE entry_seq = ?", seq),
        sqlx::query!("DELETE FROM entry_unknown_fields WHERE entry_seq = ?", seq),
        sqlx::query!("DELETE FROM entry_runs WHERE entry_seq = ?", seq),
        sqlx::query!("DELETE FROM slugs WHERE entry_seq = ?", seq),
        sqlx::query!("DELETE FROM entries_fts WHERE rowid = ?", seq),
        sqlx::query!(
            "UPDATE entries SET previous_seq = NULL WHERE previous_seq = ?",
            seq
        ),
        // The cost log keeps the spend, which still counts against the day's cap.
        sqlx::query!(
            "UPDATE screenings SET entry_seq = NULL WHERE entry_seq = ?",
            seq
        ),
        // The unit isn't given back; it stays an entry's, never a series'.
        sqlx::query!(
            "UPDATE allowance_units SET entry_seq = NULL WHERE entry_seq = ?",
            seq
        ),
        sqlx::query!("DELETE FROM entries WHERE seq = ?", seq),
    ] {
        statement.execute(&mut *transaction).await?;
    }
    Ok(())
}

/// Records that `account_id` deleted entries `ids` at `at`, with batch `batch_id`
/// when they came in one, so its later read or retry is told when.
pub async fn record_deleted(
    transaction: &mut SqliteConnection,
    account_id: &str,
    ids: &[String],
    batch_id: Option<&str>,
    at: &str,
) -> Result<(), sqlx::Error> {
    for id in ids {
        sqlx::query!(
            "INSERT INTO deleted_entries (id, account_id, batch_id, deleted_at)
             VALUES (?, ?, ?, ?)",
            id,
            account_id,
            batch_id,
            at
        )
        .execute(&mut *transaction)
        .await?;
    }
    Ok(())
}

/// When `account_id` deleted entry `id`, if it did.
pub async fn deleted_entry(
    pool: &SqlitePool,
    account_id: &str,
    id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT deleted_at FROM deleted_entries WHERE id = ? AND account_id = ?",
        id,
        account_id
    )
    .fetch_optional(pool)
    .await
}

/// When `account_id` deleted batch `batch_id`, and its members' IDs, if it did.
pub async fn deleted_batch(
    pool: &SqlitePool,
    account_id: &str,
    batch_id: &str,
) -> Result<Option<(String, Vec<String>)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, deleted_at FROM deleted_entries
         WHERE batch_id = ? AND account_id = ? ORDER BY rowid",
        batch_id,
        account_id
    )
    .fetch_all(pool)
    .await?;
    let Some(at) = rows.first().map(|row| row.deleted_at.clone()) else {
        return Ok(None);
    };
    Ok(Some((at, rows.into_iter().map(|row| row.id).collect())))
}
