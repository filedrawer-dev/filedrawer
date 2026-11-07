//! GitHub's fork check (design.md, Submission and screening, step 2): whether a
//! `code` commit is on one of the named repository's own branches or in one of its
//! tags, since GitHub serves a fork's commits under its parent's URL; GitHub's
//! commit page says first, and its API when the page doesn't. And the API calls it
//! spends: at most half of the hour's, each account a tenth of that half,
//! and none while GitHub asks the server to wait.

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

/// What the warning for a commit on no branch says it may be, and how to clear it
/// (the owner, 2026-11-07: "warn that the commit could be pruned, and encourage the
/// agent to tag it").
fn no_branch_advice(sha: &str) -> String {
    format!(
        "it may be a fork's commit, which GitHub shows under its parent's URL too, and GitHub may prune a commit that no branch or tag holds. If it is yours, push a tag at it (`git tag NAME {sha} && git push origin NAME`) and a recheck clears the flag; if it is a fork's, link the fork"
    )
}

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

/// Whether `commit` is its repository's own: on one of its branches or in one of its
/// tags, as GitHub's commit page says, or else as GitHub's API does (`api_pin`).
/// Only the repository's writers can push a branch or a tag, and GitHub doesn't
/// prune a commit one holds. On none of them, the commit is a fork's or its branch
/// was squashed or rebased away, which GitHub can't tell apart, so it is flagged and
/// never refused (the owner's decision, 2026-11-07). `account` is charged for the
/// API's calls.
pub(super) async fn github_pin(
    state: &AppState,
    commit: &GithubCommit,
    account: Option<&str>,
) -> PinCheck {
    if known_confirmed(&state.pool, commit).await {
        return PinCheck::Verified;
    }
    match commit_page(state, commit).await {
        CommitPage::Held => record_confirmed(&state.pool, commit).await,
        CommitPage::HeldByNone => PinCheck::OnNoBranch(on_no_branch_or_tag(commit)),
        CommitPage::Unread => api_pin(state, commit, account).await,
    }
}

/// "On no branch or tag", with how to clear the flag, for a complete answer.
fn on_no_branch_or_tag(commit: &GithubCommit) -> String {
    let GithubCommit { repo, sha } = commit;
    format!(
        "commit {sha} is on no branch or tag of {repo}: {}",
        no_branch_advice(sha)
    )
}

/// What GitHub's commit page said of a commit.
enum CommitPage {
    /// A branch or tag of the repository contains it.
    Held,
    /// No branch or tag does.
    HeldByNone,
    /// It said something else, or nothing: ask the API.
    Unread,
}

/// Which branches and tags of the repository contain `commit`, as the list behind
/// GitHub's own "does not belong to any branch" banner says:
/// `github.com/{repo}/branch_commits/{sha}`, which spends no API calls and lists
/// every branch and tag holding the commit, not just their tips. It isn't
/// documented, so any answer but 200 with both lists is `Unread`, and the check goes
/// to the API (the owner's decision, 2026-11-07: "build it with the fallback").
async fn commit_page(state: &AppState, commit: &GithubCommit) -> CommitPage {
    page_lists(state, commit)
        .await
        .map_or(CommitPage::Unread, |held| {
            if held {
                CommitPage::Held
            } else {
                CommitPage::HeldByNone
            }
        })
}

/// Whether the commit page lists any branch or tag holding `commit`; `None` when
/// its answer isn't that list.
async fn page_lists(state: &AppState, commit: &GithubCommit) -> Option<bool> {
    let url = format!(
        "https://github.com/{}/branch_commits/{}",
        commit.repo, commit.sha
    );
    let answer = state
        .link_checker
        .fetch_json_from("github.com", &url)
        .await
        .ok()?;
    if answer.status != 200 {
        return None;
    }
    let body = answer.body?;
    let branches = body["branches"].as_array()?;
    let tags = body["tags"].as_array()?;
    // A branch is listed as `{"branch": name, ...}` and a tag as its name: anything
    // else isn't the list this reads.
    let real = branches.iter().all(|branch| branch["branch"].is_string())
        && tags.iter().all(serde_json::Value::is_string);
    real.then_some(!branches.is_empty() || !tags.is_empty())
}

/// The fork check through GitHub's API: whether `commit` is on its repository's
/// default branch, at one of its tags, or on up to [`MAX_OTHER_BRANCHES`] other
/// branches. `account` is charged for the calls.
async fn api_pin(state: &AppState, commit: &GithubCommit, account: Option<&str>) -> PinCheck {
    let GithubCommit { repo, sha } = commit;
    let head = match compare(state, account, repo, "HEAD", sha).await {
        Ok(head) => head,
        Err(why) => return PinCheck::NotVerified(why),
    };
    if head.on_branch {
        return record_confirmed(&state.pool, commit).await;
    }
    // A tag list that can't be read still leaves the branches to try.
    let tags = list(state, account, repo, Listing::Tags).await;
    if let Ok(tags) = &tags
        && targets(tags).any(|tagged| tagged.eq_ignore_ascii_case(sha))
    {
        return record_confirmed(&state.pool, commit).await;
    }
    let branches = match list(state, account, repo, Listing::Branches).await {
        Ok(branches) => branches,
        Err(why) => return PinCheck::NotVerified(why),
    };
    let others: Vec<&str> = targets(&branches)
        .filter(|branch_head| Some(*branch_head) != head.base.as_deref())
        .collect();
    for branch_head in others.iter().take(MAX_OTHER_BRANCHES) {
        match compare(state, account, repo, branch_head, sha).await {
            Ok(comparison) if comparison.on_branch => {
                return record_confirmed(&state.pool, commit).await;
            }
            Ok(_) => {}
            Err(why) => return PinCheck::NotVerified(why),
        }
    }
    let tags = match tags {
        Ok(tags) => tags,
        Err(why) => return PinCheck::NotVerified(why),
    };
    let advice = no_branch_advice(sha);
    let every_branch = others.len() <= MAX_OTHER_BRANCHES;
    let every_tag = tags.as_array().map_or(0, Vec::len) < LISTED;
    PinCheck::OnNoBranch(match (every_branch, every_tag) {
        (true, true) => on_no_branch_or_tag(commit),
        (true, false) => format!(
            "commit {sha} is on no branch of {repo}, nor at any of the first {LISTED} of its tags, the ones the server checked: {advice}"
        ),
        (false, every_tag) => {
            let tags = if every_tag {
                "no tag".to_string()
            } else {
                format!("none of the first {LISTED} tags")
            };
            format!(
                "commit {sha} is on {tags} of {repo}, nor on its default branch or the {MAX_OTHER_BRANCHES} other branches the server checked, of {}: {advice}",
                others.len()
            )
        }
    })
}

/// How many tags or branches one call lists.
const LISTED: usize = 100;

/// What `list` lists.
#[derive(Clone, Copy)]
enum Listing {
    Tags,
    Branches,
}

impl Listing {
    /// Its name in GitHub's API paths, and in messages.
    fn name(self) -> &'static str {
        match self {
            Listing::Tags => "tags",
            Listing::Branches => "branches",
        }
    }
}

/// The first [`LISTED`] of `repo`'s tags or branches, as GitHub's API lists them.
async fn list(
    state: &AppState,
    account: Option<&str>,
    repo: &str,
    listing: Listing,
) -> Result<serde_json::Value, String> {
    let what = listing.name();
    let answer = github_call(
        state,
        account,
        &format!("/repos/{repo}/{what}?per_page={LISTED}"),
    )
    .await?;
    if answer.status != 200 {
        return Err(format!(
            "GitHub's API answered {} for the {what} of {repo}",
            answer.status
        ));
    }
    Ok(answer.body.unwrap_or_default())
}

/// The commit each listed tag or branch points at. Only commit hashes, which go
/// into later calls' paths.
fn targets(listed: &serde_json::Value) -> impl Iterator<Item = &str> {
    listed
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item["commit"]["sha"].as_str())
        .filter(|target| is_commit_hash(target))
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

/// Whether GitHub already confirmed `commit` on a branch or in a tag.
async fn known_confirmed(pool: &SqlitePool, commit: &GithubCommit) -> bool {
    sqlx::query_scalar!(
        "SELECT 1 FROM github_commits_confirmed WHERE repo = ? AND sha = ?",
        commit.repo,
        commit.sha
    )
    .fetch_optional(pool)
    .await
    .is_ok_and(|row| row.is_some())
}

/// Records that `commit` is on a branch or in a tag, so it isn't asked about again,
/// and says it is verified.
async fn record_confirmed(pool: &SqlitePool, commit: &GithubCommit) -> PinCheck {
    let now = crate::time::now();
    let recorded = sqlx::query!(
        "INSERT OR IGNORE INTO github_commits_confirmed (repo, sha, checked_at)
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
