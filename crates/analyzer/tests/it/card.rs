//! Seam: `render_card` (docs/entry-format.md, The card).

use crate::common;

use common::{EXAMPLE, entry, example_with, with_results};
use filedrawer_analyzer::{
    Author, CardContext, Choice, LINE_CAPS, Outcome, PinFlag, Reference, Replication, analyze,
    render_card,
};
use proptest::prelude::*;

fn card(text: &str) -> String {
    render_card(&analyze(text), &CardContext::default())
}

/// The card's lines, each without its label (see [`unlabeled`]).
fn lines(text: &str) -> Vec<String> {
    unlabeled(&card(text))
}

/// The rest of the card line that starts with `label`, such as `"flags: "`.
fn line_starting(text: &str, label: &str) -> String {
    line_after(&card(text), label)
}

/// The rest of the line of `card` that starts with `label`.
fn line_after(card: &str, label: &str) -> String {
    card.lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_else(|| panic!("no line starts with {label:?} in\n{card}"))
        .to_string()
}

const EFFECT_TABLE: &str = "Intervals: 95%, Wilson, per prompt.\n\n| model | condition | baseline | estimate | ci_low | ci_high | n | unit |\n|---|---|---|---|---|---|---|---|\n| m | treated vs control | 0.2 | -0.15 | -0.25 | -0.05 | 200/200 | fraction |";

#[test]
fn the_spec_example_renders_the_card_the_spec_describes() {
    // Lines 4 to 6 are quoted in entry-format.md's Example, and every line starts
    // with the label The card gives it.
    let expected = "\
entry: <id>; experiment; null; main; rows: 1 null
title: Refusal steering vector from Llama-3-8B base does not transfer to the instruct model
hypothesis: Adding the base model's refusal direction at layer 14 raises the instruct model's refusal rate on harmful prompts.
result: interval includes 0; null: +0.012 [\u{2212}0.047, +0.070], n 520/520, fraction
design: pipeline check passed (this run); predicted increase, set before run
stats: baseline 0.62; 95%, Newcombe hybrid score, per prompt
models: Llama-3-8B-Instruct; tags: steering, refusal, transfer
not tried: Layers other than 14.; Steering coefficients above 8.; Other base and instruct pairs.
flags: analyzer v2
author: <author>";
    assert_eq!(card(EXAMPLE), expected);
}

/// The labels that start the card's lines, in order (entry-format.md, The card).
const LABELS: [&str; 11] = [
    "entry: ",
    "title: ",
    "hypothesis: ",
    "result: ",
    "design: ",
    "stats: ",
    "models: ",
    "first caveat: ",
    "not tried: ",
    "flags: ",
    "author: ",
];

/// Each line of `card` without its label, after checking that every line starts
/// with the server's label for its place: line 8's only on an `effect` card.
fn unlabeled(card: &str) -> Vec<String> {
    let lines: Vec<&str> = card.lines().collect();
    let labels: Vec<&str> = LABELS
        .into_iter()
        .filter(|label| lines.len() == 11 || *label != "first caveat: ")
        .collect();
    assert_eq!(lines.len(), labels.len(), "{card}");
    lines
        .iter()
        .zip(labels)
        .map(|(line, label)| {
            line.strip_prefix(label)
                .unwrap_or_else(|| panic!("{line:?} doesn't start with {label:?}"))
                .to_string()
        })
        .collect()
}

#[test]
fn the_server_context_fills_the_id_flags_and_author() {
    let text = example_with(
        "code: https://github.com/example/steering/tree/0123456789abcdef0123456789abcdef01234567\n",
        "",
    );
    let context = CardContext {
        id: Some("fd-7k2q".into()),
        author: Some(Author {
            display_name: Some("Ada".into()),
            account_id: "a91x".into(),
            handles: vec!["github:ada".into()],
        }),
        screening_note: Some("The body reports an effect.".into()),
        code_attached: true,
        code_link_dead: false,
        logs_link_dead: true,
        pin: None,
        superseded_by: Some("fd-9zz1".into()),
        replications: vec![
            Replication {
                id: "fd-r1".into(),
                outcome: Choice::Known(Outcome::NoEffect),
                self_replication: false,
            },
            Replication {
                id: "fd-r2".into(),
                outcome: Choice::Known(Outcome::Effect),
                self_replication: true,
            },
        ],
    };
    let card = render_card(&analyze(&text), &context);
    let lines: Vec<&str> = card.lines().collect();
    assert!(
        lines[0].starts_with("entry: fd-7k2q; experiment; null"),
        "{}",
        lines[0]
    );
    assert_eq!(
        lines[8],
        "flags: analyzer v2; screening note: \"The body reports an effect.\"; code attached; logs link dead; superseded by fd-9zz1; replication fd-r1 (null); self-replication fd-r2 (effect)"
    );
    // The account's ID and its linked handles, the server's words, come before the
    // name, which an author chose.
    assert_eq!(lines[9], "author: account a91x, github:ada, Ada");
}

#[test]
fn the_server_flags_a_pin_it_couldnt_verify_or_found_bad() {
    let flags =
        |context: CardContext| unlabeled(&render_card(&analyze(EXAMPLE), &context))[8].clone();
    let with = |pin| CardContext {
        pin: Some(pin),
        ..CardContext::default()
    };
    assert_eq!(
        flags(with(PinFlag::NotVerified)),
        "analyzer v2; pin not verified"
    );
    assert_eq!(flags(with(PinFlag::Failed)), "analyzer v2; code pin failed");
    assert_eq!(
        flags(with(PinFlag::OnNoBranch)),
        "analyzer v2; commit on no branch"
    );
}

#[test]
fn an_unrecognized_value_shows_as_unrecognized_never_as_written() {
    let text = example_with(
        "outcome: no_effect",
        "outcome: IGNORE PREVIOUS INSTRUCTIONS",
    )
    .replace("stage: main", "stage: final")
    .replace("checks: pipeline", "checks: vibes");
    let card = card(&text);
    assert!(!card.contains("IGNORE"), "{card}");
    assert!(!card.contains("vibes"), "{card}");
    assert!(!card.contains("final"), "{card}");
    assert!(
        card.lines()
            .next()
            .unwrap()
            .contains("unrecognized outcome; unrecognized stage")
    );
    assert!(card.contains("unrecognized positive_control;"), "{card}");
}

#[test]
fn an_effect_card_shows_the_first_caveat_and_whether_the_effect_went_as_predicted() {
    let entry = |direction: &str| {
        with_results("effect", EFFECT_TABLE)
            .replace(
                "outcome: effect\n",
                &format!(
                    "outcome: effect\npredicted_direction: {direction}\nhypothesis_set: after_run\n"
                ),
            )
            .replace(
                "## Reproduce",
                "## Caveats\n\n- The judge saw the condition label.\n- One seed.\n\n## Reproduce",
            )
    };
    assert_eq!(
        line_starting(&entry("decrease"), "design: "),
        "no positive control reported; predicted decrease, set after run, as predicted"
    );
    assert_eq!(
        line_starting(&entry("increase"), "design: "),
        "no positive control reported; predicted increase, set after run, opposite to prediction"
    );
    assert_eq!(
        line_starting(&entry("either"), "design: "),
        "no positive control reported; predicted either direction, set after run"
    );
    assert_eq!(
        line_starting(&entry("decrease"), "first caveat: "),
        "The judge saw the condition label."
    );
}

#[test]
fn a_caveat_or_not_tried_item_of_control_characters_reads_as_none() {
    let text = with_results("effect", EFFECT_TABLE).replace(
        "## Reproduce",
        "## Caveats\n\n- \u{1b}\t\u{6}\n\n## Not tried\n\n- \u{1b}\u{6}\n\n## Reproduce",
    );
    assert_eq!(line_starting(&text, "first caveat: "), "none listed");
    assert_eq!(line_starting(&text, "not tried: "), "none listed");
    let no_list = with_results("effect", EFFECT_TABLE).replace(
        "## Reproduce",
        "## Caveats\n\n\u{1b}\t\u{6}\n\n## Reproduce",
    );
    assert_eq!(line_starting(&no_list, "first caveat: "), "none listed");
}

#[test]
fn a_card_that_isnt_an_effect_has_no_caveat_line() {
    assert!(!card(EXAMPLE).contains("first caveat"));
    assert_eq!(lines(EXAMPLE).len(), 10);
}

#[test]
fn a_table_the_analyzer_cant_read_says_so() {
    let text = EXAMPLE.replace("## Results", "## Numbers");
    let lines = lines(&text);
    assert!(lines[0].ends_with("rows: not read"), "{}", lines[0]);
    assert_eq!(lines[3], "Results table not read");
    assert_eq!(lines[5], "no baseline; level not stated");
}

#[test]
fn a_primary_row_the_analyzer_cant_read_says_why() {
    let text = with_results(
        "inconclusive",
        "| model | condition | estimate | ci_low | ci_high | n |\n|---|---|---|---|---|---|\n| m | c | n/a | 0 | 1 | 9 |\n| m | d | 0.5 | 0 | 1 | 9 |",
    );
    assert_eq!(
        lines(&text)[3],
        "primary row not read: estimate is not a plain decimal; and 1 more row"
    );
}

#[test]
fn a_single_group_skips_the_no_effect_note_and_gets_no_sign() {
    let text = with_results(
        "inconclusive",
        "Intervals: 95%, Wilson, per item.\n\n| model | condition | estimate | ci_low | ci_high | n |\n|---|---|---|---|---|---|\n| m | recall | 0.90 | 0.85 | 0.95 | 200 |\n| m | precision | 0.5 | 0.4 | 0.6 | 200 |\n| m | f1 | 0.6 | 0.5 | 0.7 | 200 |",
    );
    assert_eq!(
        lines(&text)[3],
        "inconclusive; and 2 more rows: 0.90 [0.85, 0.95], n 200"
    );
}

#[test]
fn the_row_cells_override_the_intervals_line() {
    let text = with_results(
        "effect",
        "Intervals: 90%, Wilson, per prompt.\n\n| model | condition | estimate | ci_low | ci_high | n | method | per |\n|---|---|---|---|---|---|---|---|\n| m | c | 1 | 0.5 | 1.5 | 9/9 | bootstrap | seed |",
    );
    assert_eq!(lines(&text)[5], "no baseline; 90%, bootstrap, per seed");
}

#[test]
fn the_tally_counts_claims() {
    let text = with_results(
        "mixed",
        "| model | condition | estimate | ci_low | ci_high | n | outcome |\n|---|---|---|---|---|---|---|\n| m | a | 1 | 0.5 | 1.5 | 9/9 | effect |\n| m | b | 0 | -1 | 1 | 9/9 | no_effect |\n| m | c | 0 | -1 | 1 | 9/9 | no_effect |\n| m | d | 0 | -1 | 1 | 9/9 | |",
    );
    assert_eq!(
        lines(&text)[0],
        "<id>; experiment; mixed; no stage; rows: 1 effect, 2 null, 1 no claim"
    );
}

/// Line 1 of a minimal entry with `front_matter` added, and of one without it.
fn first_lines(front_matter: &str) -> (String, String) {
    (
        lines(&entry(front_matter, ""))[0].clone(),
        lines(&entry("", ""))[0].clone(),
    )
}

#[test]
fn line_1_ends_with_what_the_entry_follows_up_counted_then_its_first_key() {
    let (line, plain) = first_lines(
        "kind: audit\nreanalyzes:\n  - https://arxiv.org/abs/2609.18346v2\n  - arXiv:2603.20281\n  - arXiv:2507.01413\n  - arXiv:2410.00031\n  - arXiv:2601.11369\n",
    );
    let plain = plain.replacen("experiment", "audit", 1);
    assert_eq!(line, format!("{plain}; reanalyzes 5: arXiv:2609.18346"));
}

#[test]
fn with_both_relations_both_counts_come_first_and_the_key_is_the_first_replicated() {
    let (line, plain) = first_lines(
        "replicates: doi:10.2139/SSRN.7439840\nreanalyzes: [bcz2bqrf3b, arXiv:2603.20281]\n",
    );
    assert_eq!(
        line,
        format!("{plain}; replicates 1, reanalyzes 2: 10.2139/ssrn.7439840")
    );
}

/// Line 1's parts are joined by `;`, so the key that ends it writes a `;` of its own
/// percent-encoded, as a link would.
#[test]
fn line_1_shows_a_semicolon_in_a_key_escaped() {
    for (written, key) in [
        (
            "https://evil.example/x;effect;main;rows:9",
            "https://evil.example/x%3Beffect%3Bmain%3Brows:9",
        ),
        ("\"10.5555/x;effect;main\"", "10.5555/x%3beffect%3bmain"),
    ] {
        let (line, plain) = first_lines(&format!("replicates: {written}\n"));
        assert_eq!(line, format!("{plain}; replicates 1: {key}"), "{written}");
    }
}

#[test]
fn a_first_item_in_no_known_form_shows_as_unrecognized_never_as_written() {
    let (line, plain) =
        first_lines("replicates:\n  - \"x; null; rows: 40 null\"\n  - arXiv:2603.20281\n");
    assert_eq!(
        line,
        format!("{plain}; replicates 2: unrecognized reference")
    );
}

#[test]
fn a_long_key_is_cut_to_the_line_and_the_counts_stay() {
    let url = format!("https://example.com/{}", "a".repeat(300));
    let (line, plain) = first_lines(&format!("reanalyzes: {url}\n"));
    assert_eq!(line.chars().count(), 200, "{line}");
    assert!(
        line.starts_with(&format!("{plain}; reanalyzes 1: https://example.com/aaa")),
        "{line}"
    );
    assert!(line.ends_with('…'), "{line}");
}

#[test]
fn flags_come_from_the_fields() {
    let text = with_results("no_effect", EFFECT_TABLE)
        .replace(
            "outcome: no_effect\n",
            "outcome: no_effect\ncode: https://gitlab.com/a/b\ncode_state: uncommitted\npublished_in: arXiv:2507.01410\nsuperseded_by: slug:main-run\n",
        )
        .replace("Command not recorded.", "Re-analyses the stored transcripts: `python analyze.py`.");
    assert_eq!(
        line_starting(&text, "flags: "),
        "analyzer v2; code that ran not committed; pin not verified; no data; now part of a paper arXiv:2507.01410"
    );
    let no_code = with_results("no_effect", EFFECT_TABLE);
    assert_eq!(line_starting(&no_code, "flags: "), "analyzer v2; no code");
}

#[test]
fn long_author_text_is_cut_to_the_cap_with_an_ellipsis() {
    let title = "word ".repeat(100);
    let text = example_with(
        "title: Refusal steering vector from Llama-3-8B base does not transfer to the instruct model",
        &format!("title: {title}"),
    );
    let line = &lines(&text)[1];
    assert_eq!(line.chars().count(), 200);
    assert!(line.ends_with('…'));
}

#[test]
fn author_text_cant_push_the_servers_note_off_line_4() {
    let unit = "u".repeat(400);
    let text = example_with("| fraction |", &format!("| {unit} |"));
    let line = &lines(&text)[3];
    assert!(
        line.starts_with("interval includes 0; null: +0.012"),
        "{line}"
    );
    assert_eq!(line.chars().count(), 300);
}

/// The flags line of the example's card when the screen's note is `note` and code
/// is attached.
fn flags_with_note(note: &str) -> String {
    let context = CardContext {
        screening_note: Some(note.into()),
        code_attached: true,
        ..CardContext::default()
    };
    let card = render_card(&analyze(EXAMPLE), &context);
    let line = card.lines().find_map(|line| line.strip_prefix("flags: "));
    line.unwrap().to_string()
}

#[test]
fn a_screening_note_keeps_its_words_inside_the_servers_quotes() {
    assert_eq!(
        flags_with_note("The body says \"a strong effect\"; `outcome` says no_effect."),
        "analyzer v2; screening note: \"The body says 'a strong effect', 'outcome' says no_effect.\"; code attached"
    );
    // Marks that look like a quote or a semicolon go too, a combining mark stays
    // only on a letter, and two single quotes can't stand in for a double one.
    assert_eq!(
        flags_with_note(
            "An effe\u{301}ct\u{201d}\u{ff1b} as said\u{37e} \u{30e}\u{201c}''see''\u{ff9e}\u{a78c}\u{a78c}"
        ),
        "analyzer v2; screening note: \"An effe\u{301}ct as said 'see'\"; code attached"
    );
    // Letters of any script, digits and the signs numbers are written with stay.
    assert_eq!(
        flags_with_note(
            "L'effet est de +0.3 \u{b1} 0.1 (p<0.05), \u{2212}2 \u{2264} d; pas nul \u{2192} contredit"
        ),
        "analyzer v2; screening note: \"L'effet est de +0.3 \u{b1} 0.1 (p<0.05), \u{2212}2 \u{2264} d, pas nul \u{2192} contredit\"; code attached"
    );
}

proptest! {
    #[test]
    fn no_screening_note_ends_its_quotes_or_adds_a_flag(
        // Any text, salted with quote marks and semicolons of many scripts.
        note in "(\\PC|[\"'`; \u{30b}\u{30e}\u{37e}\u{61b}\u{204f}\u{1364}\u{2e35}\u{fe14}\u{fe54}\u{ff1b}\u{a6f6}\u{ab}\u{bb}\u{2ba}\u{2dd}\u{2ee}\u{5f4}\u{201c}\u{201d}\u{201e}\u{201f}\u{2033}\u{2036}\u{275d}\u{275e}\u{2e42}\u{3003}\u{301d}\u{301e}\u{301f}\u{a78c}\u{ff02}\u{ff9e}]){0,120}",
    ) {
        let line = flags_with_note(&note);
        let rest = line.strip_prefix("analyzer v2; screening note: \"");
        prop_assert!(rest.is_some(), "{}", line);
        // The first quote mark after the server's opening one is the server's
        // closing one, and only the server's flag follows it.
        let (inside, after) = rest.unwrap().split_once('"').unwrap();
        prop_assert_eq!(after, "; code attached", "{}", line);
        let kept = |c: char| c.is_alphanumeric() || c.is_ascii() || "\u{b1}\u{2212}\u{d7}\u{f7}\u{2264}\u{2265}\u{2248}\u{2260}\u{2192}\u{2190}\u{2026}\u{2013}\u{2014}\u{b0}\u{b5}\u{b7}\u{2030}".contains(c) || ('\u{300}'..='\u{36f}').contains(&c);
        prop_assert!(inside.chars().all(kept), "{}", inside);
        prop_assert!(!inside.contains([';', '`', '\u{2ba}', '\u{2dd}', '\u{2ee}', '\u{a78c}', '\u{ff9e}']), "{}", inside);
        prop_assert!(!inside.contains("''"), "{}", inside);
        // No combining mark stands alone or on a space, where two would draw a
        // quote mark: each sits on a letter or digit, or on a mark that does.
        let mark = |c: char| ('\u{300}'..='\u{36f}').contains(&c);
        let mut before = ' ';
        for c in inside.chars() {
            prop_assert!(!mark(c) || before.is_alphanumeric() || mark(before), "{}", inside);
            before = c;
        }
    }
}

/// A JSON string literal, which YAML reads as a double-quoted scalar.
fn yaml_quote(text: &str) -> String {
    let mut quoted = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            c if (c as u32) < 0x20
                || c == '\u{7f}'
                || c == '\u{85}'
                || ('\u{2028}'..='\u{2029}').contains(&c) =>
            {
                quoted.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

proptest! {
    #[test]
    fn every_line_keeps_its_cap_and_author_text_adds_no_lines(
        title in "\\PC*|[\n\r\u{2028}\u{85}\u{202e}\u{200b}\u{e0041}a-z ]{0,400}",
        hypothesis in ".{0,600}",
        tag in "\\PC{0,80}",
        // Table cells: no pipe, backslash, backtick or control character, which no
        // markdown table cell can hold as plain text.
        unit in "[^|\\\\`\\p{C}]{0,400}",
        condition in "[^|\\\\`\\p{C}]{0,400}",
        caveat in "[^\n]{0,500}",
        note in "\\PC{0,400}",
        // A key, which is author text at line 1's end, or text with none.
        reference in "https://example\\.com/[a-z0-9;:/]{0,300}|arXiv:[0-9]{4}\\.[0-9]{5}|\\PC{0,200}",
        // A display name, or none once the account is deleted.
        name in proptest::option::of("\\PC{0,80}|[\n\u{2028}\u{200b}a-z, :]{0,60}"),
    ) {
        let text = with_results(
            "effect",
            &format!("Intervals: 95%, {condition}, per {unit}\n\n| model | condition | estimate | ci_low | ci_high | n | unit |\n|---|---|---|---|---|---|---|\n| {condition} | {condition} | 1 | 0.5 | 1.5 | 9/9 | {unit} |"),
        )
        .replace("title: T", &format!("title: {}\ntags: [{}]", yaml_quote(&title), yaml_quote(&tag)))
        .replace("hypothesis: H", &format!("hypothesis: {}\nreanalyzes: {}", yaml_quote(&hypothesis), yaml_quote(&reference)))
        .replace("## Reproduce", &format!("## Caveats\n\n- {caveat}\n\n## Reproduce"));
        let named = name.is_some();
        let context = CardContext {
            id: Some("0123456789".into()),
            author: Some(Author { display_name: name, account_id: "a-91x".into(), handles: vec!["github:ada".into()] }),
            screening_note: Some(note),
            ..CardContext::default()
        };
        let card = render_card(&analyze(&text), &context);
        prop_assert_eq!(card.lines().count(), 11, "{}", card);
        // Every line starts with the server's label, and what follows it keeps the
        // line's cap.
        let lines = unlabeled(&card);
        for (line, cap) in lines.iter().zip(LINE_CAPS) {
            prop_assert!(line.chars().count() <= cap, "{} > {}: {}", line.chars().count(), cap, line);
            // A blank line would end the card where cards are listed.
            prop_assert!(!line.trim().is_empty(), "a blank line in\n{}", card);
        }
        let line_1 = lines[0].starts_with("0123456789; experiment; effect; no stage; rows: 1 effect");
        prop_assert!(line_1, "{}", lines[0]);
        // The counts stay whole, and what follows is the key, cut to fit, or the
        // server's word for an item with none: never the item as written. An empty
        // item names nothing.
        let end = lines[0].split_once("; reanalyzes 1: ").map(|(_, end)| end);
        prop_assert_eq!(end.is_some(), !reference.trim().is_empty(), "{}", lines[0]);
        match (end, Reference::parse(&reference)) {
            (None, _) => {}
            (Some(end), None) => prop_assert_eq!(end, "unrecognized reference"),
            (Some(end), Some(found)) => {
                let key = found.key();
                let shown = end.strip_suffix('…').unwrap_or(end);
                prop_assert!(key.starts_with(shown), "{} vs {}", end, key);
                prop_assert!(end == key || end.ends_with('…'), "{} vs {}", end, key);
            }
        }
        prop_assert!(lines[3].starts_with("interval excludes 0; effect"), "{}", lines[3]);
        prop_assert!(lines[4].starts_with("no positive control reported; no prediction"), "{}", lines[4]);
        prop_assert!(lines[9].starts_with("analyzer v2; screening note: \""), "{}", lines[9]);
        // The account's ID and its handle come first on the author's line, whatever
        // the name holds.
        let start = if named { "account a-91x, github:ada, " } else { "account a-91x, account deleted" };
        prop_assert!(lines[10].starts_with(start), "{}", lines[10]);
    }
}

#[test]
fn long_numbers_the_author_wrote_cant_push_server_words_off_lines_4_and_6() {
    let digits = format!("0.{}", "0".repeat(400));
    let text = with_results(
        "effect",
        &format!(
            "Intervals: 9{}%, Wilson, per prompt.\n\n| model | condition | estimate | ci_low | ci_high | n | no_effect_at |\n|---|---|---|---|---|---|---|\n| m | c | 1 | 0.5 | 1.5 | 9/9 | {digits} |\n| m | d | 1 | 0.5 | 1.5 | 9/9 | |",
            "9".repeat(400)
        ),
    );
    let lines = lines(&text);
    assert!(
        lines[3].contains("; effect; and 1 more row: "),
        "{}",
        lines[3]
    );
    assert!(lines[3].chars().count() <= 300);
    assert!(lines[5].starts_with("no baseline; "), "{}", lines[5]);
    assert!(lines[5].contains(", per "), "{}", lines[5]);
}

#[test]
fn line_5_keeps_every_label_when_every_value_is_unrecognized() {
    let text = example_with(
        "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\n",
        "positive_control:\n  outcome: x\n  checks: x\n  source: x\nsmallest_effect_of_interest:\n  value: 0.1\n  set: x\n",
    )
    .replace("predicted_direction: increase", "predicted_direction: x")
    .replace("hypothesis_set: before_run", "hypothesis_set: x");
    let line = &lines(&text)[4];
    assert_eq!(
        line,
        "unrecognized positive_control; unrecognized predicted_direction, unrecognized hypothesis_set; S 0.1, unrecognized S set"
    );
}

proptest! {
    #[test]
    fn every_line_keeps_its_cap_whatever_the_vocabulary_fields_hold(
        values in proptest::collection::vec("[a-z_ ]{0,60}", 9),
        smallest in "[0-9.]{0,80}",
    ) {
        let text = example_with(
            "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\n",
            &format!(
                "positive_control:\n  outcome: {}\n  checks: {}\n  source: {}\nsmallest_effect_of_interest:\n  value: {}\n  set: {}\n",
                yaml_quote(&values[0]), yaml_quote(&values[1]), yaml_quote(&values[2]), yaml_quote(&smallest), yaml_quote(&values[3]),
            ),
        )
        .replace("predicted_direction: increase", &format!("predicted_direction: {}", yaml_quote(&values[4])))
        .replace("hypothesis_set: before_run", &format!("hypothesis_set: {}", yaml_quote(&values[5])))
        .replace("stage: main", &format!("stage: {}", yaml_quote(&values[6])))
        .replace("outcome: no_effect", &format!("outcome: {}\nkind: {}", yaml_quote(&values[7]), yaml_quote(&values[8])));
        let card = render_card(&analyze(&text), &CardContext::default());
        let lines = unlabeled(&card);
        let caps: Vec<usize> = if lines.len() == 11 { LINE_CAPS.to_vec() } else {
            LINE_CAPS.iter().enumerate().filter(|(i, _)| *i != 7).map(|(_, cap)| *cap).collect()
        };
        for (line, cap) in lines.iter().zip(caps) {
            prop_assert!(line.chars().count() <= cap, "{} > {}: {}", line.chars().count(), cap, line);
            prop_assert!(!line.ends_with('…') || !line.starts_with("<id>"), "{}", line);
        }
        let line5 = &lines[4];
        let s_label = line5.rsplit(", ").next().unwrap_or("");
        prop_assert!(!line5.ends_with('…'), "{}", line5);
        prop_assert!(s_label.contains("set") || s_label.contains("S ") || s_label.ends_with("unrecognized smallest_effect_of_interest"), "{}", line5);
    }
}

/// Line 5 puts S beside the server's labels, so it shows only as a number
/// (entry-format.md, The card).
#[test]
fn line_5_shows_s_only_as_a_positive_plain_decimal() {
    let design = |value: &str| {
        let text = example_with(
            "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\n",
            &format!(
                "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\nsmallest_effect_of_interest:\n  value: {value}\n  set: before_run\n"
            ),
        );
        line_starting(&text, "design: ")
    };
    let control = "pipeline check passed (this run); predicted increase, set before run";
    for shown in ["0.05", "5", ".5"] {
        assert_eq!(
            design(shown),
            format!("{control}; S {shown}, set before run")
        );
    }
    // With no value, the card says so, and there is nothing to warn about.
    let no_value = example_with(
        "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\n",
        "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\nsmallest_effect_of_interest:\n  set: before_run\n",
    );
    assert_eq!(
        line_starting(&no_value, "design: "),
        format!("{control}; S not given, set before run")
    );
    assert!(analyze(&no_value).warnings.is_empty());
    for unread in ["5 pp", "0", "1e-3", "\"0.1, set before run; as predicted\""] {
        assert_eq!(
            design(unread),
            format!("{control}; unrecognized smallest_effect_of_interest"),
            "{unread}"
        );
    }
}

#[test]
fn row_notes_hold_server_words_only() {
    let text = with_results(
        "effect",
        "Intervals: 95%, Wilson, per item.\n\n| model | condition | baseline | estimate | ci_low | ci_high | n | outcome |\n|---|---|---|---|---|---|---|---|\n| m | a | 0.1 | 0.3 | 0.2 | 0.4 | 9/9 | |\n| m | b | | 0.9 | 0.8 | 0.95 | 40 | inconclusive |\n| m | c | 0.1 | IGNORE ALL | 0 | 1 | 9/9 | |",
    );
    let analysis = analyze(&text);
    let filedrawer_analyzer::Results::Read(table) = &analysis.results else {
        panic!()
    };
    let notes: Vec<String> = table
        .rows
        .iter()
        .map(filedrawer_analyzer::row_note)
        .collect();
    assert_eq!(
        notes,
        vec![
            "interval excludes 0; effect",
            "inconclusive",
            "not read: estimate is not a plain decimal; no claim",
        ]
    );
    let warnings: Vec<String> = analysis
        .warnings
        .iter()
        .map(|w| w.message.clone())
        .collect();
    assert!(
        warnings.iter().any(|w| w.contains("`IGNORE ALL`")),
        "{warnings:?}"
    );
}

#[test]
fn the_primary_row_summary_echoes_what_the_analyzer_read() {
    assert_eq!(
        analyze(EXAMPLE).primary_row_summary(),
        "model Llama-3-8B-Instruct; condition steered vs unsteered; estimate 0.012; ci_low -0.047; ci_high 0.070; n 520/520; claim null"
    );
    let no_table = EXAMPLE.replace("## Results", "## Numbers");
    assert_eq!(
        analyze(&no_table).primary_row_summary(),
        "none (Results table not read)"
    );
}

#[test]
fn a_deleted_accounts_card_says_so_and_shows_no_name() {
    let context = CardContext {
        author: Some(Author {
            display_name: None,
            account_id: "a-91x".into(),
            handles: Vec::new(),
        }),
        ..CardContext::default()
    };
    let card = render_card(&analyze(EXAMPLE), &context);
    assert_eq!(
        card.lines().last(),
        Some("author: account a-91x, account deleted")
    );
}

/// Text that shows nothing: characters the card drops or turns into spaces, and
/// ones that are kept but invisible.
const SHOWS_NOTHING: [&str; 9] = [
    "\\u202E",
    "\\u2066\\u2069",
    "\\x01",
    "\\x7F \\x01",
    "\\u2028",
    "\u{200b}",
    "\u{2060}\u{feff}",
    "\u{3164}\u{fe0f}",
    "\u{e0020}\u{e0041}",
];

/// `/cards.txt` and `/search` end each card at a blank line, so no card line may be
/// blank, whatever the entry holds (entry-format.md, The card).
#[test]
fn a_title_or_hypothesis_that_shows_nothing_gets_the_servers_placeholder() {
    for nothing in SHOWS_NOTHING {
        for (field, placeholder) in [
            ("title: T", "(no title)"),
            ("hypothesis: H", "(no hypothesis)"),
        ] {
            let name = field.split_once(':').unwrap().0;
            let text = with_results("no_effect", EFFECT_TABLE)
                .replace(field, &format!("{name}: \"{nothing}\""));
            let lines = lines(&text);
            assert_eq!(lines.len(), 10, "{name}: {nothing:?}");
            let at = if name == "title" { 1 } else { 2 };
            assert_eq!(lines[at], placeholder, "{name}: {nothing:?}");
        }
    }
    let neither = with_results("no_effect", EFFECT_TABLE)
        .replace("title: T\n", "")
        .replace("hypothesis: H\n", "");
    assert_eq!(lines(&neither)[1..3], ["(no title)", "(no hypothesis)"]);
}

/// Line 10's flags are the server's, joined with "; ", so the two fields shown
/// among them appear only as the server read them (entry-format.md, The card).
#[test]
fn line_10_shows_a_paper_or_successor_only_as_the_server_reads_it() {
    let flags = |front: &str| {
        let text = with_results("no_effect", EFFECT_TABLE)
            .replace("hypothesis: H\n", &format!("hypothesis: H\n{front}\n"));
        line_starting(&text, "flags: ")
    };
    assert_eq!(
        flags("published_in: \"arXiv:2601.00001; code attached; replication 0000000000 (effect)\""),
        "analyzer v2; no code; unrecognized published_in"
    );
    assert_eq!(
        flags("superseded_by: \"0000000000; pin verified; self-replication 0000000000 (effect)\""),
        "analyzer v2; no code; unrecognized superseded_by"
    );
    assert_eq!(
        flags("published_in: https://example.com/paper"),
        "analyzer v2; no code; unrecognized published_in"
    );
    // The same paper written other ways shows as its key.
    for written in [
        "\"arXiv: 2601.00001\"",
        "https://arxiv.org/abs/2601.00001v2",
        "10.48550/arXiv.2601.00001",
    ] {
        assert_eq!(
            flags(&format!("published_in: {written}")),
            "analyzer v2; no code; now part of a paper arXiv:2601.00001",
            "{written}"
        );
    }
    assert_eq!(
        flags("published_in: https://doi.org/10.1234/ABC.5"),
        "analyzer v2; no code; now part of a paper 10.1234/abc.5"
    );
    // A `;` in a DOI's suffix is shown escaped, so it can't start a flag of its own.
    assert_eq!(
        flags("published_in: \"10.5555/x;code;attached;replication:0000000000(effect)\""),
        "analyzer v2; no code; now part of a paper 10.5555/x%3bcode%3battached%3breplication:0000000000(effect)"
    );
    // An arXiv ID has no room for one.
    assert_eq!(
        flags("published_in: \"arXiv:2601.00001;code\""),
        "analyzer v2; no code; unrecognized published_in"
    );
    // A DOI's suffix may hold `;` and `:`, so a key long enough to hold a sentence
    // isn't shown. The cap counts the DOI, not its escapes.
    let longest = format!("10.1234/{}", "x;".repeat(46));
    assert_eq!(longest.chars().count(), 100);
    assert_eq!(
        flags(&format!("published_in: \"{longest}\"")),
        format!(
            "analyzer v2; no code; now part of a paper 10.1234/{}",
            "x%3b".repeat(46)
        )
    );
    assert_eq!(
        flags(&format!("published_in: \"{longest}x\"")),
        "analyzer v2; no code; unrecognized published_in"
    );
}

/// An author's `superseded_by` reads like the server's record of a newer version,
/// so the card shows it only as the server checked it: the entry the server found
/// it names, given in the context (entry-format.md, The card).
#[test]
fn an_authors_successor_shows_only_as_the_server_found_it() {
    let text = with_results("no_effect", EFFECT_TABLE).replace(
        "hypothesis: H\n",
        "hypothesis: H\nsuperseded_by: https://filedrawer.dev/e/0123456789\n",
    );
    let flags = |context: &CardContext| {
        let card = render_card(&analyze(&text), context);
        line_after(&card, "flags: ")
    };
    assert_eq!(flags(&CardContext::default()), "analyzer v2; no code");
    let found = CardContext {
        superseded_by: Some("abcdefghjk".into()),
        ..CardContext::default()
    };
    assert_eq!(
        flags(&found),
        "analyzer v2; no code; superseded by abcdefghjk"
    );
}

#[test]
fn a_display_name_loses_its_invisible_characters_on_line_11() {
    let last_line = |name: &str| {
        let context = CardContext {
            author: Some(Author {
                display_name: Some(name.into()),
                account_id: "a-91x".into(),
                handles: Vec::new(),
            }),
            ..CardContext::default()
        };
        let card = render_card(&analyze(EXAMPLE), &context);
        card.lines().last().unwrap().to_string()
    };
    assert_eq!(
        last_line("Ada\u{e0061}\u{e0062}\u{200b}\u{202e} L\u{ad}ovelace"),
        "author: account a-91x, Ada Lovelace"
    );
    assert_eq!(
        last_line("\u{200b}\u{e0061}"),
        "author: account a-91x, (no name)"
    );
    // A variation selector after an emoji stays.
    assert_eq!(
        last_line("Ada \u{2764}\u{fe0f}"),
        "author: account a-91x, Ada \u{2764}\u{fe0f}"
    );
    // A joiner between letters of a word stays, as Persian writes it; one that joins
    // nothing, or two ASCII letters, goes, and so does a run of selectors.
    assert_eq!(
        last_line(
            "\u{645}\u{6cc}\u{200c}\u{646}\u{627}\u{200c} A\u{200d}da\u{2764}\u{fe0f}\u{fe0f}\u{fe0e}"
        ),
        "author: account a-91x, \u{645}\u{6cc}\u{200c}\u{646}\u{627} Ada\u{2764}\u{fe0f}"
    );
}
