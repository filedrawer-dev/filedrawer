//! Builders for made-up entries.

pub const EXAMPLE: &str = include_str!("../fixtures/example.md");

/// The example entry with `old` replaced by `new`, which must occur in it.
pub fn example_with(old: &str, new: &str) -> String {
    assert!(EXAMPLE.contains(old), "the example has no {old:?}");
    EXAMPLE.replacen(old, new, 1)
}

/// A Results section the analyzer reads without a warning.
pub const GOOD_RESULTS: &str = "## Results\n\nIntervals: 95%, Wilson, per prompt.\n\n| model | condition | estimate | ci_low | ci_high | n |\n|---|---|---|---|---|---|\n| m | c | 0.01 | -0.05 | 0.07 | 100/100 |";

/// A minimal entry with the given extra front matter and body. A body without its
/// own `## Results` gets [`GOOD_RESULTS`].
pub fn entry(front_matter: &str, body: &str) -> String {
    let results = if body.contains("## Results") {
        ""
    } else {
        GOOD_RESULTS
    };
    format!(
        "---\ntitle: T\nsafety_motivation: S\nhypothesis: H\noutcome: no_effect\n{front_matter}---\n\n## Setup\n\nnot recorded\n\n{results}\n\n{body}\n\n## Reproduce\n\nCommand not recorded.\n"
    )
}

/// A minimal entry whose Results section is `results`.
pub fn with_results(outcome: &str, results: &str) -> String {
    format!(
        "---\ntitle: T\nsafety_motivation: S\nhypothesis: H\noutcome: {outcome}\n---\n\n## Setup\n\nnot recorded\n\n## Results\n\n{results}\n\n## Reproduce\n\nCommand not recorded.\n"
    )
}
