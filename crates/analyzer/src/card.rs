//! The card: the few lines search and /cards.txt return (docs/entry-format.md, The card).

use crate::Analysis;
use crate::choice::{Choice, Outcome};

/// The analyzer version every card names.
pub const ANALYZER_VERSION: u32 = 1;

/// The longest each of the card's 11 lines may be, in characters.
pub const LINE_CAPS: [usize; 11] = [200, 200, 300, 300, 150, 300, 200, 300, 300, 300, 100];

/// What only the server knows about an entry. The CLI passes the default, and the
/// card shows placeholders such as `<id>` for what's missing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardContext {
    pub id: Option<String>,
    pub author: Option<Author>,
    /// Haiku's one-line doubt that the body matches the outcome.
    pub screening_note: Option<String>,
    pub code_attached: bool,
    pub code_link_dead: bool,
    pub logs_link_dead: bool,
    /// The ID of the version that superseded this one.
    pub superseded_by_version: Option<String>,
    pub replications: Vec<Replication>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author {
    pub display_name: String,
    pub account_id: String,
    /// Linked GitHub or ORCID handles, as the card shows them.
    pub handles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Replication {
    pub id: String,
    pub outcome: Choice<Outcome>,
    pub self_replication: bool,
}

/// Renders the card's 11 lines, joined by newlines.
pub fn render_card(_analysis: &Analysis, _context: &CardContext) -> String {
    String::new()
}
