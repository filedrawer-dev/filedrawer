//! The screening queue (design.md, Submission and screening): entries in `screening`
//! are screened in the fair queue's order within the day's cost caps, their verdicts
//! stored, and each entry, or its whole batch, placed once decided. An account under
//! the review guard has its entries sent to review unscreened.

use analyzer::analyze;
use sqlx::SqlitePool;
use time::OffsetDateTime;

use crate::embargo::embargoed_until;
use crate::screening::{Call, MAX_ATTEMPTS, ModelError, Subject, Verdict};
use crate::{
    AppState, accounts, allowance, attachments, entries, removed_files, series, time::stamp, unpack,
};

/// The share of the day's cap one account may spend (design.md: 10%).
const ACCOUNT_SHARE_PERCENT: i64 = 10;

/// An entry waiting to be screened.
struct Waiting {
    seq: i64,
    account_id: String,
    source: String,
    screen_attempts: i64,
}

/// Screens entry `seq` at once, as when its author ends the hold early, unless it
/// cites attachments: those wait in the queue behind entries without (design.md,
/// Attached code), for the worker. Returns its status afterwards.
pub async fn screen_now(
    state: &AppState,
    seq: i64,
    now: OffsetDateTime,
) -> Result<Option<String>, sqlx::Error> {
    if entries::has_attachments(&state.pool, seq).await? {
        return entries::status(&state.pool, seq).await;
    }
    screen_entry(state, seq, now).await
}

/// Screens entry `seq` if it is in `screening` without a verdict, its account's
/// allowance has reached the earliest day among its units (design.md, Limits and
/// abuse), and the caps allow, then places it, or its batch, when decided. Returns
/// its status afterwards, or `None` if there is no such entry.
pub async fn screen_entry(
    state: &AppState,
    seq: i64,
    now: OffsetDateTime,
) -> Result<Option<String>, sqlx::Error> {
    // One screening at a time, so an entry is never screened twice at once and the
    // day's cap is overshot by at most one item's calls. The check for a verdict
    // comes after the lock, so a second caller finds the first one's.
    let _turn = state.screening_lock.lock().await;
    let today = allowance::day_text(now.date());
    let waiting = sqlx::query_as!(
        Waiting,
        r#"SELECT seq AS "seq!: i64", account_id, source, screen_attempts
           FROM entries WHERE seq = ? AND status = 'screening' AND screen_verdict IS NULL
             AND COALESCE((SELECT day FROM entry_screen_from
                           WHERE entry_seq = entries.seq), '') <= ?"#,
        seq,
        today
    )
    .fetch_optional(&state.pool)
    .await?;
    if let Some(waiting) = waiting {
        screen_waiting(state, &waiting, now).await?;
    }
    entries::status(&state.pool, seq).await
}

async fn screen_waiting(
    state: &AppState,
    waiting: &Waiting,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    // The files first: they're screened once by hash, and an entry whose files must
    // wait shouldn't spend on its text yet. A guarded or smoke-test account's files
    // aren't screened; its text decides, as `screen_text` does for it.
    let unscreened_account = state.smoke_account.as_deref() == Some(waiting.account_id.as_str())
        || accounts::is_guarded(&state.pool, &waiting.account_id).await?;
    let mut file_doubts = Vec::new();
    if !unscreened_account {
        match screen_files(state, waiting, now).await? {
            Files::Clear(doubts) => file_doubts = doubts,
            Files::Wait => return Ok(()),
            Files::Decided(verdict) => {
                entries::store_verdict(&state.pool, waiting.seq, &verdict).await?;
                return settle(state, waiting.seq, now).await;
            }
        }
    }
    let text = screened_text(state, waiting).await?;
    let verdict = match screen_text(state, &waiting.account_id, Some(waiting.seq), &text, now)
        .await?
    {
        Screened::Verdict(verdict) => verdict,
        Screened::Guarded => Verdict::Review {
            why: "the account is under the review guard".into(),
        },
        Screened::OverCap | Screened::Failed(ModelError::Http(_) | ModelError::Status { .. }) => {
            // Waits for a later tick; an outage doesn't count against the entry.
            return Ok(());
        }
        Screened::Failed(ModelError::Unreadable(_))
            if waiting.screen_attempts + 1 < MAX_ATTEMPTS =>
        {
            entries::count_screen_attempt(&state.pool, waiting.seq).await?;
            return Ok(());
        }
        Screened::Failed(ModelError::Unreadable(_)) => Verdict::Review {
            why: format!("screening replies couldn't be read {MAX_ATTEMPTS} times"),
        },
    };
    // The files' lesser doubts join the text's, for the owner's list.
    let verdict = match verdict {
        Verdict::Pass { note, hazard_doubt } if !file_doubts.is_empty() => Verdict::Pass {
            note,
            hazard_doubt: Some(
                hazard_doubt
                    .into_iter()
                    .chain(file_doubts)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        },
        verdict => verdict,
    };
    entries::store_verdict(&state.pool, waiting.seq, &verdict).await?;
    settle(state, waiting.seq, now).await
}

/// What the screen reads for an entry: its text, then the declaration of every run it
/// uses (design.md: "Screening checks run declarations with their batch"), since the
/// run's description is shown with it once public.
async fn screened_text(state: &AppState, waiting: &Waiting) -> Result<String, sqlx::Error> {
    let mut text = waiting.source.clone();
    for (id, declaration) in entries::run_declarations(&state.pool, waiting.seq).await? {
        text.push_str(&format!(
            "\n\nThe declaration of run `{id}`, which this entry uses:\n\n{declaration}\n"
        ));
    }
    Ok(text)
}

/// What screening one text came to.
pub enum Screened {
    Verdict(Verdict),
    /// The account is under the review guard, so nothing was screened: what it sends
    /// goes to the owner's review queue.
    Guarded,
    /// The day, or the account's share of it, is spent; nothing was called.
    OverCap,
    Failed(ModelError),
}

/// Screens `text` for `account_id` within the caps, and logs every call it made;
/// for staging's smoke-test account, passes it unscreened, and for an account under
/// the review guard, screens nothing and says so. The caller
/// holds `screening_lock`, or is the series route, which takes it here.
pub async fn screen_text(
    state: &AppState,
    account_id: &str,
    entry_seq: Option<i64>,
    text: &str,
    now: OffsetDateTime,
) -> Result<Screened, sqlx::Error> {
    // Staging's smoke-test account isn't screened, so CI's smoke test, which runs
    // after every deploy, calls no model; production refuses the setting.
    if state.smoke_account.as_deref() == Some(account_id) {
        return Ok(Screened::Verdict(Verdict::Pass {
            note: Some("staging smoke account: not screened".into()),
            hazard_doubt: None,
        }));
    }
    // design.md: once the owner rejects or holds one of an account's entries, everything it
    // submits goes straight to review, whatever the day's cap.
    if accounts::is_guarded(&state.pool, account_id).await? {
        return Ok(Screened::Guarded);
    }
    if over_cap(state, account_id, now).await? {
        return Ok(Screened::OverCap);
    }
    let screening = state.screener.screen(text).await;
    state.screening_health.note(&screening, Subject::Submission);
    log_calls(
        &state.pool,
        entry_seq,
        account_id,
        &screening.calls,
        now,
        Subject::Submission,
    )
    .await?;
    Ok(match screening.verdict {
        Ok(verdict) => Screened::Verdict(verdict),
        Err(error) => {
            tracing::warn!(%error, ?entry_seq, "screening failed");
            Screened::Failed(error)
        }
    })
}

/// Whether the day, or the account's share of it, is spent.
async fn over_cap(
    state: &AppState,
    account_id: &str,
    now: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    let (total, account) = spent_on(&state.pool, account_id, now).await?;
    let cap = state.screening_daily_nano_usd;
    let shared = held_to_share(state, account_id);
    Ok(total >= cap
        || (shared && account.saturating_mul(100) >= cap.saturating_mul(ACCOUNT_SHARE_PERCENT)))
}

/// Whether `account_id` is held to an account's share of the day's cap: every
/// account but staging's screen-test account, which answers only to the day's total.
fn held_to_share(state: &AppState, account_id: &str) -> bool {
    state.screen_test_account.as_deref() != Some(account_id)
}

/// Whether attached code may not be screened now: the day, or the account's share,
/// is spent, or attachments have spent half of either (design.md, Attached code).
async fn attachments_over_cap(
    state: &AppState,
    account_id: &str,
    now: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    if over_cap(state, account_id, now).await? {
        return Ok(true);
    }
    let day = format!("{}%", now.date());
    let row = sqlx::query!(
        r#"SELECT COALESCE(SUM(cost_nano_usd), 0) AS "total!: i64",
                  COALESCE(SUM(CASE WHEN account_id = ?2 THEN cost_nano_usd END), 0) AS "account!: i64"
           FROM screenings WHERE at LIKE ?1 AND attachment = 1"#,
        day,
        account_id
    )
    .fetch_one(&state.pool)
    .await?;
    let cap = state.screening_daily_nano_usd;
    let shared = held_to_share(state, account_id);
    Ok(row.total.saturating_mul(2) >= cap
        || (shared && row.account.saturating_mul(200) >= cap.saturating_mul(ACCOUNT_SHARE_PERCENT)))
}

/// What screening the attached files of `waiting` came to.
enum Files {
    /// Every file passed, or there are none: screen the entry's text. Holds each
    /// lesser hazard doubt, naming its file, for the owner.
    Clear(Vec<String>),
    /// A file waits for the caps or an outage; so does the entry.
    Wait,
    /// A file failed or went to review, and so does the entry.
    Decided(Verdict),
}

/// The screen's call on one file: a pass, with any lesser hazard doubt for the
/// owner, a rejection, or review, each with its reason.
enum FileCall {
    Pass(Option<String>),
    Reject(String),
    Review(String),
}

/// Screens the files of `waiting`'s set that have no verdict yet, chunk by chunk,
/// within the attachment caps. A pass or a rejection is stored by hash, for every
/// set that holds the file; review isn't, since the owner decides per entry.
async fn screen_files(
    state: &AppState,
    waiting: &Waiting,
    now: OffsetDateTime,
) -> Result<Files, sqlx::Error> {
    let mut doubts = Vec::new();
    let files = attachments::entry_files(&state.pool, waiting.seq).await?;
    // Only the files still to screen fund the budget, so a passed file cited again
    // doesn't pay for what a new one hides.
    let unscreened: i64 = files
        .iter()
        .filter(|file| file.verdict.is_none())
        .map(|file| file.bytes)
        .sum();
    let unscreened = usize::try_from(unscreened).expect("file sizes are stored from usize lengths");
    let mut budget = unpack::TextBudget::for_files(unscreened);
    for file in files {
        // A ban decides every entry still to be screened that cites the file; the
        // screen's own verdict stays on the file, for analysis.
        let call = match file.verdict.as_deref() {
            _ if file.banned => FileCall::Reject(removed_files::REMOVED_BY_OWNER.into()),
            Some("pass") => FileCall::Pass(file.reason.clone()),
            Some(_) => FileCall::Reject(file.reason.clone().unwrap_or_default()),
            None => match screen_file(state, waiting, &file, &mut budget, now).await? {
                Some(call) => call,
                None => return Ok(Files::Wait),
            },
        };
        match call {
            FileCall::Pass(doubt) => {
                doubts.extend(doubt.map(|doubt| format!("attached file {}: {doubt}", file.path)));
            }
            FileCall::Reject(reason) => {
                return Ok(Files::Decided(Verdict::Reject {
                    reasons: vec![format!("attached file {}: {reason}", file.path)],
                }));
            }
            FileCall::Review(why) => {
                return Ok(Files::Decided(Verdict::Review {
                    why: format!("attached file {}: {why}", file.path),
                }));
            }
        }
    }
    Ok(Files::Clear(doubts))
}

/// Screens one file, chunk by chunk, and stores a pass or a rejection. `None` when
/// it must wait: its bytes can't be read, the caps are spent, a call failed, or a
/// reply couldn't be read (counted against the entry, which goes to review after
/// too many).
async fn screen_file(
    state: &AppState,
    waiting: &Waiting,
    file: &attachments::EntryFile,
    budget: &mut unpack::TextBudget,
    now: OffsetDateTime,
) -> Result<Option<FileCall>, sqlx::Error> {
    let text = match state.files.read(&file.sha256).await {
        Ok(Some(text)) => text,
        Ok(None) => {
            tracing::error!(sha256 = %file.sha256, "an attached file has a record but no bytes; its entry waits");
            return Ok(None);
        }
        Err(error) => {
            tracing::error!(sha256 = %file.sha256, %error, "reading an attached file failed; its entry waits");
            return Ok(None);
        }
    };
    let text = match unpack::with_unpacked(&text, budget) {
        Ok(text) => text,
        Err(limit) => return Ok(Some(FileCall::Review(limit.to_string()))),
    };
    let mut doubts = Vec::new();
    for chunk in attachments::chunks(&text) {
        if attachments_over_cap(state, &waiting.account_id, now).await? {
            return Ok(None);
        }
        let screening = state.screener.screen_as(chunk, Subject::Code).await;
        state.screening_health.note(&screening, Subject::Code);
        log_calls(
            &state.pool,
            Some(waiting.seq),
            &waiting.account_id,
            &screening.calls,
            now,
            Subject::Code,
        )
        .await?;
        match screening.verdict {
            Ok(Verdict::Pass { hazard_doubt, .. }) => doubts.extend(hazard_doubt),
            Ok(Verdict::Reject { reasons }) => {
                let reason = reasons.join("; ");
                attachments::store_file_verdict(&state.pool, &file.sha256, "reject", Some(&reason))
                    .await?;
                return Ok(Some(FileCall::Reject(reason)));
            }
            Ok(Verdict::Review { why }) => return Ok(Some(FileCall::Review(why))),
            Err(error) => {
                // Logged as `screen_text` logs a failure on an entry's text.
                tracing::warn!(%error, entry_seq = waiting.seq, sha256 = %file.sha256, "screening attached code failed");
                return match error {
                    ModelError::Unreadable(_) if waiting.screen_attempts + 1 >= MAX_ATTEMPTS => {
                        Ok(Some(FileCall::Review(format!(
                            "screening replies couldn't be read {MAX_ATTEMPTS} times"
                        ))))
                    }
                    ModelError::Unreadable(_) => {
                        entries::count_screen_attempt(&state.pool, waiting.seq).await?;
                        Ok(None)
                    }
                    ModelError::Http(_) | ModelError::Status { .. } => Ok(None),
                };
            }
        }
    }
    let doubt = (!doubts.is_empty()).then(|| doubts.join("; "));
    attachments::store_file_verdict(&state.pool, &file.sha256, "pass", doubt.as_deref()).await?;
    Ok(Some(FileCall::Pass(doubt)))
}

/// What screening has spent today (UTC) in all, across accounts.
pub async fn spent_today(pool: &SqlitePool, now: OffsetDateTime) -> Result<i64, sqlx::Error> {
    // No account has an empty ID, so only the total counts.
    Ok(spent_on(pool, "", now).await?.0)
}

/// What screening has spent today (UTC), in all and by `account_id`.
pub async fn spent_on(
    pool: &SqlitePool,
    account_id: &str,
    now: OffsetDateTime,
) -> Result<(i64, i64), sqlx::Error> {
    let day = format!("{}%", now.date());
    let row = sqlx::query!(
        r#"SELECT COALESCE(SUM(cost_nano_usd), 0) AS "total!: i64",
                  COALESCE(SUM(CASE WHEN account_id = ?2 THEN cost_nano_usd END), 0) AS "account!: i64"
           FROM screenings WHERE at LIKE ?1"#,
        day,
        account_id
    )
    .fetch_one(pool)
    .await?;
    Ok((row.total, row.account))
}

/// Logs each model call and what it cost (design.md: "the server logs what each
/// screening cost").
/// Calls whose `subject` is attached code are marked so: they may spend at most
/// half the caps.
pub async fn log_calls(
    pool: &SqlitePool,
    entry_seq: Option<i64>,
    account_id: &str,
    calls: &[Call],
    now: OffsetDateTime,
    subject: Subject,
) -> Result<(), sqlx::Error> {
    let at = stamp(now);
    let attachment = subject == Subject::Code;
    for call in calls {
        let model = call.model.id();
        let cost = call.model.cost_nano_usd(call.usage);
        sqlx::query!(
            "INSERT INTO screenings (entry_seq, account_id, model, input_tokens, output_tokens,
                 cost_nano_usd, at, attachment)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            entry_seq,
            account_id,
            model,
            call.usage.input_tokens,
            call.usage.output_tokens,
            cost,
            at,
            attachment
        )
        .execute(pool)
        .await?;
        tracing::info!(model, cost_nano_usd = cost, ?entry_seq, "screening call");
    }
    Ok(())
}

/// A screened entry, or a member of its batch, as `settle` sees it.
struct Member {
    seq: i64,
    id: String,
    status: String,
    screen_verdict: Option<String>,
}

/// Places a screened entry, or decides its batch (design.md, Agent interface: "A
/// batch is screened and published as a unit"): one member rejected rejects every
/// member still in screening; a member in review goes to review and the passed ones
/// wait; when every member passed, all are placed.
async fn settle(state: &AppState, seq: i64, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let members = sqlx::query_as!(
        Member,
        r#"SELECT m.seq AS "seq!: i64", m.id, m.status, m.screen_verdict
           FROM entries e JOIN entries m
             ON m.seq = e.seq OR (e.batch_seq IS NOT NULL AND m.batch_seq = e.batch_seq)
           WHERE e.seq = ? AND m.status NOT IN ('withdrawn', 'removed')
           ORDER BY m.seq"#,
        seq
    )
    .fetch_all(&state.pool)
    .await?;
    let in_screening = |member: &&Member| member.status == "screening";
    if let Some(failed) = members
        .iter()
        .find(|member| member.screen_verdict.as_deref() == Some("reject"))
    {
        // A member in review or held stays for the owner to see (the owner's decision,
        // 2026-11-06); publishing it later sends it back here, with its batch. A
        // frozen member goes back with it, unscreened.
        let going_back =
            |member: &&Member| member.status == "screening" || member.status == "frozen";
        for member in members.iter().filter(going_back) {
            let reasons = (member.screen_verdict.as_deref() != Some("reject")).then(|| {
                serde_json::json!([format!(
                    "another entry in its batch, {}, was rejected, and a batch is screened as one",
                    failed.id
                )])
                .to_string()
            });
            entries::reject(&state.pool, member.seq, reasons.as_deref()).await?;
        }
        return Ok(());
    }
    for member in members.iter().filter(in_screening) {
        if member.screen_verdict.as_deref() == Some("review") {
            entries::to_review(&state.pool, member.seq).await?;
        }
    }
    // Placed only when every member passed; one in review, or not yet screened,
    // holds the rest in screening.
    let decided = members.iter().all(|member| {
        member.status == "screening" && member.screen_verdict.as_deref() == Some("pass")
    });
    if !decided {
        return Ok(());
    }
    for member in members.iter().filter(in_screening) {
        place_passed(state, member.seq, now).await?;
    }
    Ok(())
}

/// Places an entry that passed: embargoed while its date is ahead, waiting while a
/// series it or its batch cites is hidden, otherwise public.
async fn place_passed(state: &AppState, seq: i64, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let Some(source) = sqlx::query_scalar!(
        "SELECT source FROM entries WHERE seq = ? AND status = 'screening'",
        seq
    )
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(());
    };
    let analysis = analyze(&source);
    if embargoed_until(analysis.fields.embargo_until.as_deref(), now).is_some() {
        entries::embargo(&state.pool, seq).await?;
    } else if series::waits(&state.pool, seq).await? {
        entries::wait_on_series(&state.pool, seq).await?;
    } else {
        entries::publish(&state.pool, seq, &analysis).await?;
    }
    Ok(())
}

/// Places entry `seq`, or decides its batch, once the owner has given it a verdict.
pub async fn settle_decided(
    state: &AppState,
    seq: i64,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let _turn = state.screening_lock.lock().await;
    settle(state, seq, now).await
}

/// Places every entry left in `screening` with a verdict, as a crash between storing
/// the verdict and placing it would leave one, or a batch whose member in review
/// has been decided since.
pub async fn settle_screened(state: &AppState, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let _turn = state.screening_lock.lock().await;
    let screened = sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries
           WHERE status = 'screening' AND screen_verdict IS NOT NULL ORDER BY seq"#
    )
    .fetch_all(&state.pool)
    .await?;
    for seq in screened {
        settle(state, seq, now).await?;
    }
    Ok(())
}

/// Entries in `screening` with no verdict yet whose allowance day has come by `now`,
/// in the fair queue's order: by position, then submission time, so every account's
/// first goes ahead of anyone's second.
pub async fn unscreened(pool: &SqlitePool, now: OffsetDateTime) -> Result<Vec<i64>, sqlx::Error> {
    let today = allowance::day_text(now.date());
    sqlx::query_scalar!(
        r#"SELECT seq AS "seq!: i64" FROM entries
           WHERE status = 'screening' AND screen_verdict IS NULL
             AND COALESCE((SELECT day FROM entry_screen_from
                           WHERE entry_seq = entries.seq), '') <= ?
           ORDER BY attachment_set_seq IS NOT NULL, queue_position, submitted_at, seq"#,
        today
    )
    .fetch_all(pool)
    .await
}
