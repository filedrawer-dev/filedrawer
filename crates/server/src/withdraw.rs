//! Deleting entries that were never public (design.md, Accounts; Entry lifecycle;
//! Agent interface): a token deletes a pending entry, or a whole pending batch, that
//! it submitted itself, and a browser session withdraws one that may still be
//! withdrawn, with its whole batch. Every rule is checked again inside the deleting
//! transaction, and the deletion itself is `deletion::delete_entries`.

use sqlx::{SqliteConnection, SqlitePool};

use crate::respond::one_line;
use crate::time::now;
use crate::{AppState, attachments, deletion, entries, series};

/// An entry's status, as the schema's CHECK lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Screening,
    Review,
    Held,
    Frozen,
    Embargoed,
    WaitingOnSeries,
    Public,
    Rejected,
    Withdrawn,
    Removed,
}

impl Status {
    /// The status as the database and the replies write it.
    pub fn name(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Screening => "screening",
            Status::Review => "review",
            Status::Held => "held",
            Status::Frozen => "frozen",
            Status::Embargoed => "embargoed",
            Status::WaitingOnSeries => "waiting_on_series",
            Status::Public => "public",
            Status::Rejected => "rejected",
            Status::Withdrawn => "withdrawn",
            Status::Removed => "removed",
        }
    }

    /// Whether an account can't see its own entry in this status.
    fn hidden(self) -> bool {
        matches!(self, Status::Withdrawn | Status::Removed)
    }
}

/// A token asking for a deletion: its account, and the token's own ID.
#[derive(Debug, Clone, Copy)]
pub struct Token<'a> {
    pub account_id: &'a str,
    pub token_id: &'a str,
}

/// Who asks to delete an entry.
#[derive(Debug, Clone, Copy)]
pub enum Asker<'a> {
    /// An API token: it deletes only pending entries it submitted itself.
    Token(Token<'a>),
    /// The account's browser session, withdrawing.
    Session { account_id: &'a str },
}

impl<'a> Asker<'a> {
    fn account_id(self) -> &'a str {
        match self {
            Asker::Token(token) => token.account_id,
            Asker::Session { account_id } => account_id,
        }
    }
}

/// Why an entry, or its batch, isn't deleted. Each names the entry it is about.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// It is public; withdrawing a public version is another matter (#31).
    #[error("entry {id} is public, and only an entry that was never public is deleted")]
    Public { id: String },
    /// A token deletes only a pending entry; this one is past its hold.
    #[error(
        "entry {id} is {}: a token deletes an entry only while it is pending, in its hour's hold; after that its author withdraws it from the account page in a browser, where that is still allowed",
        .status.name()
    )]
    NotPending { id: String, status: Status },
    /// It came in a batch, which a token deletes as one.
    #[error("entry {id} came in batch {batch}, which is deleted as one: DELETE /batches/{batch}")]
    InBatch { id: String, batch: String },
    /// Another of the account's tokens sent its text.
    #[error(
        "entry {id} was submitted with another token of this account, and only that token deletes it"
    )]
    OtherToken { id: String },
    /// It waits on the archive's owner: in review, held or frozen, or carrying the
    /// owner's verdict until the queue places it.
    #[error("entry {id} waits on the archive's owner")]
    WithOwner { id: String },
    /// It was rejected.
    #[error("entry {id} was rejected, so nothing of it is published")]
    Rejected { id: String },
    /// The archive's owner removed it, or withdrew it; a batch's member can be.
    #[error("entry {id} was removed by the archive's owner")]
    Removed { id: String },
    /// A later version of its chain exists; only the newest may be withdrawn.
    #[error("entry {id} has a later version, {newest}: withdraw that first")]
    Later { id: String, newest: String },
    /// A later version of its chain waits on the archive's owner, who can reject the
    /// chain.
    #[error(
        "the newest version of entry {id}'s chain, {newest}, waits on the archive's owner; ask the owner, who can reject the whole chain"
    )]
    LaterWithOwner { id: String, newest: String },
}

/// What a deletion did.
#[derive(Debug)]
pub enum Outcome {
    /// Deleted now, at `at`: these entries, in order.
    Deleted {
        at: String,
        ids: Vec<String>,
    },
    /// The account deleted it earlier, at `at`.
    AlreadyDeleted {
        at: String,
        ids: Vec<String>,
    },
    /// The account has no such entry, or batch, that it may see.
    NotFound,
    Refused(Refusal),
}

/// An entry as the rules read it.
struct Row {
    seq: i64,
    id: String,
    status: Status,
    batch_seq: Option<i64>,
    batch_id: Option<String>,
    token_id: Option<String>,
    review_reason: Option<String>,
    screen_verdict: Option<String>,
}

/// Which entries `rows` reads; a filter left `None` matches every entry.
#[derive(Default)]
struct Rows<'a> {
    account_id: Option<&'a str>,
    id: Option<&'a str>,
    batch_seq: Option<i64>,
    seq: Option<i64>,
}

/// The entries `filter` names, oldest first.
async fn rows(
    connection: &mut SqliteConnection,
    filter: Rows<'_>,
) -> Result<Vec<Row>, sqlx::Error> {
    let Rows {
        account_id,
        id,
        batch_seq,
        seq,
    } = filter;
    sqlx::query_as!(
        Row,
        r#"SELECT e.seq AS "seq!: i64", e.id, e.status AS "status: Status", e.batch_seq,
                  b.id AS "batch_id?", e.token_id, e.review_reason, e.screen_verdict
           FROM entries e LEFT JOIN batches b ON b.seq = e.batch_seq
           WHERE (?1 IS NULL OR e.account_id = ?1) AND (?2 IS NULL OR e.id = ?2)
             AND (?3 IS NULL OR e.batch_seq = ?3) AND (?4 IS NULL OR e.seq = ?4)
           ORDER BY e.seq"#,
        account_id,
        id,
        batch_seq,
        seq
    )
    .fetch_all(&mut *connection)
    .await
}

/// `account_id`'s entry `id`, unless it is in a status its account can't see.
async fn row(
    connection: &mut SqliteConnection,
    account_id: &str,
    id: &str,
) -> Result<Option<Row>, sqlx::Error> {
    let filter = Rows {
        account_id: Some(account_id),
        id: Some(id),
        ..Rows::default()
    };
    Ok(rows(connection, filter)
        .await?
        .pop()
        .filter(|row| !row.status.hidden()))
}

/// The newest live version of `seq`'s chain, if it isn't `seq` itself. A rejected,
/// withdrawn or removed version doesn't count.
async fn newest_version(
    connection: &mut SqliteConnection,
    seq: i64,
) -> Result<Option<Row>, sqlx::Error> {
    let newest = sqlx::query_scalar!(
        r#"WITH RECURSIVE later(seq) AS (
               SELECT e.seq FROM entries e
               WHERE e.previous_seq = ?1 AND e.status NOT IN ('rejected', 'withdrawn', 'removed')
               UNION ALL
               SELECT e.seq FROM entries e JOIN later l ON e.previous_seq = l.seq
               WHERE e.status NOT IN ('rejected', 'withdrawn', 'removed')
           )
           SELECT seq AS "seq!: i64" FROM later ORDER BY seq DESC LIMIT 1"#,
        seq
    )
    .fetch_optional(&mut *connection)
    .await?;
    let Some(newest) = newest else {
        return Ok(None);
    };
    let filter = Rows {
        seq: Some(newest),
        ..Rows::default()
    };
    Ok(rows(connection, filter).await?.pop())
}

/// Whether `token` may delete `row`: pending, out of any batch unless the batch is
/// what it deletes, and its text sent with this token.
fn token_may(row: &Row, token: Token<'_>, as_batch: bool) -> Result<(), Refusal> {
    let id = row.id.clone();
    match row.status {
        Status::Public => return Err(Refusal::Public { id }),
        Status::Pending => {}
        status => return Err(Refusal::NotPending { id, status }),
    }
    if let (false, Some(batch)) = (as_batch, &row.batch_id) {
        return Err(Refusal::InBatch {
            id,
            batch: batch.clone(),
        });
    }
    if row.token_id.as_deref() != Some(token.token_id) {
        return Err(Refusal::OtherToken { id });
    }
    Ok(())
}

/// Whether `row`'s status lets its account withdraw it (design.md, Entry lifecycle;
/// the owner's decision, 2026-11-08): never public, not rejected, and not waiting on
/// the archive's owner, which an entry also does while it reads `screening` with the
/// owner's verdict on it. The one place these statuses are listed.
fn withdrawable(row: &Row) -> Result<(), Refusal> {
    let id = row.id.clone();
    let owners_verdict =
        row.review_reason.is_some() || row.screen_verdict.as_deref() == Some("reject");
    match row.status {
        Status::Pending | Status::Embargoed | Status::WaitingOnSeries => Ok(()),
        Status::Screening if !owners_verdict => Ok(()),
        Status::Screening | Status::Review | Status::Held | Status::Frozen => {
            Err(Refusal::WithOwner { id })
        }
        Status::Rejected => Err(Refusal::Rejected { id }),
        Status::Public => Err(Refusal::Public { id }),
        Status::Withdrawn | Status::Removed => Err(Refusal::Removed { id }),
    }
}

/// `withdrawable`, and the newest version of its chain: only the newest is withdrawn,
/// and one waiting on the owner leaves the chain to the owner.
async fn session_may(
    connection: &mut SqliteConnection,
    row: &Row,
) -> Result<Result<(), Refusal>, sqlx::Error> {
    if let Err(refusal) = withdrawable(row) {
        return Ok(Err(refusal));
    }
    let Some(newest) = newest_version(connection, row.seq).await? else {
        return Ok(Ok(()));
    };
    let id = row.id.clone();
    Ok(match withdrawable(&newest) {
        Err(Refusal::WithOwner { .. }) => Err(Refusal::LaterWithOwner {
            id,
            newest: newest.id,
        }),
        _ => Err(Refusal::Later {
            id,
            newest: newest.id,
        }),
    })
}

/// What a deletion asks for: one entry, or a whole batch by token.
#[derive(Debug, Clone, Copy)]
enum Target<'a> {
    Entry { asker: Asker<'a>, id: &'a str },
    Batch { token: Token<'a>, id: &'a str },
}

impl<'a> Target<'a> {
    fn account_id(self) -> &'a str {
        match self {
            Target::Entry { asker, .. } => asker.account_id(),
            Target::Batch { token, .. } => token.account_id,
        }
    }
}

/// The entries a request would delete, and whether the rules allow it.
struct Doomed {
    rows: Vec<Row>,
    batch_id: Option<String>,
    allowed: Result<(), Refusal>,
}

/// What `target` would delete, checked by its asker's rules: by token, the entry or
/// every member of the batch; by browser session, the entry, or its whole batch.
/// `None` when the account can't see it.
async fn doomed(
    connection: &mut SqliteConnection,
    target: Target<'_>,
) -> Result<Option<Doomed>, sqlx::Error> {
    match target {
        Target::Entry {
            asker: Asker::Token(token),
            id,
        } => {
            let Some(row) = row(connection, token.account_id, id).await? else {
                return Ok(None);
            };
            let allowed = token_may(&row, token, false);
            Ok(Some(Doomed {
                rows: vec![row],
                batch_id: None,
                allowed,
            }))
        }
        Target::Entry {
            asker: Asker::Session { account_id },
            id,
        } => {
            let Some(row) = row(connection, account_id, id).await? else {
                return Ok(None);
            };
            let batch_id = row.batch_id.clone();
            let rows = match row.batch_seq {
                Some(batch_seq) => batch_rows(connection, batch_seq).await?,
                None => vec![row],
            };
            let mut allowed = Ok(());
            for row in &rows {
                allowed = session_may(connection, row).await?;
                if allowed.is_err() {
                    break;
                }
            }
            Ok(Some(Doomed {
                rows,
                batch_id,
                allowed,
            }))
        }
        Target::Batch { token, id } => {
            let batch_seq = sqlx::query_scalar!(
                r#"SELECT seq AS "seq!: i64" FROM batches WHERE id = ? AND account_id = ?"#,
                id,
                token.account_id
            )
            .fetch_optional(&mut *connection)
            .await?;
            let Some(batch_seq) = batch_seq else {
                return Ok(None);
            };
            let rows = batch_rows(connection, batch_seq).await?;
            let allowed = rows.iter().try_for_each(|row| token_may(row, token, true));
            Ok(Some(Doomed {
                rows,
                batch_id: Some(id.to_string()),
                allowed,
            }))
        }
    }
}

/// Every member of batch `batch_seq`, in order, but those the archive's owner removed:
/// they stay for analysis, and don't stop their author deleting the rest.
async fn batch_rows(
    connection: &mut SqliteConnection,
    batch_seq: i64,
) -> Result<Vec<Row>, sqlx::Error> {
    let filter = Rows {
        batch_seq: Some(batch_seq),
        ..Rows::default()
    };
    let mut rows = rows(connection, filter).await?;
    rows.retain(|row| !matches!(row.status, Status::Removed));
    Ok(rows)
}

/// Deletes what `target` names if its rules allow it, records the deletion for the
/// account, and commits, then removes the bytes of files no set holds any more.
async fn carry_out(state: &AppState, target: Target<'_>) -> Result<Outcome, sqlx::Error> {
    let account_id = target.account_id();
    let writer = state.files.write().await;
    let mut transaction = crate::db::write_tx(&state.pool).await?;
    let Some(doomed) = doomed(&mut transaction, target).await? else {
        drop(transaction);
        return already_deleted(&state.pool, target).await;
    };
    if let Err(refusal) = doomed.allowed {
        return Ok(Outcome::Refused(refusal));
    }
    let seqs: Vec<i64> = doomed.rows.iter().map(|row| row.seq).collect();
    let ids: Vec<String> = doomed.rows.into_iter().map(|row| row.id).collect();
    let at = now();
    deletion::delete_entries(&mut transaction, &seqs).await?;
    deletion::record_deleted(
        &mut transaction,
        account_id,
        &ids,
        doomed.batch_id.as_deref(),
        &at,
    )
    .await?;
    let lone = attachments::delete_lone_files(&writer, &mut transaction).await?;
    // A run's description or a series version may wait on these entries no more.
    series::mark_readable(&mut transaction).await?;
    transaction.commit().await?;
    attachments::remove_bytes(&writer, &lone).await;
    Ok(Outcome::Deleted { at, ids })
}

/// What a request that found nothing to delete answers: when the account deleted it,
/// if it did.
async fn already_deleted(pool: &SqlitePool, target: Target<'_>) -> Result<Outcome, sqlx::Error> {
    let account_id = target.account_id();
    Ok(match target {
        Target::Entry { id, .. } => match deletion::deleted_entry(pool, account_id, id).await? {
            Some(at) => Outcome::AlreadyDeleted {
                at,
                ids: vec![id.to_string()],
            },
            None => Outcome::NotFound,
        },
        Target::Batch { id, .. } => match deletion::deleted_batch(pool, account_id, id).await? {
            Some((at, ids)) => Outcome::AlreadyDeleted { at, ids },
            None => Outcome::NotFound,
        },
    })
}

/// Deletes entry `id` for `asker`: by token, a pending entry whose text it sent that
/// came in no batch; by browser session, an entry that may be withdrawn, with its
/// whole batch.
pub async fn delete_entry(
    state: &AppState,
    asker: Asker<'_>,
    id: &str,
) -> Result<Outcome, sqlx::Error> {
    carry_out(state, Target::Entry { asker, id }).await
}

/// Deletes batch `id` by `token`: every member, each pending and sent with it.
pub async fn delete_batch(
    state: &AppState,
    token: Token<'_>,
    id: &str,
) -> Result<Outcome, sqlx::Error> {
    carry_out(state, Target::Batch { token, id }).await
}

/// An entry a withdrawal would delete, as the confirm page lists it.
pub struct Member {
    pub id: String,
    pub status: &'static str,
    pub title: String,
}

/// Text a withdrawal would make public, since every other entry using it is public.
pub struct MadePublic {
    /// What it is: a run, or a series version.
    pub what: String,
    /// The run's description, or the series version's title.
    pub text: String,
}

/// What withdrawing an entry would do, for the confirm page.
pub struct Preview {
    /// The entry, or every member of its batch.
    pub members: Vec<Member>,
    /// Its batch, which is withdrawn as one.
    pub batch_id: Option<String>,
    /// Why it can't be withdrawn now, if it can't.
    pub refusal: Option<Refusal>,
    pub made_public: Vec<MadePublic>,
}

/// What withdrawing `account_id`'s entry `id` would delete and make public, and
/// whether the rules allow it. `None` when the account can't see the entry. What
/// goes public is found by deleting the entries in a transaction that is rolled
/// back, so the page tells what the withdrawal itself does.
pub async fn preview(
    pool: &SqlitePool,
    account_id: &str,
    id: &str,
) -> Result<Option<Preview>, sqlx::Error> {
    let mut trial = crate::db::write_tx(pool).await?;
    let target = Target::Entry {
        asker: Asker::Session { account_id },
        id,
    };
    let Some(doomed) = doomed(&mut trial, target).await? else {
        return Ok(None);
    };
    let mut members = Vec::new();
    for row in &doomed.rows {
        members.push(Member {
            id: row.id.clone(),
            status: row.status.name(),
            title: title_of(&mut trial, row.seq).await?,
        });
    }
    let seqs: Vec<i64> = doomed.rows.iter().map(|row| row.seq).collect();
    let made_public = match doomed.allowed {
        Ok(()) => made_public(&mut trial, &seqs).await?,
        Err(_) => Vec::new(),
    };
    trial.rollback().await?;
    Ok(Some(Preview {
        members,
        batch_id: doomed.batch_id,
        refusal: doomed.allowed.err(),
        made_public,
    }))
}

/// Entry `seq`'s title, on one line.
async fn title_of(connection: &mut SqliteConnection, seq: i64) -> Result<String, sqlx::Error> {
    let source = sqlx::query_scalar!("SELECT source FROM entries WHERE seq = ?", seq)
        .fetch_one(&mut *connection)
        .await?;
    Ok(analyzer::analyze(&source)
        .fields
        .title
        .as_deref()
        .map(one_line)
        .unwrap_or_default())
}

/// The runs' descriptions and series versions that deleting entries `seqs` makes
/// public: deletes them, in the caller's transaction, which must be rolled back, as
/// withdrawal does, then reads each run and version they used with the rules that
/// show it (`entries::run_description`, `series::mark_readable`).
async fn made_public(
    trial: &mut SqliteConnection,
    seqs: &[i64],
) -> Result<Vec<MadePublic>, sqlx::Error> {
    let mut runs = std::collections::BTreeSet::new();
    let mut cites = std::collections::BTreeSet::new();
    for &seq in seqs {
        let used = sqlx::query!(
            "SELECT account_id, run_id FROM entry_runs WHERE entry_seq = ?",
            seq
        )
        .fetch_all(&mut *trial)
        .await?;
        runs.extend(used.into_iter().map(|run| (run.account_id, run.run_id)));
        let cite = sqlx::query!(
            r#"SELECT e.series_seq AS "series_seq!: i64", e.series_version AS "version!: i64"
               FROM entries e JOIN series_versions v
                 ON v.series_seq = e.series_seq AND v.version = e.series_version
               WHERE e.seq = ? AND e.series_removed_at IS NULL AND v.readable_at IS NULL"#,
            seq
        )
        .fetch_optional(&mut *trial)
        .await?;
        cites.extend(cite.map(|cite| (cite.series_seq, cite.version)));
    }
    deletion::delete_entries(trial, seqs).await?;
    series::mark_readable(trial).await?;
    let mut shown = Vec::new();
    for (account_id, run_id) in runs {
        // The entries deleted weren't public, so the description wasn't shown before.
        if let Some(description) = entries::run_description(trial, &account_id, &run_id).await? {
            shown.push(MadePublic {
                what: format!("run {run_id}'s description"),
                text: one_line(&description),
            });
        }
    }
    for (series_seq, version) in cites {
        let readable = sqlx::query!(
            r#"SELECT s.id, v.title FROM series_versions v JOIN series s ON s.seq = v.series_seq
               WHERE v.series_seq = ? AND v.version = ? AND v.readable_at IS NOT NULL"#,
            series_seq,
            version
        )
        .fetch_optional(&mut *trial)
        .await?;
        if let Some(readable) = readable {
            shown.push(MadePublic {
                what: format!("series {}@{version}", readable.id),
                text: one_line(&readable.title),
            });
        }
    }
    Ok(shown)
}

/// An entry the account page offers to withdraw.
pub struct Listed {
    pub id: String,
    pub status: &'static str,
    pub title: String,
}

/// A chain whose newest version waits on the archive's owner, so none of it is
/// withdrawn from the account page.
pub struct WithOwner {
    /// The chain's latest version that its account could otherwise withdraw.
    pub id: String,
    pub title: String,
    pub newest: String,
}

/// The account's entries that may be withdrawn, newest first, and its chains whose
/// newest version waits on the owner.
pub async fn listing(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<(Vec<Listed>, Vec<WithOwner>), sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let filter = Rows {
        account_id: Some(account_id),
        ..Rows::default()
    };
    let rows = rows(&mut connection, filter).await?;
    let mut listed = Vec::new();
    let mut with_owner: Vec<WithOwner> = Vec::new();
    // Newest first, so the first entry met in a chain is its latest.
    for row in rows.into_iter().rev() {
        match session_may(&mut connection, &row).await? {
            Ok(()) => listed.push(Listed {
                title: title_of(&mut connection, row.seq).await?,
                id: row.id,
                status: row.status.name(),
            }),
            Err(Refusal::LaterWithOwner { newest, .. })
                if !with_owner.iter().any(|chain| chain.newest == newest) =>
            {
                with_owner.push(WithOwner {
                    title: title_of(&mut connection, row.seq).await?,
                    id: row.id,
                    newest,
                });
            }
            Err(_) => {}
        }
    }
    Ok((listed, with_owner))
}
