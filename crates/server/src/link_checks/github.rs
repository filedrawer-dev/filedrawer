//! GitHub's fork check (design.md, Submission and screening, step 2): whether a
//! `code` commit is on one of the named repository's own branches, since GitHub
//! serves a fork's commits under its parent's URL. And the API calls it spends: at
//! most half of the hour's, each account a tenth of that half, and none while GitHub
//! asks the server to wait.

use std::time::Duration;

use analyzer::GithubCommit;
use sqlx::SqlitePool;
use time::OffsetDateTime;

use super::{ApiAnswer, LinkChecker, PinCheck};
use crate::AppState;
use crate::limits::Rate;

/// The most branches besides the default one that the check compares a commit with
/// (the owner's decision, 2026-11-07).
const MAX_OTHER_BRANCHES: usize = 10;

/// What a refusal for a fork's commit tells its author to do.
const FORK_ADVICE: &str = "GitHub shows a fork's commits under its parent's URL too, so link the repository whose branch holds it";

/// The pause key for GitHub's API.
const API_HOST: &str = "api.github.com";

impl LinkChecker {
    /// The calls the server may spend in an hour: half of GitHub's, since the token's
    /// account shares the rest with its owner (the owner's decision, 2026-11-07).
    fn github_budget(&self) -> u32 {
        self.github_hourly / 2
    }

    /// The calls one account may spend in an hour: a tenth of the server's (the
    /// owner's decision, 2026-11-07, made of the whole hour's; of the server's half,
    /// uncertainties.md, "An account's share of GitHub's API calls"), as screening
    /// holds an account to a tenth of its cap.
    fn account_share(&self) -> Rate {
        Rate {
            count: (self.github_budget() / 10).max(1),
            per: Duration::from_secs(60 * 60),
        }
    }

    fn quota(&self) -> std::sync::MutexGuard<'_, Option<(u32, i64)>> {
        self.github_quota
            .lock()
            .expect("no thread panics holding it")
    }

    /// Whether GitHub's last answer left fewer than `floor` calls this hour.
    fn github_below(&self, floor: u32) -> bool {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        self.quota()
            .is_some_and(|(remaining, reset)| remaining < floor && now < reset)
    }

    /// Whether the server has spent its half of the hour's calls.
    fn github_budget_spent(&self) -> bool {
        self.github_below(self.github_hourly - self.github_budget())
    }

    /// Whether fewer than three quarters of the hour's calls are left, so the
    /// rechecks leave the server's half to submissions.
    pub(super) fn github_low(&self) -> bool {
        let three_quarters = self.github_hourly / 4 * 3;
        self.github_below(three_quarters)
    }

    /// Whether GitHub's API asked the server to wait, and the wait isn't over.
    pub(super) fn github_paused(&self) -> bool {
        self.pauses.holds(API_HOST)
    }

    /// Keeps what GitHub's answer said is left. Answers to concurrent calls can
    /// arrive out of order, so within one hour the lowest count stands.
    fn note_quota(&self, answer: &ApiAnswer) {
        if let (Some(remaining), Some(reset)) = (answer.remaining, answer.reset) {
            let mut quota = self.quota();
            let remaining = match *quota {
                Some((before, same_reset)) if same_reset == reset => before.min(remaining),
                _ => remaining,
            };
            *quota = Some((remaining, reset));
        }
    }
}

/// Whether `commit` is on one of its repository's own branches: the default one,
/// then up to [`MAX_OTHER_BRANCHES`] others. `account` is charged for the calls.
pub(super) async fn github_pin(
    state: &AppState,
    commit: &GithubCommit,
    account: Option<&str>,
) -> PinCheck {
    let GithubCommit { repo, sha } = commit;
    if known_on_branch(&state.pool, commit).await {
        return PinCheck::Verified;
    }
    let head = match compare(state, account, repo, "HEAD", sha).await {
        Ok(head) => head,
        Err(why) => return PinCheck::NotVerified(why),
    };
    if head.on_branch {
        return record_on_branch(&state.pool, commit).await;
    }
    let branches = match github_call(
        state,
        account,
        &format!("/repos/{repo}/branches?per_page=100"),
    )
    .await
    {
        Ok(answer) if answer.status == 200 => answer.body.unwrap_or_default(),
        Ok(answer) => {
            return PinCheck::NotVerified(format!(
                "GitHub's API answered {} for the branches of {repo}",
                answer.status
            ));
        }
        Err(why) => return PinCheck::NotVerified(why),
    };
    // Only commit hashes go into the next calls' paths.
    let others: Vec<&str> = branches
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|branch| branch["commit"]["sha"].as_str())
        .filter(|branch_head| is_commit_hash(branch_head))
        .filter(|branch_head| Some(*branch_head) != head.base.as_deref())
        .collect();
    for branch_head in others.iter().take(MAX_OTHER_BRANCHES) {
        match compare(state, account, repo, branch_head, sha).await {
            Ok(comparison) if comparison.on_branch => {
                return record_on_branch(&state.pool, commit).await;
            }
            Ok(_) => {}
            Err(why) => return PinCheck::NotVerified(why),
        }
    }
    PinCheck::Failed(if others.len() <= MAX_OTHER_BRANCHES {
        format!("commit {sha} isn't on any branch of {repo}; {FORK_ADVICE}")
    } else {
        format!(
            "commit {sha} isn't on {repo}'s default branch or the {MAX_OTHER_BRANCHES} other branches the server checked, of {}; {FORK_ADVICE}",
            others.len()
        )
    })
}

fn is_commit_hash(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What GitHub's compare API said of a commit against a branch head.
struct Comparison {
    /// The commit is the head or behind it, so on that branch.
    on_branch: bool,
    /// The head it was compared with.
    base: Option<String>,
}

async fn compare(
    state: &AppState,
    account: Option<&str>,
    repo: &str,
    base: &str,
    sha: &str,
) -> Result<Comparison, String> {
    let answer = github_call(
        state,
        account,
        &format!("/repos/{repo}/compare/{base}...{sha}?per_page=1"),
    )
    .await?;
    if answer.status != 200 {
        return Err(format!(
            "GitHub's API answered {} for commit {sha} in {repo}",
            answer.status
        ));
    }
    let body = answer.body.unwrap_or_default();
    let on_branch = match body["status"].as_str() {
        Some("identical" | "behind") => true,
        Some("ahead" | "diverged") => false,
        _ => return Err(format!("GitHub's API compared commit {sha} with nothing")),
    };
    Ok(Comparison {
        on_branch,
        base: body["base_commit"]["sha"].as_str().map(str::to_string),
    })
}

/// Whether GitHub's answer is a rate limit: a 429, or a 403 with no calls left, with
/// `Retry-After`, or saying so (GitHub's REST docs, "Rate limits for the REST API").
fn rate_limited(answer: &ApiAnswer) -> bool {
    let says_so = || {
        answer
            .body
            .as_ref()
            .and_then(|body| body["message"].as_str())
            .is_some_and(|message| message.to_ascii_lowercase().contains("rate limit"))
    };
    answer.status == 429
        || (answer.status == 403
            && (answer.remaining == Some(0) || answer.retry_after.is_some() || says_so()))
}

/// One call to GitHub's API, within the server's half of the hour's calls and
/// `account`'s share, unless GitHub asked the server to wait.
async fn github_call(
    state: &AppState,
    account: Option<&str>,
    path: &str,
) -> Result<ApiAnswer, String> {
    let checker = &state.link_checker;
    let spent = || "the server has used its half of the hour's GitHub API calls".to_string();
    let waiting = || "GitHub's API asked the server to wait before calling it again".to_string();
    if checker.github_budget_spent() {
        return Err(spent());
    }
    if checker.pauses.holds(API_HOST) {
        return Err(waiting());
    }
    if let Some(account) = account {
        state
            .rate_limits
            .take(
                &format!("github:{account}"),
                checker.account_share(),
                std::time::Instant::now(),
            )
            .map_err(|_| {
                "this account has used its share of the server's GitHub API calls for the hour"
                    .to_string()
            })?;
    }
    let answer = checker
        .fetcher
        .github(path)
        .await
        .map_err(|why| format!("GitHub's API didn't answer ({why})"))?;
    checker.note_quota(&answer);
    if rate_limited(&answer) {
        // With no calls left, the quota waits for the reset by itself.
        if answer.remaining == Some(0) {
            return Err(spent());
        }
        checker.pauses.pause(API_HOST, answer.retry_after);
        return Err(waiting());
    }
    checker.pauses.clear(API_HOST);
    match answer.status {
        500..=599 => Err(format!("GitHub's API answered {}", answer.status)),
        _ => Ok(answer),
    }
}

/// Whether GitHub's API already confirmed `commit` on a branch.
async fn known_on_branch(pool: &SqlitePool, commit: &GithubCommit) -> bool {
    sqlx::query_scalar!(
        "SELECT 1 FROM github_commits_on_branches WHERE repo = ? AND sha = ?",
        commit.repo,
        commit.sha
    )
    .fetch_optional(pool)
    .await
    .is_ok_and(|row| row.is_some())
}

/// Records that `commit` is on a branch, so it isn't asked about again, and says it
/// is verified.
async fn record_on_branch(pool: &SqlitePool, commit: &GithubCommit) -> PinCheck {
    let now = crate::time::now();
    let recorded = sqlx::query!(
        "INSERT OR IGNORE INTO github_commits_on_branches (repo, sha, checked_at)
         VALUES (?, ?, ?)",
        commit.repo,
        commit.sha,
        now
    )
    .execute(pool)
    .await;
    if let Err(error) = recorded {
        tracing::warn!(%error, "recording a commit GitHub confirmed failed");
    }
    PinCheck::Verified
}
