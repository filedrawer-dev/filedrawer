//! Seam: `analyze`, for the front matter, required sections and refusals
//! (docs/entry-format.md, Front matter; design.md, Submission and screening).

mod common;

use analyzer::{CardContext, Choice, Outcome, Refusal, Role, WarningKind, analyze, render_card};
use common::{EXAMPLE, entry, example_with};

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
        Choice::Unrecognised("null-ish".into())
    );
    assert_eq!(
        warning_kinds(&example_with("outcome: no_effect", "outcome: null-ish")),
        vec![WarningKind::UnrecognisedValue]
    );
}

#[test]
fn unknown_values_in_nested_fields_are_warned() {
    let text = example_with("checks: pipeline", "checks: judge");
    assert_eq!(warning_kinds(&text), vec![WarningKind::UnrecognisedValue]);
    let text = example_with("role: subject", "role: seller");
    assert_eq!(warning_kinds(&text), vec![WarningKind::UnrecognisedValue]);
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
    assert_eq!(analysis.fields.replicates.as_deref(), Some("2507.01410"));
    assert_eq!(warning_kinds(&text), vec![WarningKind::NumberForPaper]);
    assert_eq!(
        warning_kinds(&entry("replicates: arXiv:2507.01410\n", "")),
        vec![]
    );
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
}

#[test]
fn label_and_fd_fields_are_stripped_with_a_warning() {
    let text = entry("label: supported null\nfd_status: public\n", "");
    let analysis = analyze(&text);
    assert_eq!(analysis.unrecognised_fields, Vec::<String>::new());
    assert_eq!(
        warning_kinds(&text),
        vec![WarningKind::StrippedField, WarningKind::StrippedField]
    );
}

#[test]
fn unknown_fields_are_listed_and_warned() {
    let text = entry("compute: 3 GPU hours\n", "");
    assert_eq!(analyze(&text).unrecognised_fields, vec!["compute"]);
    assert_eq!(warning_kinds(&text), vec![WarningKind::UnrecognisedField]);
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
fn stored_text_strips_label_and_fd_fields_and_keeps_their_neighbours() {
    let text = "---\ntitle: T\nlabel: supported null\nfd_status: public\nfd_flags:\n  - verified\n  - safe\nhypothesis: H\n---\n\n## Setup\n\nlabel: stays in the body\n";
    assert_eq!(
        analyzer::stored_text(text),
        "---\ntitle: T\nhypothesis: H\n---\n\n## Setup\n\nlabel: stays in the body\n"
    );
    assert_eq!(analyzer::stored_text(EXAMPLE), EXAMPLE);
}

#[test]
fn stored_text_strips_quoted_keys_and_after_a_byte_order_mark() {
    let text = "\u{feff}---\r\ntitle: T\r\n\"fd_status\": verified\r\n'label': x\r\nhypothesis: H\r\n---\r\nbody\r\n";
    assert_eq!(
        analyzer::stored_text(text),
        "\u{feff}---\r\ntitle: T\r\nhypothesis: H\r\n---\r\nbody\r\n"
    );
}

#[test]
fn an_embargo_that_isnt_a_date_is_refused() {
    for value in [
        "[2026-12-20]",
        "\n  date: 2026-12-20",
        "next spring",
        "2026-13-40",
        "2026-1-5",
    ] {
        let analysis = analyze(&entry(&format!("embargo_until: {value}\n"), ""));
        assert!(
            analysis
                .refusals
                .iter()
                .any(|r| matches!(r, Refusal::EmbargoUnreadable(_))),
            "{value:?}: {:?}",
            analysis.refusals
        );
    }
    let fine = analyze(&entry("embargo_until: 2026-12-20\n", ""));
    assert_eq!(fine.refusals, vec![]);
    assert_eq!(fine.fields.embargo_until.as_deref(), Some("2026-12-20"));
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
        let stored = analyzer::stored_text(&text);
        let analysis = analyze(&stored);
        proptest::prop_assert!(
            !analysis.warnings.iter().any(|w| w.kind == WarningKind::StrippedField),
            "{}", stored
        );
        let mut unknown = analysis.unrecognised_fields.clone();
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
    let found: Vec<String> = analyzer::slug_references(text).into_iter().collect();
    assert_eq!(found, vec!["main-run", "pilot_2", "x.y"]);
    assert!(analyzer::is_valid_slug("a.b-c_1"));
    assert!(!analyzer::is_valid_slug("x\nstatus: public"));
    assert!(!analyzer::is_valid_slug(&"a".repeat(65)));
}

#[test]
fn attachments_names_the_attachment_set_an_entry_cites() {
    let analysis = analyze(&example_with(
        "tags: [steering, refusal, transfer]",
        "tags: [steering, refusal, transfer]\nattachments: as-0123abcd",
    ));
    assert_eq!(analysis.fields.attachments.as_deref(), Some("as-0123abcd"));
    assert_eq!(analysis.unrecognised_fields, Vec::<String>::new());
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
