//! The owner's daily email (design.md, Admin): what screening published, what waits
//! for the owner, what failed, and new feedback, sent once a day after midnight UTC,
//! and a ping to healthchecks.io after each run.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use sqlx::SqlitePool;
use time::{Date, Duration, OffsetDateTime};

use crate::config::Environment;
use crate::error::with_causes;
use crate::event_log::Read;
use crate::link_checks::LinkEvents;
use crate::mail::Email;
use crate::respond::{DELIMITER, one_line};
use crate::screening::Failure;
use crate::time::{parse, parse_day, stamp};
use crate::{AppState, mail_queue, queue, search_log};

/// The most feedback items an email holds in full; past it the oldest are counted.
const FEEDBACK_IN_FULL: usize = 20;

/// What pinging returns: the error's text when the ping didn't go through.
pub type Pinging<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Where the job reports each run. The server holds one when
/// `FILEDRAWER_DAILY_EMAIL_PING` is set; tests pass a fake that keeps each ping.
pub trait Pinger: Send + Sync {
    /// Reports a run: `ok` for one that did its job, sent or with nothing to send.
    fn ping(&self, ok: bool) -> Pinging<'_>;
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DailyRun {
    /// Today's run has been done.
    NotDue,
    /// Nothing to report, so nothing was sent.
    Quiet,
    /// The email went to this many admins.
    Sent { to: usize },
    /// There was something to report but no admin with an address to send it to.
    NoOneToTell,
}

#[derive(Debug, thiserror::Error)]
pub enum DailyError {
    #[error("the database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("the email reached {sent} of the {of} admins it was due to")]
    Mail { sent: usize, of: usize },
}

/// One run of the job at `now`: when today's run (UTC) hasn't been done, emails
/// every admin what there is to report since the last run, if anything, then pings.
/// A run that fails leaves the period where it was, so the next run retries it.
pub async fn send_daily_email(
    state: &AppState,
    now: OffsetDateTime,
) -> Result<DailyRun, DailyError> {
    // With no run to start from, yesterday's midnight (UTC): the same for every
    // retry that day, so a retry knows which admins it already reached.
    let yesterday = (now.date() - Duration::days(1)).midnight().assume_utc();
    let start = match last_run(&state.pool).await? {
        Some(last) if last > now => {
            tracing::warn!(
                last = %stamp(last),
                "the daily email's last run is later than now; the clock was wrong then or is now"
            );
            yesterday
        }
        Some(last) if last.date() == now.date() => return Ok(DailyRun::NotDue),
        Some(last) => last,
        None => yesterday,
    };
    let result = run(state, start, now).await;
    let ok = match &result {
        Ok(DailyRun::NoOneToTell) => state.environment != Environment::Production,
        Ok(_) => true,
        Err(_) => false,
    };
    if let Some(pinger) = &state.pinger
        && let Err(error) = pinger.ping(ok).await
    {
        tracing::error!(%error, "the daily email's ping to healthchecks.io failed");
    }
    result
}

/// Reports the period from `start` to `now`.
async fn run(
    state: &AppState,
    start: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<DailyRun, DailyError> {
    let screen = state.screening_health.failures().read();
    let links = LinkReads::read(state.link_checker.events());
    let report = Report::gather(state, start, now, &screen, &links).await?;
    // Stores the run and marks the events read as reported, so the next run starts
    // after this one.
    let close_period = async || -> Result<(), sqlx::Error> {
        store_run(&state.pool, now).await?;
        state.screening_health.failures().drop_read(&screen);
        links.drop_from(state.link_checker.events());
        Ok(())
    };
    if !report.worth_sending(state.environment) {
        close_period().await?;
        return Ok(DailyRun::Quiet);
    }
    let admins = admins(&state.pool).await?;
    if admins.is_empty() {
        tracing::warn!("the daily email has something to report but no admin to send it to");
        close_period().await?;
        return Ok(DailyRun::NoOneToTell);
    }
    let subject = match state.environment {
        Environment::Production => "File Drawer daily report".to_string(),
        Environment::Staging => "File Drawer (staging) daily report".to_string(),
        Environment::Dev => "File Drawer (dev) daily report".to_string(),
    };
    let body = report.text(&state.public_url);
    let start_text = stamp(start);
    let (mut sent, mut due) = (0, 0);
    for admin in admins {
        if admin.sent_from.as_deref() == Some(start_text.as_str()) {
            continue;
        }
        due += 1;
        let email = Email {
            to: admin.email,
            subject: subject.clone(),
            body: body.clone(),
        };
        match state.mailer.send(email).await {
            Ok(()) => {
                store_sent(&state.pool, &admin.id, &start_text).await?;
                sent += 1;
            }
            Err(error) => {
                tracing::error!(%error, account = %admin.id, "the daily email wasn't sent")
            }
        }
    }
    if sent < due {
        return Err(DailyError::Mail { sent, of: due });
    }
    close_period().await?;
    Ok(DailyRun::Sent { to: sent })
}

/// What one email reports.
struct Report {
    start: OffsetDateTime,
    now: OffsetDateTime,
    /// When the server started, if after `start`: the in-memory counts run from then.
    started_at: Option<OffsetDateTime>,
    published: Vec<Published>,
    /// The accounts agents made in the period, and the accounts claimed in it,
    /// whenever they were made (#130).
    agent_made: i64,
    agent_claimed: i64,
    /// Entries in review or held, the oldest first.
    waiting: Vec<Waiting>,
    frozen: Vec<String>,
    /// Withdrawal notices' reasons the screen held for the owner, the oldest first.
    held_reasons: Vec<crate::notices::HeldReason>,
    screen: Read<Failure>,
    /// Whether the screen's last call failed, with no success after it.
    screen_failing: bool,
    cap_usd: f64,
    /// The UTC days whose cap ran out.
    cap_out: Vec<Date>,
    at_share: Vec<AtShare>,
    links: LinkReads,
    feedback: Vec<Feedback>,
    feedback_total: usize,
    backups: Backups,
    /// Emails the queue dropped unsent in the period, and those waiting at its end
    /// for longer than `mail_queue::LATE`.
    mail_dropped: i64,
    mail_late: i64,
    /// The days of the period on which a daily cap was half reached or more.
    caps: Vec<CapDay>,
}

/// A UTC day on which the read counter or the search log took half its daily cap or
/// more.
struct CapDay {
    what: &'static str,
    day: Date,
    counted: i64,
    cap: i64,
    /// Reads or searches past the cap, which went uncounted.
    dropped: i64,
}

/// An entry published in the period, and how it carries its code (#93).
struct Published {
    id: String,
    /// It names a `code` link.
    code_linked: bool,
    /// It cites an attachment set.
    code_attached: bool,
}

/// An entry waiting for the owner: in review or held.
struct Waiting {
    id: String,
    status: String,
    submitted_at: String,
}

/// An account that reached its share of a day's screening cap.
struct AtShare {
    account_id: String,
    day: Date,
}

/// What a run read from each of the link checks' logs.
#[derive(Clone)]
struct LinkReads {
    paused: Read<String>,
    half_spent: Read<()>,
    recheck_failures: Read<String>,
}

impl LinkReads {
    fn read(events: &LinkEvents) -> LinkReads {
        LinkReads {
            paused: events.paused.read(),
            half_spent: events.half_spent.read(),
            recheck_failures: events.recheck_failures.read(),
        }
    }

    /// Marks what was read reported.
    fn drop_from(&self, events: &LinkEvents) {
        events.paused.drop_read(&self.paused);
        events.half_spent.drop_read(&self.half_spent);
        events.recheck_failures.drop_read(&self.recheck_failures);
    }
}

struct Feedback {
    account_id: String,
    experiment: String,
    wanted: String,
    workaround: String,
    created_at: String,
}

/// The days from `first` to `last` on which the read counter, taking at most
/// `read_cap` readers a day, or the search log took half their cap or more. Each
/// drops only once full, so a day that dropped any is among them.
async fn cap_days(
    pool: &SqlitePool,
    read_cap: i64,
    first: Date,
    last: Date,
) -> Result<Vec<CapDay>, sqlx::Error> {
    let mut days = Vec::new();
    let mut day = first;
    while day <= last {
        let key = day.to_string();
        // A closed day's readers are its total; an open day's are still rows.
        let read = sqlx::query!(
            r#"SELECT IFNULL((SELECT readers FROM read_days WHERE day = ?1), 0)
                        + (SELECT COUNT(*) FROM entry_readers WHERE day = ?1) AS "counted!: i64",
                      IFNULL((SELECT dropped FROM read_days WHERE day = ?1), 0) AS "dropped!: i64""#,
            key
        )
        .fetch_one(pool)
        .await?;
        let searched = sqlx::query!(
            r#"SELECT (SELECT COUNT(*) FROM search_log WHERE day = ?1) AS "counted!: i64",
                      IFNULL((SELECT dropped FROM search_log_dropped WHERE day = ?1), 0) AS "dropped!: i64""#,
            key
        )
        .fetch_one(pool)
        .await?;
        let read = CapDay {
            what: "read counter",
            day,
            counted: read.counted,
            cap: read_cap,
            dropped: read.dropped,
        };
        let searched = CapDay {
            what: "search log",
            day,
            counted: searched.counted,
            cap: search_log::MAX_PER_DAY,
            dropped: searched.dropped,
        };
        days.extend(
            [read, searched]
                .into_iter()
                .filter(|load| load.counted * 2 >= load.cap),
        );
        day = day.next_day().expect("a day before the end of time");
    }
    Ok(days)
}

/// What `backups/` holds.
enum Backups {
    NotSetUp,
    Unreadable(String),
    NoCopy,
    Newest { name: String, date: Date },
}

impl Report {
    async fn gather(
        state: &AppState,
        start: OffsetDateTime,
        now: OffsetDateTime,
        screen: &Read<Failure>,
        links: &LinkReads,
    ) -> Result<Report, sqlx::Error> {
        let pool = &state.pool;
        let (from, to) = (stamp(start), stamp(now));
        let published = sqlx::query!(
            r#"SELECT id, source, attachment_set_seq IS NOT NULL AS "code_attached!: bool"
               FROM entries
               WHERE status = 'public' AND published_at > ?1 AND published_at <= ?2
               ORDER BY published_at, seq"#,
            from,
            to
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| Published {
            code_linked: analyzer::analyze(&row.source).fields.code.is_some(),
            id: row.id,
            code_attached: row.code_attached,
        })
        .collect();
        let agents = sqlx::query!(
            r#"SELECT COALESCE(SUM(made_by_agent_at > ?1 AND made_by_agent_at <= ?2), 0)
                          AS "made!: i64",
                      COALESCE(SUM(claimed_at > ?1 AND claimed_at <= ?2), 0) AS "claimed!: i64"
               FROM accounts WHERE made_by_agent_at IS NOT NULL"#,
            from,
            to
        )
        .fetch_one(pool)
        .await?;
        let standing = sqlx::query!(
            r#"SELECT id, status, submitted_at FROM entries
               WHERE status IN ('review', 'held', 'frozen')
               ORDER BY submitted_at, seq"#
        )
        .fetch_all(pool)
        .await?;
        let (frozen, waiting): (Vec<_>, Vec<_>) =
            standing.into_iter().partition(|row| row.status == "frozen");
        let held_reasons = crate::notices::held(pool).await?;
        let health = state.screening_health.snapshot();
        let screen_failing = health.last_failure.as_ref().is_some_and(|failure| {
            health
                .last_success
                .is_none_or(|success| success < failure.at)
        });
        let (cap_out, at_share) = spend(state, start.date(), now.date()).await?;
        let mut feedback: Vec<Feedback> = sqlx::query_as!(
            Feedback,
            r#"SELECT account_id, experiment, wanted, workaround, created_at FROM feedback
               WHERE created_at > ?1 AND created_at <= ?2
               ORDER BY created_at, rowid"#,
            from,
            to
        )
        .fetch_all(pool)
        .await?;
        let feedback_total = feedback.len();
        feedback.drain(..feedback_total.saturating_sub(FEEDBACK_IN_FULL));
        let (mail_dropped, mail_late) = mail_queue::dropped_and_late(pool, start, now).await?;
        let caps = cap_days(pool, state.reads.cap(), start.date(), now.date()).await?;
        Ok(Report {
            start,
            now,
            started_at: Some(state.started_at).filter(|started| *started > start),
            published,
            waiting: waiting
                .into_iter()
                .map(|row| Waiting {
                    id: row.id,
                    status: row.status,
                    submitted_at: row.submitted_at,
                })
                .collect(),
            agent_made: agents.made,
            agent_claimed: agents.claimed,
            frozen: frozen.into_iter().map(|row| row.id).collect(),
            held_reasons,
            screen: screen.clone(),
            screen_failing,
            cap_usd: state.screening_daily_nano_usd as f64 / 1e9,
            cap_out,
            at_share,
            links: links.clone(),
            feedback,
            feedback_total,
            backups: backups(&state.backups_dir),
            mail_dropped,
            mail_late,
            caps,
        })
    }

    /// Whether anything in it is worth an email in `environment`. Outside production
    /// only problems are, since CI publishes test entries on staging after each
    /// deploy (the owner's decision, 2026-11-09). Email dropped or late sends it in
    /// production only (the owner's decision, 2026-11-18).
    fn worth_sending(&self, environment: Environment) -> bool {
        let production = environment == Environment::Production;
        let recheck_failed = self.links.recheck_failures.count() > 0;
        let backups = match &self.backups {
            Backups::NotSetUp => production,
            Backups::Unreadable(_) | Backups::NoCopy => true,
            Backups::Newest { date, .. } => self.stale(*date),
        };
        // Half a cap only warns, like a host asking the server to wait; reads or
        // searches lost past it are a problem.
        let problems = (self.screen.count() > 0 && self.screen_failing)
            || !self.cap_out.is_empty()
            || self.caps.iter().any(|day| day.dropped > 0)
            || recheck_failed
            || backups
            || self.feedback_total > 0;
        let news = !self.published.is_empty()
            || !self.waiting.is_empty()
            || !self.frozen.is_empty()
            || !self.held_reasons.is_empty()
            || !self.at_share.is_empty()
            || !self.caps.is_empty();
        let mail = self.mail_dropped > 0 || self.mail_late > 0;
        problems || (production && (news || mail))
    }

    /// Whether a copy dated `date` is older than yesterday's.
    fn stale(&self, date: Date) -> bool {
        date < self.now.date() - Duration::days(1)
    }

    /// The email's body, its links under `url`.
    fn text(&self, url: &str) -> String {
        let mut text = format!(
            "File Drawer daily report, {} to {} (UTC).\n",
            stamp(self.start),
            stamp(self.now)
        );
        if let Some(started) = self.started_at {
            let _ = writeln!(
                text,
                "The screen's and the link checks' counts run from when the server started, at {}.",
                stamp(started)
            );
        }
        if !self.published.is_empty() {
            let _ = writeln!(text, "\nPublished ({}):", self.published.len());
            for Published { id, .. } in &self.published {
                let _ = writeln!(text, "- {id} {url}/admin/e/{id}");
            }
            self.code_line(&mut text);
        }
        if self.agent_made > 0 || self.agent_claimed > 0 {
            let _ = writeln!(
                text,
                "\nAccounts agents made: {}. Accounts claimed: {}.",
                self.agent_made, self.agent_claimed
            );
        }
        if let Some(oldest) = self.waiting.first() {
            let _ = writeln!(
                text,
                "\nWaiting for the owner ({}), the oldest submitted {} ago: {url}/admin",
                self.waiting.len(),
                self.age(&oldest.submitted_at)
            );
            for Waiting { id, status, .. } in &self.waiting {
                let _ = writeln!(text, "- {id} {status} {url}/admin/e/{id}");
            }
        }
        if !self.frozen.is_empty() {
            let _ = writeln!(text, "\nFrozen ({}):", self.frozen.len());
            for id in &self.frozen {
                let _ = writeln!(text, "- {id} {url}/admin/e/{id}");
            }
        }
        if !self.held_reasons.is_empty() {
            let _ = writeln!(
                text,
                "\nWithdrawal reasons held for the owner ({}), kept off their notices; no admin page shows the reason yet:",
                self.held_reasons.len()
            );
            for crate::notices::HeldReason { id, why } in &self.held_reasons {
                let _ = writeln!(
                    text,
                    "- {id} {}: notice {url}/e/{id}, entry {url}/admin/e/{id}",
                    one_line(why)
                );
            }
        }
        self.screening_lines(&mut text, url);
        self.link_lines(&mut text);
        self.mail_line(&mut text);
        self.cap_lines(&mut text);
        let _ = writeln!(text, "\nNightly copy: {}", self.backup_line());
        self.feedback_lines(&mut text, url);
        text
    }

    /// How many of the published entries link their code, attach it, or do neither,
    /// so the owner sees whether asking about code only after the post leaves most
    /// entries without it (#93). An entry that does both counts in both.
    fn code_line(&self, text: &mut String) {
        let linked = self.published.iter().filter(|e| e.code_linked).count();
        let attached = self.published.iter().filter(|e| e.code_attached).count();
        let neither = self
            .published
            .iter()
            .filter(|e| !e.code_linked && !e.code_attached)
            .count();
        let _ = writeln!(
            text,
            "Code: {linked} linked, {attached} attached, {neither} with neither."
        );
    }

    fn screening_lines(&self, text: &mut String, url: &str) {
        if let Some(last) = self.screen.events.last() {
            let failed = self.screen.count();
            let _ = writeln!(
                text,
                "\nScreening: {}; the last{} at {}: {}",
                counted(failed, "failed call"),
                if self.screen_failing {
                    " call failed,"
                } else {
                    ""
                },
                stamp(last.at),
                one_line(&last.error)
            );
        }
        if !self.cap_out.is_empty() {
            let days: Vec<String> = self.cap_out.iter().map(Date::to_string).collect();
            let _ = writeln!(
                text,
                "\nScreening cap: ran out on {}, of today's ${:.2} cap.",
                days.join(", "),
                self.cap_usd
            );
        }
        if !self.at_share.is_empty() {
            let _ = writeln!(text, "\nAccounts at their share of the screening cap:");
            for AtShare { account_id, day } in &self.at_share {
                let _ = writeln!(
                    text,
                    "- {account_id} on {day} {url}/admin/accounts/{account_id}"
                );
            }
        }
    }

    fn link_lines(&self, text: &mut String) {
        let links = &self.links;
        let mut lines = Vec::new();
        let rechecks = &links.recheck_failures;
        if let Some(error) = rechecks.events.last() {
            lines.push(format!(
                "- rechecks failed {}; the last: {}",
                counted(rechecks.count(), "time"),
                one_line(error)
            ));
        }
        let mut paused: BTreeMap<&str, u64> = BTreeMap::new();
        for host in &links.paused.events {
            *paused.entry(host).or_default() += 1;
        }
        if !paused.is_empty() {
            let mut hosts: Vec<String> = paused
                .iter()
                .map(|(host, times)| format!("{host} ({})", counted(*times, "time")))
                .collect();
            if links.paused.dropped > 0 {
                hosts.push(format!("and {} earlier", links.paused.dropped));
            }
            lines.push(format!(
                "- hosts that asked the server to wait: {}",
                hosts.join(", ")
            ));
        }
        let half_spent = links.half_spent.count();
        if half_spent > 0 {
            lines.push(format!(
                "- GitHub API calls not made because the server had spent its half of the hour's: {half_spent}"
            ));
        }
        if !lines.is_empty() {
            let _ = writeln!(text, "\nLink checks:\n{}", lines.join("\n"));
        }
    }

    fn mail_line(&self, text: &mut String) {
        if self.mail_dropped == 0 && self.mail_late == 0 {
            return;
        }
        let _ = writeln!(
            text,
            "\nEmail: {} dropped unsent, {} waiting over an hour.",
            self.mail_dropped, self.mail_late
        );
    }

    fn cap_lines(&self, text: &mut String) {
        if self.caps.is_empty() {
            return;
        }
        let _ = writeln!(text, "\nDaily caps:");
        for CapDay {
            what,
            day,
            counted,
            cap,
            dropped,
        } in &self.caps
        {
            let _ = write!(
                text,
                "- {what}, {day}: {counted} counted, of a cap of {cap} a day"
            );
            if *dropped > 0 {
                let _ = write!(text, "; {dropped} more went uncounted");
            }
            let _ = writeln!(text);
        }
    }

    fn backup_line(&self) -> String {
        match &self.backups {
            Backups::NotSetUp => "not set up: no backups/ beside the database".to_string(),
            Backups::Unreadable(error) => format!("can't read backups/: {}", one_line(error)),
            Backups::NoCopy => "none in backups/".to_string(),
            Backups::Newest { name, date } if self.stale(*date) => {
                format!("{name}, older than yesterday")
            }
            Backups::Newest { name, .. } => name.clone(),
        }
    }

    /// The feedback, below the delimiter: each item's server-written header and
    /// labels, and its author text with every line marked and no link live.
    fn feedback_lines(&self, text: &mut String, url: &str) {
        if self.feedback_total == 0 {
            return;
        }
        let _ = writeln!(text, "\nFeedback ({}): below.", self.feedback_total);
        let left_out = self.feedback_total - self.feedback.len();
        if left_out > 0 {
            let _ = writeln!(
                text,
                "{left_out} more feedback items came; the oldest are left out."
            );
        }
        let _ = writeln!(text, "\n{DELIMITER}");
        for item in &self.feedback {
            let _ = writeln!(
                text,
                "\nfeedback at {} from {url}/admin/accounts/{}",
                item.created_at, item.account_id
            );
            for (label, value) in [
                ("experiment", &item.experiment),
                ("wanted", &item.wanted),
                ("workaround", &item.workaround),
            ] {
                let _ = writeln!(text, "{label}:\n{}", marked(value));
            }
        }
    }

    /// How long ago `submitted` was, in whole days, else hours, else minutes.
    fn age(&self, submitted: &str) -> String {
        let Some(at) = parse(submitted) else {
            return "an unknown time".to_string();
        };
        let age = self.now - at;
        if age.whole_days() >= 1 {
            counted(age.whole_days() as u64, "day")
        } else if age.whole_hours() >= 1 {
            counted(age.whole_hours() as u64, "hour")
        } else {
            counted(age.whole_minutes().max(0) as u64, "minute")
        }
    }
}

/// `count` and `noun`, plural unless one.
fn counted(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Author text for the email: control characters and bidi marks removed, `://`
/// written `[:]//` so no link is live, and each line prefixed `| `, so none can
/// pass for one of the server's.
fn marked(text: &str) -> String {
    text.split(['\n', '\u{2028}', '\u{2029}'])
        .map(|line| {
            let clean: String = line
                .chars()
                .filter(|c| !c.is_control() && !analyzer::is_bidi_control(*c))
                .collect();
            format!("| {}", clean.replace("://", "[:]//"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The UTC days from `first` up to, not including, `today` whose cap ran out, and
/// the accounts that reached their share of one. Today's spend waits for tomorrow's
/// email, when the day is whole.
async fn spend(
    state: &AppState,
    first: Date,
    today: Date,
) -> Result<(Vec<Date>, Vec<AtShare>), sqlx::Error> {
    let (from, to) = (first.to_string(), today.to_string());
    let rows = sqlx::query!(
        r#"SELECT substr(at, 1, 10) AS "day!: String", account_id,
                  SUM(cost_nano_usd) AS "spent!: i64"
           FROM screenings WHERE at >= ?1 AND at < ?2
           GROUP BY 1, account_id ORDER BY 1, account_id"#,
        from,
        to
    )
    .fetch_all(&state.pool)
    .await?;
    let cap = state.screening_daily_nano_usd;
    let mut days: BTreeMap<Date, i64> = BTreeMap::new();
    let mut at_share = Vec::new();
    for row in rows {
        let Some(day) = parse_day(&row.day) else {
            continue;
        };
        *days.entry(day).or_default() += row.spent;
        if queue::held_to_share(state, &row.account_id) && queue::reached_share(row.spent, cap) {
            at_share.push(AtShare {
                account_id: row.account_id,
                day,
            });
        }
    }
    let cap_out = days
        .into_iter()
        .filter(|(_, spent)| *spent >= cap)
        .map(|(day, _)| day)
        .collect();
    Ok((cap_out, at_share))
}

/// The newest nightly copy in `dir`: a file named `filedrawer-YYYY-MM-DD.db`, by
/// the date in its name (#41's timer writes `*.tmp` and renames it when whole).
fn backups(dir: &Path) -> Backups {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Backups::NotSetUp,
        Err(error) => return Backups::Unreadable(error.to_string()),
    };
    let newest = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let date = name.strip_prefix("filedrawer-")?.strip_suffix(".db")?;
            Some((parse_day(date)?, name))
        })
        .max();
    match newest {
        Some((date, name)) => Backups::Newest { name, date },
        None => Backups::NoCopy,
    }
}

/// An admin and the start of the period they were last sent.
struct Admin {
    id: String,
    email: String,
    sent_from: Option<String>,
}

async fn admins(pool: &SqlitePool) -> Result<Vec<Admin>, sqlx::Error> {
    sqlx::query_as!(
        Admin,
        r#"SELECT a.id AS "id!", a.email AS "email!", s.period_start AS "sent_from?"
           FROM accounts a LEFT JOIN daily_email_sent s ON s.account_id = a.id
           WHERE a.is_admin = 1 AND a.email IS NOT NULL AND a.deleted_at IS NULL
           ORDER BY a.id"#
    )
    .fetch_all(pool)
    .await
}

async fn last_run(pool: &SqlitePool) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    let last = sqlx::query_scalar!("SELECT last_run_at FROM daily_email_runs WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    Ok(last.as_deref().and_then(parse))
}

async fn store_run(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let now = stamp(now);
    sqlx::query!(
        "INSERT INTO daily_email_runs (id, last_run_at) VALUES (1, ?)
         ON CONFLICT (id) DO UPDATE SET last_run_at = excluded.last_run_at",
        now
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn store_sent(pool: &SqlitePool, account_id: &str, start: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO daily_email_sent (account_id, period_start) VALUES (?, ?)
         ON CONFLICT (account_id) DO UPDATE SET period_start = excluded.period_start",
        account_id,
        start
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// How long a ping may take before the job gives up on it.
const PING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Pings a healthchecks.io check: its URL on success, and `<URL>/fail` on failure.
pub struct HttpPinger {
    url: String,
    http: reqwest::Client,
}

impl HttpPinger {
    pub fn new(url: &str) -> HttpPinger {
        HttpPinger {
            url: url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(PING_TIMEOUT)
                .build()
                .expect("a client with a timeout builds"),
        }
    }
}

impl Pinger for HttpPinger {
    fn ping(&self, ok: bool) -> Pinging<'_> {
        Box::pin(async move {
            let url = if ok {
                self.url.clone()
            } else {
                format!("{}/fail", self.url)
            };
            // The error leaves the URL out: anyone who has it can report the job.
            let answer = self
                .http
                .get(url)
                .send()
                .await
                .map_err(|error| with_causes(&error.without_url()))?;
            if answer.status().is_success() {
                Ok(())
            } else {
                Err(format!("healthchecks.io answered {}", answer.status()))
            }
        })
    }
}
