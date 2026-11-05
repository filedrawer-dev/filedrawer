//! The Results table and what analyzer v1 reads from each row
//! (docs/entry-format.md, The Results table; Outcome and the card's numbers).

use std::collections::BTreeMap;

use crate::choice::Claim;

/// The six columns a table needs before the analyzer reads it.
pub const CORE_COLUMNS: [&str; 6] = ["model", "condition", "estimate", "ci_low", "ci_high", "n"];

/// The Results table, or why the analyzer couldn't read one.
#[derive(Debug, Clone, PartialEq)]
pub enum Results {
    NotRead(String),
    Read(ResultsTable),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultsTable {
    pub intervals: Intervals,
    pub rows: Vec<Row>,
}

/// The "Intervals:" line above the table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Intervals {
    /// The level as written, without the percent sign, such as "95".
    pub level: Option<String>,
    pub method: Option<String>,
    pub per: Option<String>,
}

/// One row: its cells by lowercased column name, its claim and its reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: BTreeMap<String, String>,
    /// `None` when the row claims nothing.
    pub claim: Option<Claim>,
    pub reading: Reading,
}

impl Row {
    /// The cell under `column`, trimmed; empty when the table has no such column.
    pub fn cell(&self, column: &str) -> &str {
        self.cells.get(column).map_or("", |text| text.trim())
    }
}

/// What the analyzer read from a row's numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Read(Interval),
    /// The row couldn't be read, with the reason, such as "estimate is not a number".
    NotRead(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub estimate: f64,
    pub low: f64,
    pub high: f64,
    /// The no-effect value, and whether the interval includes it. `None` on a single
    /// group with no `no_effect_at`, whose level has no no-effect value.
    pub no_effect: Option<NoEffect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoEffect {
    /// As written in `no_effect_at`, or "0" by default.
    pub value_text: String,
    pub value: f64,
    pub included: bool,
}

/// How many rows make each claim, for the card's tally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub effect: usize,
    pub no_effect: usize,
    pub inconclusive: usize,
    pub no_claim: usize,
}
