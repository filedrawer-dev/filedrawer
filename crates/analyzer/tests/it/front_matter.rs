//! Seam: `analyze`, for the front matter, required sections and refusals
//! (docs/entry-format.md, Front matter; design.md, Submission and screening).

use crate::common;

use common::{EXAMPLE, entry, example_with};
use filedrawer_analyzer::{
    CardContext, Choice, Outcome, Refusal, Role, WarningKind, analyze, render_card,
};

fn warning_kinds(text: &str) -> Vec<WarningKind> {
    analyze(text).warnings.iter().map(|w| w.kind).collect()
}

#[test]
fn the_spec_example_is_accepted_and_read() {
    let analysis = analyze(EXAMPLE);
    assert_eq!(analysis.refusals, vec![]);
    assert_eq!(analysis.warnings, vec![]);
    let fields = &analysis.fields;
    assert_eq!(
        fields.title.as_deref(),
        Some(
            "Refusal steering vector from Llama-3-8B base does not transfer to the instruct model"
        )
    );
    assert_eq!(fields.outcome, Choice::Known(Outcome::NoEffect));
    assert_eq!(fields.models.len(), 1);
    assert_eq!(
        fields.models[0].name.as_deref(),
        Some("Llama-3-8B-Instruct")
    );
    assert_eq!(fields.models[0].role, Choice::Known(Role::Subject));
    assert_eq!(fields.tags, vec!["steering", "refusal", "transfer"]);
    assert_eq!(fields.run_dates, vec!["2026-10-20", "2026-10-20"]);
    assert_eq!(
        fields.written_by[0].model.as_deref(),
        Some("claude-opus-5-5")
    );
    assert!(
        analysis
            .sections
            .not_tried
            .as_deref()
            .unwrap()
            .contains("Layers other than 14.")
    );
}

#[test]
fn each_required_field_refuses_when_missing() {
    for field in ["title", "safety_motivation", "hypothesis", "outcome"] {
        let line = EXAMPLE
            .lines()
            .find(|line| line.starts_with(&format!("{field}:")))
            .unwrap();
        let analysis = analyze(&example_with(&format!("{line}\n"), ""));
        assert_eq!(
            analysis.refusals,
            vec![Refusal::MissingField(field)],
            "{field}"
        );
    }
}

#[test]
fn outcome_null_counts_as_missing() {
    let analysis = analyze(&example_with("outcome: no_effect", "outcome: null"));
    assert_eq!(analysis.refusals, vec![Refusal::MissingField("outcome")]);
    let message = analysis.refusals[0].to_string();
    assert!(message.contains("write `no_effect`"), "{message}");
}

#[test]
fn an_empty_required_field_counts_as_missing() {
    let analysis = analyze(&example_with(
        "hypothesis: Adding",
        "hypothesis: \"\"\nx: Adding",
    ));
    assert!(
        analysis
            .refusals
            .contains(&Refusal::MissingField("hypothesis"))
    );
}

#[test]
fn each_required_section_refuses_when_missing() {
    let no_setup = example_with("## Setup", "## Set up");
    assert_eq!(
        analyze(&no_setup).refusals,
        vec![Refusal::MissingSection("Setup")]
    );
    let no_reproduce = example_with("## Reproduce", "## How to rerun");
    assert_eq!(
        analyze(&no_reproduce).refusals,
        vec![Refusal::MissingSection("Reproduce")]
    );
}

#[test]
fn an_entry_without_front_matter_is_refused() {
    let analysis = analyze("# Just markdown\n\n## Setup\n\nx\n\n## Reproduce\n\ny\n");
    assert!(matches!(
        analysis.refusals.as_slice(),
        [Refusal::FrontMatterUnreadable(_)]
    ));
}

#[test]
fn front_matter_that_isnt_yaml_is_refused() {
    let analysis = analyze(&example_with("stage: main", "stage: [main"));
    assert!(matches!(
        analysis.refusals.as_slice(),
        [Refusal::FrontMatterUnreadable(_)]
    ));
}

#[test]
fn front_matter_with_windows_line_endings_is_read() {
    let analysis = analyze(&EXAMPLE.replace('\n', "\r\n"));
    assert_eq!(analysis.refusals, vec![]);
    assert_eq!(analysis.fields.outcome, Choice::Known(Outcome::NoEffect));
}

#[test]
fn an_entry_over_100_kib_is_refused() {
    let body = format!("## Results\n\n{}", "x".repeat(100 * 1024));
    let analysis = analyze(&entry("", &body));
    assert!(matches!(
        analysis.refusals.as_slice(),
        [Refusal::TooLarge { .. }]
    ));
    let small = analyze(&entry("", "## Results\n\nnone"));
    assert_eq!(small.refusals, vec![]);
}

#[test]
fn more_than_ten_logs_are_refused() {
    let urls: Vec<String> = (0..11)
        .map(|i| format!("https://wandb.ai/a/b/runs/{i}"))
        .collect();
    let analysis = analyze(&entry(&format!("logs: [{}]\n", urls.join(", ")), ""));
    assert_eq!(analysis.refusals, vec![Refusal::TooManyLogs { count: 11 }]);
    let ten = format!("logs: [{}]\n", urls[..10].join(", "));
    assert_eq!(analyze(&entry(&ten, "")).refusals, vec![]);
}

#[test]
fn an_unknown_outcome_is_kept_and_warned_not_refused() {
    let analysis = analyze(&example_with("outcome: no_effect", "outcome: null-ish"));
    assert_eq!(analysis.refusals, vec![]);
    assert_eq!(
        analysis.fields.outcome,
        Choice::Unrecognized("null-ish".into())
    );
    assert_eq!(
        warning_kinds(&example_with("outcome: no_effect", "outcome: null-ish")),
        vec![WarningKind::UnrecognizedValue]
    );
}

#[test]
fn unknown_values_in_nested_fields_are_warned() {
    let text = example_with("checks: pipeline", "checks: judge");
    assert_eq!(warning_kinds(&text), vec![WarningKind::UnrecognizedValue]);
    let text = example_with("role: subject", "role: seller");
    assert_eq!(warning_kinds(&text), vec![WarningKind::UnrecognizedValue]);
}

#[test]
fn a_flow_mapping_is_warned() {
    let text = example_with(
        "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\n",
        "positive_control: {outcome: passed, checks: pipeline, source: this_run}\n",
    );
    assert_eq!(warning_kinds(&text), vec![WarningKind::FlowMapping]);
}

#[test]
fn a_bare_arxiv_number_is_warned_and_kept_as_written() {
    let text = entry("replicates: 2507.01410\n", "");
    let analysis = analyze(&text);
    assert_eq!(analysis.fields.replicates, vec!["2507.01410"]);
    assert_eq!(warning_kinds(&text), vec![WarningKind::NumberForPaper]);
    assert_eq!(
        warning_kinds(&entry("replicates: arXiv:2507.01410\n", "")),
        vec![]
    );
}

#[test]
fn replicates_and_reanalyzes_take_one_item_or_a_list() {
    let text = entry(
        "replicates: slug:pilot\nreanalyzes:\n  - arXiv:2609.18346\n  - 10.2139/ssrn.7439840\n  - https://www.lesswrong.com/posts/abc/title\n",
        "",
    );
    let analysis = analyze(&text);
    assert_eq!(analysis.fields.replicates, vec!["slug:pilot"]);
    assert_eq!(
        analysis.fields.reanalyzes,
        vec![
            "arXiv:2609.18346",
            "10.2139/ssrn.7439840",
            "https://www.lesswrong.com/posts/abc/title"
        ]
    );
    assert_eq!(warning_kinds(&text), vec![]);
    let flow = analyze(&entry("reanalyzes: [arXiv:2609.18346, bcz2bqrf3b]\n", ""));
    assert_eq!(
        flow.fields.reanalyzes,
        vec!["arXiv:2609.18346", "bcz2bqrf3b"]
    );
}

#[test]
fn an_item_in_no_form_the_format_takes_is_kept_and_warned() {
    let text = entry(
        "replicates:\n  - Yun (2026)\n  - arXiv:2507.01413, arXiv:2603.20281\n",
        "",
    );
    let analysis = analyze(&text);
    assert_eq!(
        analysis.fields.replicates,
        vec!["Yun (2026)", "arXiv:2507.01413, arXiv:2603.20281"]
    );
    let warnings: Vec<_> = analysis
        .warnings
        .iter()
        .filter(|warning| warning.kind == WarningKind::NotAReference)
        .collect();
    assert_eq!(warnings.len(), 2, "{:?}", analysis.warnings);
    assert!(
        warnings[0].message.contains("`Yun (2026)`"),
        "{}",
        warnings[0]
    );
    assert!(warnings[1].message.contains("own line"), "{}", warnings[1]);
}

#[test]
fn a_mapping_item_is_skipped_with_a_warning_naming_the_forms() {
    // A space after the colon makes YAML read a mapping.
    let text = entry(
        "reanalyzes:\n  - arXiv: 2507.01413\n  - arXiv:2603.20281\n",
        "",
    );
    let analysis = analyze(&text);
    assert_eq!(analysis.fields.reanalyzes, vec!["arXiv:2603.20281"]);
    let malformed: Vec<_> = analysis
        .warnings
        .iter()
        .filter(|warning| warning.kind == WarningKind::MalformedField)
        .collect();
    assert_eq!(malformed.len(), 1, "{:?}", analysis.warnings);
    assert!(
        malformed[0].message.contains("arXiv:<id>"),
        "{}",
        malformed[0]
    );
    assert_eq!(
        warning_kinds(&entry("replicates:\n  id: arXiv:2507.01413\n", "")),
        vec![WarningKind::MalformedField]
    );
}

#[test]
fn past_ten_items_the_rest_are_dropped_with_a_warning() {
    let items: String = (10..20)
        .map(|n| format!("  - arXiv:2507.000{n}\n"))
        .collect();
    // A dropped item isn't read, so it isn't told it was kept.
    let text = entry(
        &format!("reanalyzes:\n{items}  - Yun (2026)\n  - arXiv:2507.00021\n"),
        "",
    );
    let analysis = analyze(&text);
    assert_eq!(analysis.fields.reanalyzes.len(), 10);
    assert_eq!(analysis.fields.reanalyzes[9], "arXiv:2507.00019");
    assert_eq!(warning_kinds(&text), vec![WarningKind::TooManyReferences]);
}

#[test]
fn published_in_must_be_a_doi_or_arxiv_id() {
    assert_eq!(
        warning_kinds(&entry("published_in: arXiv:2507.01410\n", "")),
        vec![]
    );
    assert_eq!(
        warning_kinds(&entry("published_in: 10.1234/abc.5\n", "")),
        vec![]
    );
    assert_eq!(
        warning_kinds(&entry("published_in: https://example.com/paper\n", "")),
        vec![WarningKind::NotAPaper]
    );
    // Text after the ID makes it no ID: the card shows the value beside the
    // server's own flags, so it must be one the server reads.
    assert_eq!(
        warning_kinds(&entry(
            "published_in: \"arXiv:2507.01410; code attached\"\n",
            ""
        )),
        vec![WarningKind::NotAPaper]
    );
    // A paper's page reads as the paper, and so does a space after `arXiv:`.
    for read in ["https://arxiv.org/abs/2507.01410", "\"arXiv: 2507.01410\""] {
        assert_eq!(
            warning_kinds(&entry(&format!("published_in: {read}\n"), "")),
            vec![],
            "{read}"
        );
    }
    // A bare number is warned about once, as a number.
    assert_eq!(
        warning_kinds(&entry("published_in: 2507.01410\n", "")),
        vec![WarningKind::NumberForPaper]
    );
}

#[test]
fn label_and_fd_fields_are_stripped_with_a_warning() {
    let text = entry("label: supported null\nfd_status: public\n", "");
    let analysis = analyze(&text);
    assert_eq!(analysis.unrecognized_fields, Vec::<String>::new());
    assert_eq!(
        warning_kinds(&text),
        vec![WarningKind::StrippedField, WarningKind::StrippedField]
    );
}

#[test]
fn unknown_fields_are_listed_and_warned() {
    let text = entry("compute: 3 GPU hours\n", "");
    assert_eq!(analyze(&text).unrecognized_fields, vec!["compute"]);
    assert_eq!(warning_kinds(&text), vec![WarningKind::UnrecognizedField]);
}

#[test]
fn a_github_code_link_without_a_commit_hash_is_refused() {
    for url in [
        "https://github.com/a/b",
        "https://github.com/a/b/tree/main",
        "https://github.com/a/b/tree/v1.0",
        "https://huggingface.co/a/b/tree/main",
    ] {
        let analysis = analyze(&entry(&format!("code: {url}\n"), ""));
        assert_eq!(
            analysis.refusals,
            vec![Refusal::UnpinnedCode { url: url.into() }],
            "{url}"
        );
    }
}

#[test]
fn a_code_link_with_a_commit_hash_passes() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    for url in [
        format!("https://github.com/a/b/tree/{hash}"),
        format!("https://github.com/a/b/blob/{hash}/src/x.py"),
        format!("https://github.com/a/b/commit/{hash}"),
        format!("https://huggingface.co/a/b/tree/{hash}"),
    ] {
        let text = entry(&format!("code: {url}\n"), "");
        let analysis = analyze(&text);
        assert_eq!(analysis.fields.code.as_deref(), Some(url.as_str()));
        assert_eq!(analysis.refusals, vec![], "{url}");
        assert_eq!(warning_kinds(&text), vec![], "{url}");
    }
}

#[test]
fn a_github_lookalike_host_is_not_listed() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let text = entry(
        &format!("code: https://github.com.evil.example/a/b/tree/{hash}\n"),
        "",
    );
    assert_eq!(analyze(&text).refusals, vec![]);
    assert_eq!(warning_kinds(&text), vec![WarningKind::HostNotListed]);
}

#[test]
fn a_host_is_read_as_a_browser_reads_it() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    // A browser opens evil.example: a backslash ends the host.
    let text = entry(
        &format!("code: https://evil.example\\@github.com/a/b/tree/{hash}\n"),
        "",
    );
    assert_eq!(warning_kinds(&text), vec![WarningKind::HostNotListed]);
    assert!(render_card(&analyze(&text), &CardContext::default()).contains("pin not verified"));
    // A scheme or host in capitals is still github.com.
    let text = entry(&format!("code: HTTPS://GitHub.com/a/b/tree/{hash}\n"), "");
    assert_eq!(warning_kinds(&text), vec![]);
    assert_eq!(analyze(&text).refusals, vec![]);
}

#[test]
fn a_zenodo_code_link_is_accepted_with_a_note_that_its_pin_is_checked_online() {
    let text = entry("code: https://zenodo.org/records/1234567\n", "");
    assert_eq!(analyze(&text).refusals, vec![]);
    assert_eq!(warning_kinds(&text), vec![WarningKind::PinNotChecked]);
}

#[test]
fn wandb_links_are_warned_since_the_server_cant_tell_a_dead_one() {
    // wandb.ai sends every link to forge.coreweave.com, which answers the same page
    // for a run that doesn't exist.
    let text = entry("logs: [https://wandb.ai/a/b/runs/abc123]\n", "");
    assert_eq!(warning_kinds(&text), vec![WarningKind::HostNotListed]);
}

#[test]
fn logs_on_an_unlisted_host_are_warned() {
    let text = entry("logs: [https://example.com/run.log]\n", "");
    assert_eq!(warning_kinds(&text), vec![WarningKind::HostNotListed]);
}

#[test]
fn reproduce_fetching_code_at_a_moving_ref_is_refused() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let moving = "pip install git+https://github.com/a/b@main";
    let text = example_with(
        "`python steer.py",
        &format!("`{moving}`\n\n`python steer.py"),
    );
    assert_eq!(
        analyze(&text).refusals,
        vec![Refusal::MovingRef {
            command: moving.into()
        }]
    );

    let unpinned_clone = "pip install git+https://github.com/a/b";
    let text = example_with(
        "`python steer.py",
        &format!("`{unpinned_clone}`\n\n`python steer.py"),
    );
    assert_eq!(analyze(&text).refusals.len(), 1);

    let pinned = format!("pip install git+https://github.com/a/b@{hash}");
    let text = example_with(
        "`python steer.py",
        &format!("`{pinned}`\n\n`python steer.py"),
    );
    assert_eq!(analyze(&text).refusals, vec![]);
}

/// A hash in a command's query or fragment selects nothing: the command still
/// fetches the branch.
#[test]
fn reproduce_fetching_a_moving_ref_with_a_hash_beside_it_is_refused() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    for moving in [
        format!("pip install git+https://github.com/a/b#egg=b@{hash}"),
        format!("curl -LO https://github.com/a/b/archive/main.zip?x/{hash}"),
        format!("curl -LO https://github.com/a/b/archive/main.zip#/{hash}"),
        // A branch named `x/<hash>`.
        format!("curl -LO https://github.com/a/b/archive/refs/heads/x/{hash}.zip"),
        format!("curl -O https://raw.githubusercontent.com/a/b/main/steer.py?v={hash}"),
        // GitHub's sites spelled another way.
        "curl -LO https://www.github.com/a/b/archive/main.zip".to_string(),
        "curl -LO https://github.com./a/b/archive/main.zip".to_string(),
        "curl -O https://raw.githubusercontent.com./a/b/main/steer.py".to_string(),
        "curl -LO https://codeload.github.com./a/b/zip/main".to_string(),
    ] {
        let text = example_with(
            "`python steer.py",
            &format!("`{moving}`\n\n`python steer.py"),
        );
        assert_eq!(
            analyze(&text).refusals,
            vec![Refusal::MovingRef {
                command: moving.clone()
            }],
            "{moving}"
        );
    }
    for pinned in [
        format!("pip install git+https://github.com/a/b@{hash}#egg=b"),
        format!("curl -LO https://github.com/a/b/archive/{hash}.zip?download=1"),
        format!("curl -O https://raw.githubusercontent.com/a/b/{hash}/steer.py?raw=1"),
    ] {
        let text = example_with(
            "`python steer.py",
            &format!("`{pinned}`\n\n`python steer.py"),
        );
        assert_eq!(analyze(&text).refusals, vec![], "{pinned}");
    }
}

#[test]
fn yaml_aliases_are_refused_before_they_expand() {
    let mut bomb = String::from("a0: &a0 [x,x,x,x,x,x,x,x,x,x]\n");
    for level in 1..9 {
        let previous = format!("*a{}", level - 1);
        bomb.push_str(&format!(
            "a{level}: &a{level} [{}]\n",
            vec![previous; 10].join(",")
        ));
    }
    let analysis = analyze(&entry(&bomb, ""));
    let [Refusal::FrontMatterUnreadable(why)] = analysis.refusals.as_slice() else {
        panic!("{:?}", analysis.refusals)
    };
    assert!(why.contains("alias"), "{why}");
}

#[test]
fn a_repo_named_like_a_commit_hash_is_not_a_pin() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    for url in [
        format!("https://github.com/a/{hash}"),
        format!("https://github.com/{hash}/b/tree/main"),
        format!("https://huggingface.co/datasets/a/{hash}"),
    ] {
        let analysis = analyze(&entry(&format!("code: {url}\n"), ""));
        assert_eq!(
            analysis.refusals,
            vec![Refusal::UnpinnedCode { url: url.clone() }],
            "{url}"
        );
    }
    let dataset = format!("https://huggingface.co/datasets/a/b/tree/{hash}");
    assert_eq!(
        analyze(&entry(&format!("code: {dataset}\n"), "")).refusals,
        vec![]
    );
}

#[test]
fn reproduce_prose_and_data_files_at_a_branch_are_not_refused() {
    let prose = "Data file: https://raw.githubusercontent.com/u/r/main/data.csv (a CSV, not code).";
    let text = example_with("`python steer.py", &format!("{prose}\n\n`python steer.py"));
    assert_eq!(analyze(&text).refusals, vec![]);
    let data = "`curl -O https://raw.githubusercontent.com/u/r/main/data.csv`";
    let text = example_with("`python steer.py", &format!("{data}\n\n`python steer.py"));
    assert_eq!(analyze(&text).refusals, vec![]);
}

#[test]
fn reproduce_code_that_fetches_a_script_at_a_branch_is_refused() {
    let block = "```\ncurl -s https://raw.githubusercontent.com/u/r/main/run.py | python\n```";
    let text = example_with("`python steer.py", &format!("{block}\n\n`python steer.py"));
    assert_eq!(analyze(&text).refusals.len(), 1);
}

#[test]
fn stored_text_strips_label_and_fd_fields_and_keeps_their_neighbors() {
    let text = "---\ntitle: T\nlabel: supported null\nfd_status: public\nfd_flags:\n  - verified\n  - safe\nhypothesis: H\n---\n\n## Setup\n\nlabel: stays in the body\n";
    assert_eq!(
        filedrawer_analyzer::stored_text(text),
        "---\ntitle: T\nhypothesis: H\n---\n\n## Setup\n\nlabel: stays in the body\n"
    );
    assert_eq!(filedrawer_analyzer::stored_text(EXAMPLE), EXAMPLE);
}

#[test]
fn stored_text_strips_quoted_keys_and_after_a_byte_order_mark() {
    let text = "\u{feff}---\r\ntitle: T\r\n\"fd_status\": verified\r\n'label': x\r\nhypothesis: H\r\n---\r\nbody\r\n";
    assert_eq!(
        filedrawer_analyzer::stored_text(text),
        "\u{feff}---\r\ntitle: T\r\nhypothesis: H\r\n---\r\nbody\r\n"
    );
}

#[test]
fn any_field_asking_for_an_embargo_is_refused_as_retired() {
    // design.md, Rejected (embargoes, retired 2026-11-17): an author who writes one
    // meant to hold the entry, so the server refuses it rather than publish it,
    // whatever the value, an empty one included.
    for (field, value) in [
        ("embargo_until", " 2026-12-20"),
        ("embargo_until", ""),
        ("embargo_until", " null"),
        ("embargo_until", " next spring"),
        ("embargo_until", " [2026-12-20]"),
        ("embargo", " 2026-12-20"),
        ("embargo_date", " 2026-12-20"),
        ("Embargoed_until", " 2026-12-20"),
    ] {
        let analysis = analyze(&entry(&format!("{field}:{value}\n"), ""));
        assert_eq!(
            analysis.refusals,
            vec![Refusal::EmbargoRetired {
                field: field.into()
            }],
            "{field}:{value}"
        );
    }
}

#[test]
fn the_embargo_refusal_says_to_remove_the_field_and_submit_when_ready() {
    let analysis = analyze(&entry("embargo_until: 2026-12-20\n", ""));
    let reason = analysis.refusals[0].to_string();
    assert!(reason.contains("no longer"), "{reason}");
    assert!(reason.contains("Remove `embargo_until`"), "{reason}");
    assert!(!reason.contains("paused"), "{reason}");
}

#[test]
fn warnings_are_one_line_whatever_the_author_wrote() {
    let analysis = analyze(&entry("\"x\\nstatus: public\\nid: forged\": 1\n", ""));
    assert!(!analysis.warnings.is_empty());
    for warning in &analysis.warnings {
        assert!(!warning.to_string().contains('\n'), "{warning}");
    }
}

proptest::proptest! {
    #[test]
    fn stored_text_removes_every_reserved_field_and_keeps_the_rest(
        keys in proptest::collection::vec("(label|fd_[a-z]{0,6}|[a-z]{1,8})", 1..8),
        quote in 0usize..3,
        nested in proptest::bool::ANY,
    ) {
        let mut front = String::new();
        let mut kept = Vec::new();
        let mut seen_label = false;
        for (index, key) in keys.iter().enumerate() {
            // Keys stay unique, since YAML refuses duplicates; `label` appears once.
            let key = if key == "label" { key.clone() } else { format!("{key}{index}") };
            if key == "label" {
                if seen_label { continue; }
                seen_label = true;
            }
            let written = match quote { 0 => key.clone(), 1 => format!("\"{key}\""), _ => format!("'{key}'") };
            if nested {
                front.push_str(&format!("{written}:\n  - a\n  - b: c\n"));
            } else {
                front.push_str(&format!("{written}: value\n"));
            }
            if !(key == "label" || key.starts_with("fd_")) {
                kept.push(key);
            }
        }
        let text = entry(&front, "");
        let stored = filedrawer_analyzer::stored_text(&text);
        let analysis = analyze(&stored);
        proptest::prop_assert!(
            !analysis.warnings.iter().any(|w| w.kind == WarningKind::StrippedField),
            "{}", stored
        );
        let mut unknown = analysis.unrecognized_fields.clone();
        unknown.sort();
        kept.sort();
        let missing: Vec<&String> = kept.iter().filter(|k| !unknown.contains(k)).collect();
        proptest::prop_assert!(missing.is_empty(), "lost {:?} in\n{}", missing, stored);
        proptest::prop_assert_eq!(analysis.refusals.clone(), vec![]);
    }
}

#[test]
fn slug_references_are_found_in_fields_and_body_and_end_at_punctuation() {
    let text = "replicates: slug:main-run\n\nSee slug:pilot_2. And slug:x.y, then slug: alone.";
    let found: Vec<String> = filedrawer_analyzer::slug_references(text)
        .into_iter()
        .collect();
    assert_eq!(found, vec!["main-run", "pilot_2", "x.y"]);
    assert!(filedrawer_analyzer::is_valid_slug("a.b-c_1"));
    assert!(!filedrawer_analyzer::is_valid_slug("x\nstatus: public"));
    assert!(!filedrawer_analyzer::is_valid_slug(&"a".repeat(65)));
}

#[test]
fn attachments_names_the_attachment_set_an_entry_cites() {
    let analysis = analyze(&example_with(
        "tags: [steering, refusal, transfer]",
        "tags: [steering, refusal, transfer]\nattachments: as-0123abcd",
    ));
    assert_eq!(analysis.fields.attachments.as_deref(), Some("as-0123abcd"));
    assert_eq!(analysis.unrecognized_fields, Vec::<String>::new());
    assert_eq!(analyze(EXAMPLE).fields.attachments, None);
}

#[test]
fn a_series_cite_the_server_cant_read_is_refused() {
    // entry-format.md, Fields: `series` is `<id>@<version>` or
    // `<id>@<version>#<section>`, with a version from 1. The field test's agents made
    // up IDs such as `ser_madeUp1`, which the server refuses.
    for bad in ["ser_madeUp1@1", "s-abc", "s-abc@0"] {
        let text = example_with("tags:", &format!("series: {bad}\ntags:"));
        assert_eq!(
            analyze(&text).refusals,
            vec![Refusal::SeriesCite(bad.to_string())],
            "{bad}"
        );
    }
    let text = example_with("tags:", "series: ser_madeUp1@1\ntags:");
    assert_eq!(
        analyze(&text).refusals[0].to_string(),
        "`series: ser_madeUp1@1` should be `<id>@<version>` or `<id>@<version>#<section>`, with a version from 1"
    );
    for good in ["s-34d1twxyg1@1", "s-34d1twxyg1@2#setup"] {
        let text = example_with("tags:", &format!("series: {good}\ntags:"));
        assert_eq!(analyze(&text).refusals, vec![], "{good}");
    }
}

/// A required field must show something (entry-format.md, Required): text of only
/// spaces, control characters or invisible characters is no text.
#[test]
fn a_required_field_that_shows_nothing_is_missing() {
    for nothing in [
        "\\u202E",
        "\\x01",
        "\\x7F \\u0085",
        "\u{200b}\u{2060}",
        "\u{3164}",
        "\u{fe0f}",
        "\u{e0020}\u{e0041}",
        "\u{2800}",
    ] {
        for (field, name) in [
            ("title: T", "title"),
            ("hypothesis: H", "hypothesis"),
            ("safety_motivation: S", "safety_motivation"),
        ] {
            let text = entry("", "").replace(field, &format!("{name}: \"{nothing}\""));
            assert_eq!(
                analyze(&text).refusals,
                vec![Refusal::MissingField(name)],
                "{name}: {nothing:?}"
            );
        }
    }
    // Invisible characters beside text that shows are no reason to refuse.
    let text = entry("", "").replace("title: T", "title: \"T\u{200b}\"");
    assert_eq!(analyze(&text).refusals, vec![]);
}

/// A tag is a short label on one line (entry-format.md, Fields): one that isn't is
/// skipped with a warning that doesn't repeat it.
#[test]
fn a_tag_that_is_no_short_label_is_skipped_with_a_warning() {
    let long = "x".repeat(61);
    let text = entry(
        &format!(
            "tags: [steering, \"x 1\\nsteering 99999\", \"  refusal \", \"hid\u{e0064}den\", \"zero\u{200b}width\", \"sep\\u2028arator\", {long}, {}]\n",
            "y".repeat(60)
        ),
        "",
    );
    let analysis = analyze(&text);
    assert_eq!(analysis.refusals, vec![]);
    assert_eq!(
        analysis.fields.tags,
        vec![
            "steering".to_string(),
            "refusal".to_string(),
            "y".repeat(60)
        ]
    );
    let messages: Vec<&str> = analysis
        .warnings
        .iter()
        .map(|warning| warning.message.as_str())
        .collect();
    let skipped = |position: usize| {
        format!(
            "tag {position} in `tags` was not read: a tag is at most 60 characters on one line, with no invisible characters"
        )
    };
    assert_eq!(
        messages,
        vec![skipped(2), skipped(4), skipped(5), skipped(6), skipped(7)]
    );
    assert_eq!(warning_kinds(&text), vec![WarningKind::MalformedField; 5]);
}

/// A warning names a tag by its place in the list as written, and a tag that
/// shows nothing is skipped like any other that is no label; an empty one is no tag.
#[test]
fn a_skipped_tag_is_named_by_its_place_as_written() {
    let text = entry(
        "tags: [steering, \"\", \"\u{200b}\", [nested], \"a\\nb\", refusal]\n",
        "",
    );
    let analysis = analyze(&text);
    assert_eq!(analysis.fields.tags, vec!["steering", "refusal"]);
    let messages: Vec<&str> = analysis
        .warnings
        .iter()
        .map(|warning| warning.message.as_str())
        .collect();
    assert_eq!(messages.len(), 3, "{messages:?}");
    for (message, position) in messages.iter().zip([3, 4, 5]) {
        assert!(
            message.starts_with(&format!("tag {position} in `tags` was not read")),
            "{message}"
        );
    }
}

#[test]
fn tags_past_the_first_20_are_dropped_with_a_warning() {
    let tags: Vec<String> = (1..=21).map(|n| format!("t{n}")).collect();
    let text = entry(&format!("tags: [{}]\n", tags.join(", ")), "");
    let analysis = analyze(&text);
    assert_eq!(analysis.refusals, vec![]);
    assert_eq!(analysis.fields.tags, tags[..20]);
    assert_eq!(
        analysis.warnings[0].message,
        "`tags` lists 21 tags; only the first 20 were read"
    );
    assert_eq!(warning_kinds(&text), vec![WarningKind::MalformedField]);
    let twenty = entry(&format!("tags: [{}]\n", tags[..20].join(", ")), "");
    assert_eq!(analyze(&twenty).warnings, vec![]);
}

#[test]
fn a_valid_tag_is_a_short_label_on_one_line() {
    use filedrawer_analyzer::is_valid_tag;
    for tag in [
        "steering",
        "sparse autoencoders",
        "GPT-4o",
        "\u{6a21}\u{578b}",
        &"x".repeat(60),
        // Persian writes a zero-width non-joiner inside words.
        "\u{645}\u{6cc}\u{200c}\u{646}\u{627}",
        // Devanagari: a half form, written with a zero-width joiner after the virama.
        "\u{915}\u{94d}\u{200d}\u{937}",
        // Emoji sequences: joined emoji, a presentation selector, and both.
        "\u{1f469}\u{200d}\u{1f4bb}",
        "\u{2764}\u{fe0f}",
        "\u{1f441}\u{fe0f}\u{200d}\u{1f5e8}\u{fe0f}",
        "1\u{fe0f}\u{20e3}",
        "#\u{fe0f}\u{20e3}",
        "*\u{fe0f}\u{20e3}",
    ] {
        assert!(is_valid_tag(tag), "{tag:?}");
    }
    for tag in [
        "",
        " ",
        "a\nb",
        "a\tb",
        "a\u{85}b",
        "a\u{2028}b",
        "a\u{2029}b",
        "a\u{200b}b",
        "a\u{202e}b",
        "a\u{e0041}b",
        "\u{3164}",
        &"x".repeat(61),
        // A joiner joins only visible characters, and no two ASCII ones.
        "a\u{200d}b",
        "a\u{200c}b",
        "\u{200c}\u{645}\u{6cc}",
        "\u{645}\u{6cc}\u{200c}",
        "\u{645}\u{6cc}\u{200c}\u{200c}\u{646}\u{627}",
        "\u{645}\u{6cc}\u{200c} \u{646}\u{627}",
        "\u{200d}",
        // A presentation selector follows a visible character, one at a time; the
        // other variation selectors aren't read.
        "\u{2764}\u{fe0f}\u{fe0f}",
        "\u{fe0f}a",
        "a \u{fe0f}",
        "a\u{fe00}",
        "\u{2764}\u{e0100}",
        // After an ASCII character only a keycap's base takes one.
        "a\u{fe0f}",
        "x\u{fe0e}",
        // Egyptian hieroglyph format controls show nothing on their own.
        "a\u{13430}b",
        "\u{13430}",
    ] {
        assert!(!is_valid_tag(tag), "{tag:?}");
    }
}

#[test]
fn superseded_by_must_be_an_entry_id_or_a_slug() {
    for read in [
        "abcdefghjk",
        "slug:main-run",
        "https://filedrawer.dev/e/0123456789",
    ] {
        assert_eq!(
            warning_kinds(&entry(&format!("superseded_by: {read}\n"), "")),
            vec![],
            "{read}"
        );
    }
    let text = entry("superseded_by: \"0123456789; code attached\"\n", "");
    assert_eq!(warning_kinds(&text), vec![WarningKind::UnrecognizedValue]);
    assert_eq!(
        analyze(&text).warnings[0].message,
        "`superseded_by: 0123456789; code attached` is not an entry ID or `slug:<name>`, so the card can't show it"
    );
}
