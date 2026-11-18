//! The outgoing mail queue (#103; design.md, Hosting and operations): every email the
//! server sends a person waits in `outgoing_mail` until the one sender sends it,
//! retried with a doubling wait and dropped unsent after a day.
//!
//! No row holds a link's secret or an address: the sender reads the account's address
//! and mints a verify or reset link as it sends, so neither sits in the database, its
//! replica or its nightly copies while mail is down. Delivery is at least once: a
//! restart between a send and its row's delete sends it again.

use sqlx::{SqliteConnection, SqlitePool};
use time::{Duration, OffsetDateTime};
use tokio::sync::Notify;

use crate::accounts::{self, EMAIL_LINK_HOURS, Purpose};
use crate::mail::{Email, MailError};
use crate::time::{now_utc, stamp};
use crate::{AppState, db};

/// The wait after a row's first failed try; it doubles with each try after.
const FIRST_WAIT: Duration = Duration::minutes(1);
/// The longest wait between tries.
const LONGEST_WAIT: Duration = Duration::hours(1);
/// How long a row waits before it is dropped unsent: a day-old signup or reset
/// email is stale, though its link would still work a day from the send.
pub const DROP_AFTER: Duration = Duration::hours(24);
/// How long a dropped row stays, for the daily email to count.
const KEEP_DROPPED: Duration = Duration::days(7);
/// A row waiting longer is late, which production's daily email reports.
pub const LATE: Duration = Duration::hours(1);
/// How often the sender looks for due rows when nothing wakes it.
const TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// An email to queue for an account.
pub enum Queued {
    /// A link to verify the account's address.
    Verify,
    /// A link to choose a new password.
    Reset,
    /// A note that someone tried to sign up with the account's address.
    Taken,
    /// A notice to an admin, with its own text.
    Notice { subject: String, body: String },
}

impl Queued {
    fn kind(&self) -> &'static str {
        match self {
            Queued::Verify => "verify",
            Queued::Reset => "reset",
            Queued::Taken => "taken",
            Queued::Notice { .. } => "notice",
        }
    }

    /// The email a stored row stands for; the table's CHECK allows only these kinds.
    fn stored(kind: &str, subject: Option<String>, body: Option<String>) -> Queued {
        match kind {
            "verify" => Queued::Verify,
            "reset" => Queued::Reset,
            "taken" => Queued::Taken,
            _ => Queued::Notice {
                subject: subject.unwrap_or_default(),
                body: body.unwrap_or_default(),
            },
        }
    }
}

/// The sender's state shared through `AppState`: what wakes it, the lock that keeps
/// it to one run at a time, and how its last sends went, for `GET /staging/status`.
#[derive(Default)]
pub struct MailQueue {
    wake: Notify,
    sender: tokio::sync::Mutex<Sender>,
    health: std::sync::Mutex<MailHealth>,
}

/// How the SMTP server has answered lately: after a failure that stops a run, no run
/// starts before `failing_until`, so a request can't make the server try SMTP again.
#[derive(Default)]
struct Sender {
    failures: i64,
    failing_until: Option<OffsetDateTime>,
}

/// The last send that went and the last that didn't, with its failure's class.
#[derive(Debug, Clone, Default)]
pub struct MailHealth {
    pub last_sent: Option<OffsetDateTime>,
    pub last_failure: Option<(OffsetDateTime, String)>,
}

impl MailQueue {
    pub fn health(&self) -> MailHealth {
        self.health.lock().expect("no panic holds the lock").clone()
    }

    fn note(&self, at: OffsetDateTime, failure: Option<&SendFailure>) {
        let mut health = self.health.lock().expect("no panic holds the lock");
        match failure {
            Some(failure) => health.last_failure = Some((at, failure.to_string())),
            None => health.last_sent = Some(at),
        }
    }
}

/// What a failed send says about the row and the server, kept as a class rather
/// than the error's text, which can hold the recipient's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendFailure {
    /// The address can't be parsed, or the email can't be built: it never will.
    Address,
    /// The server refused this recipient for good (550, 551, 553).
    Refused(u16),
    /// The server turned this email away for now, such as a full mailbox (452) or a
    /// greylist (450): it waits on its own, and the run goes on.
    Rejected(u16),
    /// The server can't take mail at all: it is closing (421), TLS is unavailable
    /// (454), or it refuses the login (530, 534, 535, 538).
    Server(u16),
    /// No answer: the connection, TLS or a timeout failed.
    Connect,
}

impl SendFailure {
    pub fn of(error: &MailError) -> SendFailure {
        match error {
            MailError::Address(_) | MailError::Build(_) => SendFailure::Address,
            MailError::Reply {
                code: code @ (550 | 551 | 553),
            } => SendFailure::Refused(*code),
            MailError::Reply {
                code: code @ (421 | 454 | 530 | 534 | 535 | 538),
            } => SendFailure::Server(*code),
            MailError::Reply { code } => SendFailure::Rejected(*code),
            MailError::Unreachable(_) | MailError::Setup(_) => SendFailure::Connect,
        }
    }
}

impl std::fmt::Display for SendFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendFailure::Address => write!(f, "address"),
            SendFailure::Refused(code) => write!(f, "refused {code}"),
            SendFailure::Rejected(code) | SendFailure::Server(code) => write!(f, "smtp {code}"),
            SendFailure::Connect => write!(f, "connect"),
        }
    }
}

/// Queues `mail` for `account_id` and wakes the sender. Signup and reset queue where
/// they once sent in the background, so a page answers as fast as before, whether
/// or not an account exists.
pub async fn queue(state: &AppState, account_id: &str, mail: Queued) -> Result<(), sqlx::Error> {
    let mut tx = db::write_tx(&state.pool).await?;
    queue_in(&mut tx, account_id, &mail, now_utc()).await?;
    tx.commit().await?;
    wake(state);
    Ok(())
}

/// Queues `mail` inside the caller's transaction, which then calls [`wake`] once it
/// commits. An account has at most one waiting row of each kind but `notice`: a new
/// one replaces it, so a person who asks again while mail is down gets one email.
pub async fn queue_in(
    conn: &mut SqliteConnection,
    account_id: &str,
    mail: &Queued,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let kind = mail.kind();
    if !matches!(mail, Queued::Notice { .. }) {
        sqlx::query!(
            "DELETE FROM outgoing_mail
             WHERE account_id = ? AND kind = ? AND dropped_at IS NULL",
            account_id,
            kind
        )
        .execute(&mut *conn)
        .await?;
    }
    let (subject, body) = match mail {
        Queued::Notice { subject, body } => (Some(subject.as_str()), Some(body.as_str())),
        _ => (None, None),
    };
    let now = stamp(now);
    sqlx::query!(
        "INSERT INTO outgoing_mail (account_id, kind, subject, body, created_at, next_try_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        account_id,
        kind,
        subject,
        body,
        now,
        now
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Wakes the sender to send what is due.
pub fn wake(state: &AppState) {
    state.mail_queue.wake.notify_one();
}

/// What one run of the sender did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub sent: usize,
    /// Rows dropped for a failure of their own.
    pub dropped: usize,
    /// Whether a failure stopped the run, or an earlier one kept it from starting.
    pub stopped: bool,
}

/// One run of the sender at `now`: drops the rows past [`DROP_AFTER`], then sends
/// each due row, oldest due first. A failure of the row's own drops it, or for one
/// the server may take later, puts it off by its own wait; a failure of the server's
/// stops the run and leaves the row due, and no run starts before the server's wait.
pub async fn send_due(state: &AppState, now: OffsetDateTime) -> Result<Run, sqlx::Error> {
    let queue = &state.mail_queue;
    let mut sender = queue.sender.lock().await;
    let mut run = Run::default();
    if sender.failing_until.is_some_and(|until| now < until) {
        run.stopped = true;
        return Ok(run);
    }
    drop_expired(&state.pool, now).await?;
    while let Some(row) = next_due(&state.pool, now).await? {
        let Some((email, link)) = prepare(state, &row).await? else {
            continue;
        };
        let Err(error) = state.mailer.send(email).await else {
            sqlx::query!("DELETE FROM outgoing_mail WHERE id = ?", row.id)
                .execute(&state.pool)
                .await?;
            run.sent += 1;
            queue.note(now, None);
            *sender = Sender::default();
            continue;
        };
        let failure = SendFailure::of(&error);
        tracing::warn!(%failure, kind = %row.kind, "an email wasn't sent");
        queue.note(now, Some(&failure));
        if let Some(link) = link {
            accounts::retire_email_link(&state.pool, &link).await?;
        }
        let class = failure.to_string();
        match failure {
            SendFailure::Address | SendFailure::Refused(_) => {
                drop_row(&state.pool, row.id, &failure, now).await?;
                run.dropped += 1;
                continue;
            }
            SendFailure::Server(_) | SendFailure::Connect => {
                sqlx::query!(
                    "UPDATE outgoing_mail SET last_error = ? WHERE id = ?",
                    class,
                    row.id
                )
                .execute(&state.pool)
                .await?;
                sender.failures += 1;
                sender.failing_until = Some(now + wait(sender.failures));
                run.stopped = true;
                break;
            }
            SendFailure::Rejected(_) => {}
        }
        let tries = row.tries + 1;
        let next = stamp(now + wait(tries));
        sqlx::query!(
            "UPDATE outgoing_mail SET tries = ?, next_try_at = ?, last_error = ? WHERE id = ?",
            tries,
            next,
            class,
            row.id
        )
        .execute(&state.pool)
        .await?;
    }
    Ok(run)
}

/// The wait after the `tries`th failure in a row: 1 minute, doubling, at most an hour.
fn wait(tries: i64) -> Duration {
    let doublings = u32::try_from(tries.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(6);
    (FIRST_WAIT * 2_i32.pow(doublings)).min(LONGEST_WAIT)
}

struct Row {
    id: i64,
    account_id: String,
    kind: String,
    subject: Option<String>,
    body: Option<String>,
    tries: i64,
}

async fn next_due(pool: &SqlitePool, now: OffsetDateTime) -> Result<Option<Row>, sqlx::Error> {
    let now = stamp(now);
    sqlx::query_as!(
        Row,
        "SELECT id, account_id, kind, subject, body, tries FROM outgoing_mail
         WHERE dropped_at IS NULL AND next_try_at <= ?
         ORDER BY next_try_at, id LIMIT 1",
        now
    )
    .fetch_optional(pool)
    .await
}

/// The email `row` stands for, with the hash of the link it minted, if any; or none,
/// with the row deleted, when it has no one to go to: the account is deleted or has
/// no address, its address is verified already, or it is no longer an admin. The
/// account is read and the link minted in one transaction, so a deletion can't come
/// between them.
async fn prepare(
    state: &AppState,
    row: &Row,
) -> Result<Option<(Email, Option<Vec<u8>>)>, sqlx::Error> {
    let mut tx = db::write_tx(&state.pool).await?;
    let account = sqlx::query!(
        r#"SELECT email, email_verified_at IS NOT NULL AS "verified: bool",
                  is_admin AS "is_admin: bool"
           FROM accounts WHERE id = ? AND deleted_at IS NULL"#,
        row.account_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let mail = Queued::stored(&row.kind, row.subject.clone(), row.body.clone());
    let to = account.as_ref().and_then(|account| account.email.clone());
    let wanted = account.as_ref().is_some_and(|account| match mail {
        Queued::Verify => !account.verified,
        Queued::Notice { .. } => account.is_admin,
        Queued::Reset | Queued::Taken => true,
    });
    let (Some(to), true) = (to, wanted) else {
        sqlx::query!("DELETE FROM outgoing_mail WHERE id = ?", row.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(None);
    };
    let url = &state.public_url;
    let (subject, body, link) = match mail {
        Queued::Verify => {
            let (secret, hash) =
                accounts::new_email_link(&mut tx, &row.account_id, Purpose::Verify).await?;
            let body = format!(
                "Someone, probably you, signed up for File Drawer with this address.\n\n\
                 To verify it, open this link and press the button:\n{url}/verify?token={secret}\n\n\
                 The link works for {EMAIL_LINK_HOURS} hours. If you didn't sign up, ignore this email.\n"
            );
            (
                "Verify your email for File Drawer".to_string(),
                body,
                Some(hash),
            )
        }
        Queued::Reset => {
            let (secret, hash) =
                accounts::new_email_link(&mut tx, &row.account_id, Purpose::Reset).await?;
            let body = format!(
                "Someone, probably you, asked to choose a new File Drawer password for this \
                 address.\n\nTo choose one, open this link:\n{url}/reset/confirm?token={secret}\n\n\
                 The link works for {EMAIL_LINK_HOURS} hours. If you didn't ask, ignore this \
                 email; your password hasn't changed.\n"
            );
            (
                "Choose a new File Drawer password".to_string(),
                body,
                Some(hash),
            )
        }
        Queued::Taken => {
            let body = format!(
                "Someone tried to sign up for File Drawer with this address, which already \
                 has an account.\n\nIf it was you, log in at {url}/login, or choose a new \
                 password at {url}/reset. If it wasn't, ignore this email.\n"
            );
            (
                "Someone tried to sign up with your email".to_string(),
                body,
                None,
            )
        }
        Queued::Notice { subject, body } => (subject, body, None),
    };
    tx.commit().await?;
    Ok(Some((Email { to, subject, body }, link)))
}

/// Marks a row dropped, keeping only its kind, account, times and failure's class.
async fn drop_row(
    pool: &SqlitePool,
    id: i64,
    failure: &SendFailure,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let (class, now) = (failure.to_string(), stamp(now));
    sqlx::query!(
        "UPDATE outgoing_mail
         SET dropped_at = ?, last_error = ?, subject = NULL, body = NULL WHERE id = ?",
        now,
        class,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Drops every row queued more than [`DROP_AFTER`] before `now`.
async fn drop_expired(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let (before, now) = (stamp(now - DROP_AFTER), stamp(now));
    sqlx::query!(
        "UPDATE outgoing_mail SET dropped_at = ?, subject = NULL, body = NULL
         WHERE dropped_at IS NULL AND created_at < ?",
        now,
        before
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// For the hourly tidier: drops the expired rows even when no run gets to them, and
/// deletes the rows dropped more than [`KEEP_DROPPED`] ago.
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    drop_expired(pool, now).await?;
    let before = stamp(now - KEEP_DROPPED);
    sqlx::query!("DELETE FROM outgoing_mail WHERE dropped_at < ?", before)
        .execute(pool)
        .await?;
    Ok(())
}

/// How many rows wait, and when the oldest was queued.
pub async fn waiting(pool: &SqlitePool) -> Result<(i64, Option<String>), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!: i64", MIN(created_at) AS oldest
           FROM outgoing_mail WHERE dropped_at IS NULL"#
    )
    .fetch_one(pool)
    .await?;
    Ok((row.count, row.oldest))
}

/// For the daily email: how many rows were dropped after `from` up to `to`, and how
/// many waited at `to` since more than [`LATE`] before it.
pub async fn dropped_and_late(
    pool: &SqlitePool,
    from: OffsetDateTime,
    to: OffsetDateTime,
) -> Result<(i64, i64), sqlx::Error> {
    let (late_before, from, to) = (stamp(to - LATE), stamp(from), stamp(to));
    let row = sqlx::query!(
        r#"SELECT
             COALESCE(SUM(dropped_at > ?1 AND dropped_at <= ?2), 0) AS "dropped!: i64",
             COALESCE(SUM(dropped_at IS NULL AND created_at < ?3), 0) AS "late!: i64"
           FROM outgoing_mail"#,
        from,
        to,
        late_before
    )
    .fetch_one(pool)
    .await?;
    Ok((row.dropped, row.late))
}

/// Sends what is due whenever a row is queued, and every minute, for
/// `filedrawer-server serve`. An error is logged and the loop goes on.
pub async fn run_sender(state: AppState) {
    let mut ticks = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            _ = ticks.tick() => {}
            () = state.mail_queue.wake.notified() => {}
        }
        if let Err(error) = send_due(&state, now_utc()).await {
            tracing::error!(%error, "sending queued email failed");
        }
    }
}
