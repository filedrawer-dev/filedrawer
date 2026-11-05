//! Screening (design.md, Submission and screening, steps 3 and 4). Build step 3 adds
//! the Anthropic screener; until then only the pass-all stub exists.

/// Which screen runs on a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screener {
    /// Passes everything. For dev and staging before screening exists; production
    /// refuses it (see `Config::from_env`).
    PassAll,
}

/// A screen's decision on one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
}

impl Screener {
    pub async fn screen(&self, _source: &str) -> Verdict {
        match self {
            Screener::PassAll => Verdict::Pass,
        }
    }
}
