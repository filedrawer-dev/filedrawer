//! Seam: `render_card` (docs/entry-format.md, The card).

mod common;

use analyzer::{
    Author, CardContext, Choice, LINE_CAPS, Outcome, Replication, analyze, render_card,
};
use common::{EXAMPLE, example_with, with_results};
use proptest::prelude::*;

fn card(text: &str) -> String {
    render_card(&analyze(text), &CardContext::default())
}

fn lines(text: &str) -> Vec<String> {
    card(text).lines().map(str::to_string).collect()
}

/// The card line that starts with `prefix`.
fn line_starting(text: &str, prefix: &str) -> String {
    lines(text)
        .into_iter()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starts with {prefix:?} in\n{}", card(text)))
}

const EFFECT_TABLE: &str = "Intervals: 95%, Wilson, per prompt.\n\n| model | condition | baseline | estimate | ci_low | ci_high | n | unit |\n|---|---|---|---|---|---|---|---|\n| m | treated vs control | 0.2 | -0.15 | -0.25 | -0.05 | 200/200 | fraction |";

#[test]
fn the_spec_example_renders_the_card_the_spec_describes() {
    // Lines 5 and 6 are quoted in entry-format.md's Example. Line 4 puts the server's
    // note first, as The card's rules say, ahead of the numbers as written.
    let expected = "\
<id>; experiment; null; main; rows: 1 null
Refusal steering vector from Llama-3-8B base does not transfer to the instruct model
Adding the base model's refusal direction at layer 14 raises the instruct model's refusal rate on harmful prompts.
interval includes 0; null: +0.012 [\u{2212}0.047, +0.070], n 520/520, fraction
pipeline check passed (this run); predicted increase, set before run
baseline 0.62; 95%, Newcombe hybrid score, per prompt
models: Llama-3-8B-Instruct; tags: steering, refusal, transfer
not tried: Layers other than 14.; Steering coefficients above 8.; Other base and instruct pairs.
analyzer v1
<author>";
    assert_eq!(card(EXAMPLE), expected);
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
            display_name: "Ada".into(),
            account_id: "a91x".into(),
            handles: vec!["github:ada".into()],
        }),
        screening_note: Some("The body reports an effect.".into()),
        code_attached: true,
        code_link_dead: false,
        logs_link_dead: true,
        superseded_by_version: Some("fd-9zz1".into()),
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
        lines[0].starts_with("fd-7k2q; experiment; null"),
        "{}",
        lines[0]
    );
    assert_eq!(
        lines[8],
        "analyzer v1; screening note: \"The body reports an effect.\"; code attached; logs link dead; superseded by fd-9zz1; replication fd-r1 (null); self-replication fd-r2 (effect)"
    );
    assert_eq!(lines[9], "Ada, account a91x, github:ada");
}

#[test]
fn an_unrecognised_value_shows_as_unrecognised_never_as_written() {
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
            .contains("unrecognised outcome; unrecognised stage")
    );
    assert!(card.contains("unrecognised positive_control;"), "{card}");
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
        line_starting(&entry("decrease"), "no positive control"),
        "no positive control reported; predicted decrease, set after run, as predicted"
    );
    assert_eq!(
        line_starting(&entry("increase"), "no positive control"),
        "no positive control reported; predicted increase, set after run, opposite to prediction"
    );
    assert_eq!(
        line_starting(&entry("either"), "no positive control"),
        "no positive control reported; predicted either direction, set after run"
    );
    assert_eq!(
        line_starting(&entry("decrease"), "first caveat"),
        "first caveat: The judge saw the condition label."
    );
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
        "primary row not read: estimate `n/a` is not a plain decimal; and 1 more row"
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

#[test]
fn flags_come_from_the_fields() {
    let text = with_results("no_effect", EFFECT_TABLE)
        .replace(
            "outcome: no_effect\n",
            "outcome: no_effect\ncode: https://gitlab.com/a/b\ncode_state: uncommitted\npublished_in: arXiv:2507.01410\nsuperseded_by: slug:main-run\n",
        )
        .replace("Command not recorded.", "Re-analyses the stored transcripts: `python analyze.py`.");
    assert_eq!(
        line_starting(&text, "analyzer v1"),
        "analyzer v1; code that ran not committed; pin not verified; no data; now part of a paper arXiv:2507.01410; superseded by slug:main-run"
    );
    let no_code = with_results("no_effect", EFFECT_TABLE);
    assert_eq!(
        line_starting(&no_code, "analyzer v1"),
        "analyzer v1; no code"
    );
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
        title in "\\PC*|[\n\r\u{2028}\u{85}a-z ]{0,400}",
        hypothesis in ".{0,600}",
        tag in "\\PC{0,80}",
        // Table cells: no pipe, backslash, backtick or control character, which no
        // markdown table cell can hold as plain text.
        unit in "[^|\\\\`\\p{C}]{0,400}",
        condition in "[^|\\\\`\\p{C}]{0,400}",
        caveat in "[^\n]{0,500}",
        note in "\\PC{0,400}",
    ) {
        let text = with_results(
            "effect",
            &format!("Intervals: 95%, {condition}, per {unit}\n\n| model | condition | estimate | ci_low | ci_high | n | unit |\n|---|---|---|---|---|---|---|\n| {condition} | {condition} | 1 | 0.5 | 1.5 | 9/9 | {unit} |"),
        )
        .replace("title: T", &format!("title: {}\ntags: [{}]", yaml_quote(&title), yaml_quote(&tag)))
        .replace("hypothesis: H", &format!("hypothesis: {}", yaml_quote(&hypothesis)))
        .replace("## Reproduce", &format!("## Caveats\n\n- {caveat}\n\n## Reproduce"));
        let context = CardContext { screening_note: Some(note), ..CardContext::default() };
        let card = render_card(&analyze(&text), &context);
        let lines: Vec<&str> = card.lines().collect();
        prop_assert_eq!(lines.len(), 11, "{}", card);
        for (line, cap) in lines.iter().zip(LINE_CAPS) {
            prop_assert!(line.chars().count() <= cap, "{} > {}: {}", line.chars().count(), cap, line);
        }
        prop_assert!(lines[3].starts_with("interval excludes 0; effect"), "{}", lines[3]);
        prop_assert!(lines[4].starts_with("no positive control reported; no prediction"), "{}", lines[4]);
        prop_assert!(lines[9].starts_with("analyzer v1; screening note: \""), "{}", lines[9]);
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
fn line_5_keeps_every_label_when_every_value_is_unrecognised() {
    let text = example_with(
        "positive_control:\n  outcome: passed\n  checks: pipeline\n  source: this_run\n",
        "positive_control:\n  outcome: x\n  checks: x\n  source: x\nsmallest_effect_of_interest:\n  value: 0.1\n  set: x\n",
    )
    .replace("predicted_direction: increase", "predicted_direction: x")
    .replace("hypothesis_set: before_run", "hypothesis_set: x");
    let line = &lines(&text)[4];
    assert_eq!(
        line,
        "unrecognised positive_control; unrecognised predicted_direction, unrecognised hypothesis_set; S 0.1, unrecognised S set"
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
        let lines: Vec<&str> = card.lines().collect();
        let caps: Vec<usize> = if lines.len() == 11 { LINE_CAPS.to_vec() } else {
            LINE_CAPS.iter().enumerate().filter(|(i, _)| *i != 7).map(|(_, cap)| *cap).collect()
        };
        for (line, cap) in lines.iter().zip(caps) {
            prop_assert!(line.chars().count() <= cap, "{} > {}: {}", line.chars().count(), cap, line);
            prop_assert!(!line.ends_with('…') || !line.starts_with("<id>"), "{}", line);
        }
        let line5 = lines[4];
        let s_label = line5.rsplit(", ").next().unwrap_or("");
        prop_assert!(!line5.ends_with('…'), "{}", line5);
        prop_assert!(s_label.contains("set") || s_label.contains("S "), "{}", line5);
    }
}
