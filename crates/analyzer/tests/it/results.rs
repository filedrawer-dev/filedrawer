//! Seam: `analyze`, for the Results table and analyzer v1's reading of each row
//! (docs/entry-format.md, The Results table; Outcome and the card's numbers).

use crate::common;

use analyzer::{Claim, Reading, Results, Tally, WarningKind, analyze};
use common::{EXAMPLE, with_results};

const HEADER: &str = "| model | condition | estimate | ci_low | ci_high | n | outcome |\n|---|---|---|---|---|---|---|";

fn table(rows: &[&str]) -> String {
    format!(
        "Intervals: 95%, Wilson, per session.\n\n{HEADER}\n{}",
        rows.join("\n")
    )
}

fn rows(text: &str) -> Vec<analyzer::Row> {
    match analyze(text).results {
        Results::Read(table) => table.rows,
        Results::NotRead(why) => panic!("table not read: {why}"),
    }
}

fn kinds(text: &str) -> Vec<WarningKind> {
    analyze(text).warnings.iter().map(|w| w.kind).collect()
}

#[test]
fn the_spec_example_row_is_read() {
    let analysis = analyze(EXAMPLE);
    let Results::Read(table) = &analysis.results else {
        panic!("not read: {:?}", analysis.results)
    };
    assert_eq!(table.intervals.level.as_deref(), Some("95"));
    assert_eq!(
        table.intervals.method.as_deref(),
        Some("Newcombe hybrid score")
    );
    assert_eq!(table.intervals.per.as_deref(), Some("prompt"));
    let row = &table.rows[0];
    assert_eq!(row.cell("counts"), "328/520 vs 322/520");
    assert_eq!(row.claim, Some(Claim::NoEffect));
    let Reading::Read(interval) = &row.reading else {
        panic!("row not read: {:?}", row.reading)
    };
    assert_eq!(
        (interval.estimate, interval.low, interval.high),
        (0.012, -0.047, 0.070)
    );
    let no_effect = interval.no_effect.as_ref().unwrap();
    assert_eq!(no_effect.value_text, "0");
    assert!(no_effect.included);
}

#[test]
fn an_entry_without_results_is_not_read_and_warned() {
    let text = EXAMPLE.replace("## Results", "## Numbers");
    assert!(matches!(analyze(&text).results, Results::NotRead(_)));
    assert_eq!(kinds(&text), vec![WarningKind::ResultsNotRead]);
}

#[test]
fn a_table_missing_a_core_column_is_not_read() {
    let text = with_results(
        "effect",
        "| model | condition | estimate | ci_low | ci_high |\n|---|---|---|---|---|\n| m | c | 1 | 0 | 2 |",
    );
    assert!(matches!(analyze(&text).results, Results::NotRead(_)));
    assert_eq!(kinds(&text), vec![WarningKind::ResultsNotRead]);
}

#[test]
fn the_first_table_with_every_core_column_is_read_and_columns_match_in_any_case_and_order() {
    let grid = "| model | a | b |\n|---|---|---|\n| m | 1 | 2 |";
    let core = "Intervals: 90%, exact, per item.\n\n| N | CI_HIGH | Estimate | ci_low | Condition | Model |\n|---|---|---|---|---|---|\n| 16 | 0.9 | 0.5 | 0.1 | x | m |";
    let text = with_results("inconclusive", &format!("{grid}\n\n{core}"));
    let rows = rows(&text);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cell("model"), "m");
    assert!(matches!(rows[0].reading, Reading::Read(_)));
}

#[test]
fn a_table_under_a_later_section_is_not_the_results_table() {
    let text = format!(
        "{}\n\n## Appendix\n\n{}",
        with_results("effect", "No table here."),
        table(&["| m | c | 1 | 0 | 2 | 16/16 | |"])
    );
    assert!(matches!(analyze(&text).results, Results::NotRead(_)));
    assert_eq!(kinds(&text), vec![WarningKind::ResultsNotRead]);
}

#[test]
fn the_primary_rows_empty_claim_takes_the_entry_outcome_and_other_empty_rows_claim_nothing() {
    let text = with_results(
        "effect",
        &table(&[
            "| m | c | 1 | 0.5 | 1.5 | 16/16 | |",
            "| m | d | 1 | 0.5 | 1.5 | 16/16 | |",
        ]),
    );
    let rows = rows(&text);
    assert_eq!(rows[0].claim, Some(Claim::Effect));
    assert_eq!(rows[1].claim, None);
}

#[test]
fn a_mixed_entrys_empty_primary_row_claims_nothing() {
    let text = with_results("mixed", &table(&["| m | c | 1 | 0.5 | 1.5 | 16/16 | |"]));
    assert_eq!(rows(&text)[0].claim, None);
}

#[test]
fn a_rows_own_claim_wins_and_an_unknown_claim_is_warned() {
    let text = with_results(
        "effect",
        &table(&[
            "| m | c | 1 | 0.5 | 1.5 | 16/16 | inconclusive |",
            "| m | d | 1 | 0.5 | 1.5 | 16/16 | supported |",
        ]),
    );
    let rows = rows(&text);
    assert_eq!(rows[0].claim, Some(Claim::Inconclusive));
    assert_eq!(rows[1].claim, None);
    assert_eq!(kinds(&text), vec![WarningKind::ClaimNotRead]);
}

#[test]
fn the_tally_counts_every_rows_claim() {
    let text = with_results(
        "no_effect",
        &table(&[
            "| m | c | 0 | -1 | 1 | 16/16 | |",
            "| m | d | 2 | 1 | 3 | 16/16 | effect |",
            "| m | e | 0 | -1 | 1 | 16/16 | no_effect |",
            "| m | f | 0 | -1 | 1 | 16/16 | inconclusive |",
            "| m | g | 0 | -1 | 1 | 16/16 | |",
        ]),
    );
    assert_eq!(
        analyze(&text).tally(),
        Tally {
            effect: 1,
            no_effect: 2,
            inconclusive: 1,
            no_claim: 1
        }
    );
}

#[test]
fn minus_signs_and_a_leading_plus_are_numbers() {
    let text = with_results(
        "effect",
        &table(&["| m | c | +0.5 | \u{2212}0.25 | 1.25 | 16/16 | |"]),
    );
    let Reading::Read(interval) = &rows(&text)[0].reading else {
        panic!("not read")
    };
    assert_eq!(
        (interval.estimate, interval.low, interval.high),
        (0.5, -0.25, 1.25)
    );
}

#[test]
fn a_row_with_a_missing_or_odd_number_is_not_read_and_says_why() {
    let text = with_results(
        "effect",
        &table(&[
            "| m | c | | 0 | 1 | 16/16 | |",
            "| m | d | 5% | 0 | 1 | 16/16 | |",
            "| m | e | 1e-3 | 0 | 1 | 16/16 | |",
        ]),
    );
    let rows = rows(&text);
    for row in &rows {
        let Reading::NotRead(why) = &row.reading else {
            panic!("read: {row:?}")
        };
        assert!(why.contains("estimate"), "{why}");
    }
    assert_eq!(kinds(&text), vec![WarningKind::RowNotRead; 3]);
}

#[test]
fn an_interval_whose_low_end_is_above_its_high_end_is_not_read() {
    let text = with_results("effect", &table(&["| m | c | 1 | 2 | 0 | 16/16 | |"]));
    assert!(matches!(rows(&text)[0].reading, Reading::NotRead(_)));
}

#[test]
fn an_interval_of_zero_width_is_read_as_written() {
    let text = with_results("effect", &table(&["| m | c | 0 | 0 | 0 | 16/16 | |"]));
    let Reading::Read(interval) = &rows(&text)[0].reading else {
        panic!("not read")
    };
    assert!(interval.no_effect.as_ref().unwrap().included);
}

#[test]
fn the_interval_is_checked_against_no_effect_at() {
    let header = "| model | condition | estimate | ci_low | ci_high | n | no_effect_at |\n|---|---|---|---|---|---|---|";
    let text = with_results(
        "effect",
        &format!(
            "{header}\n| m | odds ratio | 2 | 1.2 | 3 | 16/16 | 1 |\n| m | auroc | 0.55 | 0.45 | 0.65 | 16/16 | 0.5 |"
        ),
    );
    let rows = rows(&text);
    let excluded = |row: &analyzer::Row| match &row.reading {
        Reading::Read(interval) => interval
            .no_effect
            .as_ref()
            .map(|n| (n.value_text.clone(), n.included)),
        Reading::NotRead(_) => None,
    };
    assert_eq!(excluded(&rows[0]), Some(("1".into(), false)));
    assert_eq!(excluded(&rows[1]), Some(("0.5".into(), true)));
}

#[test]
fn a_single_group_without_no_effect_at_gets_no_note() {
    let header = "| model | condition | baseline | estimate | ci_low | ci_high | n |\n|---|---|---|---|---|---|---|";
    let text = with_results(
        "inconclusive",
        &format!(
            "{header}\n| m | recall | | 0.9 | 0.8 | 0.95 | 40 |\n| m | lift | 0.2 | 0.1 | -0.1 | 0.3 | 40 |"
        ),
    );
    let rows = rows(&text);
    let Reading::Read(level) = &rows[0].reading else {
        panic!()
    };
    assert_eq!(level.no_effect, None);
    let Reading::Read(difference) = &rows[1].reading else {
        panic!()
    };
    assert!(difference.no_effect.as_ref().unwrap().included);
}

#[test]
fn every_n_form_in_the_spec_is_accepted_and_others_are_warned() {
    let good = [
        "16/16",
        "10/10/10",
        "16",
        "96 paired",
        "80/80 (2/2 days)",
        "160 (2 days)",
        "206 paired (29 prompts)",
    ];
    for n in good {
        let text = with_results(
            "effect",
            &table(&[&format!("| m | c | 1 | 0.5 | 1.5 | {n} | |")]),
        );
        assert_eq!(kinds(&text), vec![], "{n}");
    }
    for n in ["lots", "16 vs 16", "", "16/"] {
        let text = with_results(
            "effect",
            &table(&[&format!("| m | c | 1 | 0.5 | 1.5 | {n} | |")]),
        );
        assert_eq!(kinds(&text), vec![WarningKind::OddN], "{n:?}");
    }
}

#[test]
fn intervals_without_a_level_are_warned() {
    let text = with_results(
        "effect",
        &format!(
            "Intervals: Wilson, per session.\n\n{HEADER}\n| m | c | 1 | 0.5 | 1.5 | 16/16 | |"
        ),
    );
    let Results::Read(table) = analyze(&text).results else {
        panic!()
    };
    assert_eq!(table.intervals.level, None);
    assert_eq!(table.intervals.method.as_deref(), Some("Wilson"));
    assert_eq!(table.intervals.per.as_deref(), Some("session"));
    assert_eq!(kinds(&text), vec![WarningKind::LevelNotStated]);

    let no_line = with_results(
        "effect",
        &format!("{HEADER}\n| m | c | 1 | 0.5 | 1.5 | 16/16 | |"),
    );
    assert_eq!(kinds(&no_line), vec![WarningKind::LevelNotStated]);
}

#[test]
fn the_intervals_line_splits_method_and_unit_at_the_whole_word_per() {
    let text = with_results(
        "effect",
        &format!(
            "Intervals: 99.5% bootstrap percentile (permuted), per seed\n\n{HEADER}\n| m | c | 1 | 0.5 | 1.5 | 16/16 | |"
        ),
    );
    let Results::Read(table) = analyze(&text).results else {
        panic!()
    };
    assert_eq!(table.intervals.level.as_deref(), Some("99.5"));
    assert_eq!(
        table.intervals.method.as_deref(),
        Some("bootstrap percentile (permuted)")
    );
    assert_eq!(table.intervals.per.as_deref(), Some("seed"));
}

#[test]
fn the_level_is_the_first_percentage_even_after_a_stray_percent_sign() {
    let text = with_results(
        "effect",
        &format!(
            "Intervals: Wilson (n%), 95%, per prompt\n\n{HEADER}\n| m | c | 1 | 0.5 | 1.5 | 16/16 | |"
        ),
    );
    let Results::Read(table) = analyze(&text).results else {
        panic!()
    };
    assert_eq!(table.intervals.level.as_deref(), Some("95"));
    assert_eq!(table.intervals.method.as_deref(), Some("Wilson (n%)"));
}

#[test]
fn per_splits_the_line_after_text_whose_case_changes_length() {
    let text = with_results(
        "effect",
        &format!(
            "Intervals: 95%, \u{130}stanbul bootstrap, per prompt\n\n{HEADER}\n| m | c | 1 | 0.5 | 1.5 | 16/16 | |"
        ),
    );
    let Results::Read(table) = analyze(&text).results else {
        panic!()
    };
    assert_eq!(table.intervals.per.as_deref(), Some("prompt"));
}
