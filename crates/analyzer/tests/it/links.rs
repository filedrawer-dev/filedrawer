//! Seam: `code_pin`, which says whether a `code` link is pinned by its shape, and
//! `github_commit`, which names the repository and commit the server asks GitHub
//! about (entry-format.md, Fields: `code`): the owner and the repository go into an
//! API path sent with the server's token, so only plain names pass.

use filedrawer_analyzer::{
    CardContext, Pin, Refusal, analyze, code_pin, github_commit, render_card,
};

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

/// A hash pins a link only where it selects what the link opens: the path segment
/// after `tree`, `blob` and the like. Anywhere else the link still opens a branch,
/// which a later push moves (design.md, Submission and screening, step 2).
#[test]
fn a_commit_hash_outside_the_place_that_selects_the_commit_pins_nothing() {
    for link in [
        format!("https://github.com/owner/repo#{SHA}"),
        format!("https://github.com/owner/repo?{SHA}"),
        format!("https://github.com/owner/repo/tree/main#{SHA}"),
        format!("https://github.com/owner/repo/tree/main?{SHA}"),
        format!("https://github.com/owner/repo/tree/main/{SHA}"),
        format!("https://github.com/owner/repo/blob/main/{SHA}/x.py"),
        format!("https://github.com/owner/repo/issues/{SHA}"),
        format!("https://github.com/owner/repo/{SHA}"),
        format!("https://github.com/owner/repo/archive/refs/heads/main.zip?{SHA}"),
        // A browser resolves `..` before it asks, so these open `tree/main`.
        format!("https://github.com/owner/repo/tree/{SHA}/../../tree/main"),
        format!("https://github.com/owner/repo/tree/{SHA}/%2e%2e/%2e%2e/tree/main"),
        format!("https://huggingface.co/owner/model#{SHA}"),
        format!("https://huggingface.co/owner/model/tree/main?{SHA}"),
        format!("https://huggingface.co/owner/model/tree/main/{SHA}"),
        format!("https://huggingface.co/datasets/owner/data/{SHA}"),
        format!("https://huggingface.co/datasets/owner/data/tree/main#{SHA}"),
    ] {
        assert_eq!(code_pin(&link), Pin::Unpinned, "{link}");
        assert_eq!(github_commit(&link), None, "{link}");
    }
}

#[test]
fn a_commit_hash_where_github_selects_the_commit_pins_the_link() {
    for link in [
        format!("https://github.com/owner/repo/tree/{SHA}"),
        format!("https://github.com/owner/repo/tree/{SHA}/src"),
        format!("https://github.com/owner/repo/tree/{SHA}?tab=readme-ov-file#usage"),
        format!("https://github.com/owner/repo/blob/{SHA}/src/x.py#L3"),
        format!("https://github.com/owner/repo/raw/{SHA}/src/x.py"),
        format!("https://github.com/owner/repo/blame/{SHA}/src/x.py"),
        format!("https://github.com/owner/repo/commit/{SHA}"),
        format!("https://github.com/owner/repo/pull/7/commits/{SHA}"),
        format!("https://github.com/owner/repo/archive/{SHA}.zip"),
        format!("https://github.com/owner/repo/archive/{SHA}.tar.gz"),
        format!("https://github.com/owner/repo/zipball/{SHA}"),
        format!("https://github.com/owner/repo/tarball/{SHA}"),
    ] {
        assert_eq!(code_pin(&link), Pin::Pinned, "{link}");
        let commit = github_commit(&link).expect("a pinned GitHub link");
        assert_eq!(commit.repo, "owner/repo", "{link}");
        assert_eq!(commit.sha, SHA, "{link}");
    }
}

/// The refusal tells an author where the hash goes, since `entry-check` prints it
/// to agents that have a hash and put it in the wrong place.
#[test]
fn the_refusal_of_an_unpinned_link_shows_how_to_write_a_pinned_one() {
    let url = format!("https://github.com/owner/repo#{SHA}");
    let analysis = analyze(&format!("---\ncode: {url}\n---\n"));
    let refusal = analysis
        .refusals
        .iter()
        .find(|refusal| **refusal == Refusal::UnpinnedCode { url: url.clone() })
        .expect("the link is refused as unpinned")
        .to_string();
    for part in [
        url.as_str(),
        "https://github.com/<owner>/<repo>/tree/<commit>",
        "https://huggingface.co/<owner>/<repo>/tree/<commit>",
        "query or fragment",
    ] {
        assert!(refusal.contains(part), "{part}: {refusal}");
    }
}

#[test]
fn a_commit_hash_where_hugging_face_selects_the_revision_pins_the_link() {
    for link in [
        format!("https://huggingface.co/owner/model/tree/{SHA}"),
        format!("https://huggingface.co/owner/model/blob/{SHA}/config.json"),
        format!("https://huggingface.co/owner/model/resolve/{SHA}/config.json?download=true"),
        format!("https://huggingface.co/owner/model/raw/{SHA}/config.json"),
        format!("https://huggingface.co/owner/model/commit/{SHA}"),
        format!("https://huggingface.co/owner/model/commits/{SHA}"),
        format!("https://huggingface.co/owner/model/blame/{SHA}/train.py"),
        // A model with a bare name.
        format!("https://huggingface.co/gpt2/tree/{SHA}"),
        format!("https://huggingface.co/datasets/owner/data/tree/{SHA}"),
        format!("https://huggingface.co/datasets/owner/data/resolve/{SHA}/run.jsonl"),
        format!("https://huggingface.co/spaces/owner/demo/blob/{SHA}/app.py"),
    ] {
        assert_eq!(code_pin(&link), Pin::Pinned, "{link}");
    }
}

/// GitHub's and Hugging Face's own sites go by other spellings too: `www.`, a final
/// dot, `hf.co`, and a pip-style `git+https://` link. A link spelled so takes the
/// pin rule, so one at no commit is refused as it is on `github.com`. The server
/// fetches only the exact listed hosts, so a pinned one keeps the unlisted host's
/// "pin not verified" rather than lose its flag without its check.
#[test]
fn another_spelling_of_a_listed_site_takes_the_pin_rule() {
    for link in [
        "https://www.github.com/owner/repo".to_string(),
        "https://github.com./owner/repo".to_string(),
        "https://WWW.GitHub.COM./owner/repo/tree/main".to_string(),
        "https://www.huggingface.co/owner/model".to_string(),
        "https://huggingface.co./owner/model/tree/main".to_string(),
        "https://hf.co/owner/model".to_string(),
        "git+https://github.com/owner/repo".to_string(),
        "git+https://github.com/owner/repo@main".to_string(),
        format!("git+https://github.com/owner/repo#egg=x@{SHA}"),
    ] {
        assert_eq!(code_pin(&link), Pin::Unpinned, "{link}");
        let text = format!("---\ncode: {link}\n---\n");
        assert_eq!(
            analyze(&text)
                .refusals
                .iter()
                .filter(|refusal| matches!(refusal, Refusal::UnpinnedCode { .. }))
                .count(),
            1,
            "{link}"
        );
    }
    for link in [
        format!("https://www.github.com/owner/repo/tree/{SHA}"),
        format!("https://github.com./owner/repo/commit/{SHA}"),
        format!("https://hf.co/owner/model/tree/{SHA}"),
        format!("git+https://github.com/owner/repo@{SHA}"),
        format!("git+https://github.com/owner/repo@{SHA}#egg=x"),
    ] {
        assert_eq!(code_pin(&link), Pin::HostNotListed, "{link}");
        assert_eq!(github_commit(&link), None, "{link}");
    }
}

/// A host that serves a repository's files under another name, as GitHub's download
/// hosts do, isn't a listed host for a `code` link (design.md, Submission and
/// screening, step 2), so a link there draws the unlisted host's flag.
#[test]
fn a_download_host_is_not_a_listed_host() {
    for link in [
        "https://codeload.github.com/owner/repo/zip/refs/heads/main",
        "https://raw.githubusercontent.com/owner/repo/main/x.py",
        "https://gist.github.com/owner/0123",
    ] {
        assert_eq!(code_pin(link), Pin::HostNotListed, "{link}");
    }
}

/// An entry stored before a pin rule may hold a link the rule now refuses; its card
/// says the pin isn't verified, since no check ran on it.
#[test]
fn the_card_flags_an_unpinned_link_as_not_verified() {
    let text = "---\ncode: https://www.github.com/owner/repo\n---\n";
    let card = render_card(&analyze(text), &CardContext::default());
    assert!(card.contains("; pin not verified"), "{card}");
}
