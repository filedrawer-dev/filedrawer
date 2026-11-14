//! Seam: `github_commit`, which names the repository and commit the server asks
//! GitHub about (entry-format.md, Fields: `code`): the owner and the repository go
//! into an API path sent with the server's token, so only plain names pass.

use filedrawer_analyzer::github_commit;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn a_github_link_whose_owner_or_repository_isnt_a_plain_name_names_no_commit() {
    for link in [
        // A bad owner with a good repository, and the other way round.
        format!("https://github.com/a%2Fb/repo/tree/{SHA}"),
        format!("https://github.com/owner/re%3Fpo/tree/{SHA}"),
        format!("https://github.com/own~er/repo/commit/{SHA}"),
        // `..` and `.` once `.git` is taken off.
        format!("https://github.com/owner/...git/tree/{SHA}"),
        format!("https://github.com/owner/..git/tree/{SHA}"),
    ] {
        assert_eq!(github_commit(&link), None, "{link}");
    }
}

#[test]
fn a_plain_owner_and_repository_are_lower_cased_without_dot_git() {
    let commit = github_commit(&format!(
        "https://github.com/Owner/Re-po_1.x.git/commit/{SHA}"
    ))
    .expect("a pinned GitHub link");

    assert_eq!(commit.repo, "owner/re-po_1.x");
    assert_eq!(commit.sha, SHA);
}
