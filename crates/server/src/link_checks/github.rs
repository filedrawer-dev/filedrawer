//! GitHub's fork check (design.md, Submission and screening, step 2): whether a
//! `code` commit is on one of the named repository's own branches, since GitHub
//! serves a fork's commits under its parent's URL. And the API calls it spends: the
//! server's hour, and each account's tenth of it.

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

impl LinkChecker {
    /// The calls one account may spend: a tenth of the server's an hour (the owner's
    /// decision, 2026-11-07), as screening holds an account to a tenth of its cap.
    fn account_share(&self) -> Rate {
        Rate {
            count: (self.github_hourly / 10).max(1),
            per: Duration::from_secs(60 * 60),
        }
    }

    fn quota(&self) -> std::sync::MutexGuard<'_, Option<(u32, i64)>> {
        self.github_quota
            .lock()
            .expect("no thread panics holding it")
    }

    /// Whether GitHub said the hour's calls are spent, and they haven't come back.
    fn github_spent(&self) -> bool {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        self.quota()
            .is_some_and(|(remaining, reset)| remaining == 0 && now < reset)
    }

    /// Whether fewer than half the hour's calls are left, so the weekly recheck
    /// leaves the rest to submissions.
    pub(super) fn github_low(&self) -> bool {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        self.quota()
            .is_some_and(|(remaining, reset)| remaining < self.github_hourly / 2 && now < reset)
    }

    fn note_quota(&self, answer: &ApiAnswer) {
        if let (Some(remaining), Some(reset)) = (answer.remaining, answer.reset) {
            *self.quota() = Some((remaining, reset));
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

/// One call to GitHub's API, within `account`'s share and the server's calls.
async fn github_call(
    state: &AppState,
    account: Option<&str>,
    path: &str,
) -> Result<ApiAnswer, String> {
    let checker = &state.link_checker;
    let spent = || "the server's GitHub API calls for the hour are spent".to_string();
    if checker.github_spent() {
        return Err(spent());
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
    match answer.status {
        403 | 429 if answer.remaining == Some(0) => Err(spent()),
        429 | 500..=599 => Err(format!("GitHub's API answered {}", answer.status)),
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
