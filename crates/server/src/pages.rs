//! The HTML pages for people (design.md, Pages for people), from askama templates
//! with escaping on.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use analyzer::{Choice, Outcome};

use crate::accounts::{Guarded, TokenView};
use crate::entries::{ChainVersion, Doubted, EarlierVersion, OwnerEntry, PublicEntry};
use crate::removed_entries::Standing;
use crate::removed_files::{FileState, FileUse, OwnersFile};
use crate::series::{Owned, Refused};
use crate::suspension::{OwnersAccount, OwnersEntry};
use crate::withdraw::{Listed, MadePublic, Member, WithOwner};

/// A rendered page with its status code.
pub fn page(status: StatusCode, template: &impl Template) -> Response {
    match template.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(error) => {
            tracing::error!(%error, "a template failed to render");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response()
        }
    }
}

/// The outcome stamped on a card: its word, and the class that colours it.
#[derive(Debug, Clone, Copy)]
pub struct Stamp {
    pub word: &'static str,
    pub class: &'static str,
}

impl Stamp {
    pub fn of(outcome: &Choice<Outcome>) -> Stamp {
        match outcome.known() {
            Some(outcome) => Stamp {
                word: outcome.card_word(),
                class: outcome.card_word(),
            },
            None => Stamp {
                word: "unrecognised",
                class: "unknown",
            },
        }
    }
}

/// A card in the drawer: the analyzer's card, line by line, under its stamp.
pub struct CardView {
    pub id: String,
    pub stamp: Stamp,
    pub lines: Vec<String>,
}

impl CardView {
    pub fn new(id: &str, card: &str, outcome: &Choice<Outcome>) -> CardView {
        CardView {
            id: id.to_string(),
            stamp: Stamp::of(outcome),
            lines: card.lines().map(str::to_string).collect(),
        }
    }
}

/// A divider card's tab: an outcome filter.
pub struct Tab {
    pub label: &'static str,
    pub href: String,
    pub current: bool,
}

/// Whether line `index` (counted from 0) of a card is its title: line 2
/// (entry-format.md, The card), which a card in the drawer links to its entry. It
/// takes a reference because templates pass their values by reference.
pub fn is_title_line(index: &usize) -> bool {
    *index == 1
}

/// What `drawer.html` shows on the home and search pages: the outcome tabs, the
/// subject search, and the cards.
pub struct Drawer<'a> {
    pub tabs: Vec<Tab>,
    /// The words searched for, kept in the box and the tabs.
    pub query: &'a str,
    /// The `outcome` filter, or empty for all.
    pub outcome: &'a str,
    pub cards: Vec<CardView>,
}

impl<'a> Drawer<'a> {
    /// The drawer for `entries`, found with `outcome` and `query`.
    pub fn new(outcome: &'a str, query: &'a str, entries: &[PublicEntry]) -> Drawer<'a> {
        let cards = entries
            .iter()
            .map(|entry| {
                let (card, stamped) = entry.card_and_outcome();
                CardView::new(&entry.id, &card, &stamped)
            })
            .collect();
        Drawer {
            tabs: outcome_tabs(outcome, query),
            query,
            outcome,
            cards,
        }
    }
}

/// The outcome tabs, with `current` (an `outcome` value, or empty for all) marked,
/// each keeping the words searched for, `query`.
fn outcome_tabs(current: &str, query: &str) -> Vec<Tab> {
    [
        ("ALL", ""),
        ("NULL", "no_effect"),
        ("INCONCLUSIVE", "inconclusive"),
        ("EFFECT", "effect"),
        ("MIXED", "mixed"),
    ]
    .into_iter()
    .map(|(label, value)| {
        let mut href = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in [("q", query), ("outcome", value)] {
            if !value.is_empty() {
                href.append_pair(name, value);
            }
        }
        let href = href.finish();
        Tab {
            label,
            href: if href.is_empty() {
                "/search".to_string()
            } else {
                format!("/search?{href}")
            },
            current: value == current,
        }
    })
    .collect()
}

#[derive(Template)]
#[template(path = "home.html")]
pub struct HomePage<'a> {
    pub public_url: &'a str,
    pub drawer: Drawer<'a>,
}

#[derive(Template)]
#[template(path = "beta.html")]
pub struct BetaPage<'a> {
    pub public_url: &'a str,
}

/// A page of the server's own markdown, `/terms` or `/privacy`, rendered.
#[derive(Template)]
#[template(path = "legal.html")]
pub struct LegalPage<'a> {
    pub title: &'a str,
    pub html: &'a str,
}

#[derive(Template)]
#[template(path = "search.html")]
pub struct SearchPage<'a> {
    /// The words searched for as the tab's title shows them: in part, as
    /// `respond::echoed` cuts them.
    pub title: String,
    pub drawer: Drawer<'a>,
    /// The page of newer results, and of older ones, if there are any.
    pub previous: Option<String>,
    pub next: Option<String>,
}

/// One of the server's own fields above the delimiter, as the plain text has it,
/// with a link when it names an entry or a series.
pub struct Field {
    pub key: String,
    pub value: String,
    pub href: Option<String>,
}

/// The server's fields at the top of plain-text reply `text`, the lines above its
/// delimiter, so a page shows exactly what the plain text puts there.
pub fn fields_above(text: &str) -> Vec<Field> {
    let above = text
        .split_once(crate::respond::DELIMITER)
        .map_or(text, |(above, _)| above);
    above
        .lines()
        .map(|line| {
            let (key, value) = line.split_once(": ").unwrap_or(("", line));
            let first = value.split([',', ' ']).next().unwrap_or("");
            let href = match key {
                "previous" | "superseded_by" | "entry" => Some(format!("/e/{first}")),
                "link" => value.rsplit_once(" = ").map(|(_, id)| format!("/e/{id}")),
                "series" => Some(format!("/series/{first}")),
                _ => None,
            };
            Field {
                key: key.to_string(),
                value: value.to_string(),
                href,
            }
        })
        .collect()
}

/// An attached file on an entry's page: its path, and its URL on the files host
/// until the archive's owner removed it.
pub struct FileView {
    pub path: String,
    pub href: Option<String>,
}

/// The series an entry cites, below the delimiter on its page.
pub struct SeriesBlock {
    pub cite: String,
    pub href: String,
    pub title: String,
    pub html: String,
}

#[derive(Template)]
#[template(path = "entry.html")]
pub struct EntryPage<'a> {
    pub id: &'a str,
    pub stamp: Stamp,
    /// The server's fields, shown above the delimiter by `above_delimiter.html`.
    pub fields: Vec<Field>,
    /// Whether the body or series description lost any markup on the way.
    pub dropped: bool,
    pub title: Option<String>,
    /// Run descriptions, one line each.
    pub notes: Vec<String>,
    pub files: Vec<FileView>,
    pub card_lines: Vec<String>,
    pub series: Option<SeriesBlock>,
    pub front_matter: String,
    /// The body, rendered by `markdown::to_html`.
    pub body: String,
}

#[derive(Template)]
#[template(path = "series_page.html")]
pub struct SeriesPage<'a> {
    pub id: &'a str,
    /// The server's fields, shown above the delimiter by `above_delimiter.html`.
    pub fields: Vec<Field>,
    pub dropped: bool,
    pub title: Option<String>,
    /// The description, rendered by `markdown::to_html`.
    pub description: String,
}

/// A short page: a heading, a sentence and maybe a link.
#[derive(Template)]
#[template(path = "message.html")]
pub struct Message<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub link: Option<(&'a str, &'a str)>,
}

#[derive(Template, Default)]
#[template(path = "signup.html")]
pub struct SignupPage<'a> {
    pub error: Option<&'a str>,
    pub email: &'a str,
    pub display_name: &'a str,
    pub invite: &'a str,
}

#[derive(Template, Default)]
#[template(path = "login.html")]
pub struct LoginPage<'a> {
    pub error: Option<&'a str>,
    pub email: &'a str,
}

#[derive(Template)]
#[template(path = "verify.html")]
pub struct VerifyPage<'a> {
    pub token: &'a str,
}

#[derive(Template)]
#[template(path = "reset.html")]
pub struct ResetPage;

#[derive(Template)]
#[template(path = "reset_confirm.html")]
pub struct ResetConfirmPage<'a> {
    pub token: &'a str,
    pub error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "delete.html")]
pub struct DeletePage<'a> {
    pub error: Option<&'a str>,
    pub unpublished: i64,
    pub public: i64,
    /// Hidden series that stop the deletion.
    pub blocking: &'a [String],
}

#[derive(Template)]
#[template(path = "account.html")]
pub struct AccountPage<'a> {
    pub display_name: &'a str,
    pub account_id: &'a str,
    pub email: Option<&'a str>,
    pub verified: bool,
    pub tokens: &'a [TokenView],
    /// The series this account made.
    pub series: &'a [Owned],
    /// A token just made, shown this once.
    pub new_token: Option<&'a str>,
    /// Its entries that may be withdrawn.
    pub withdrawable: &'a [Listed],
    /// Its chains whose newest version waits on the archive's owner.
    pub with_owner: &'a [WithOwner],
}

/// The confirm page for withdrawing an entry, or its whole batch.
#[derive(Template)]
#[template(path = "withdraw.html")]
pub struct WithdrawPage<'a> {
    /// The entry the account chose.
    pub id: &'a str,
    pub batch_id: Option<&'a str>,
    /// The entry, or every member of its batch.
    pub members: &'a [Member],
    /// Why it can't be withdrawn now, if it can't.
    pub refusal: Option<&'a str>,
    pub made_public: &'a [MadePublic],
}

/// An entry on the admin queue page, with the title read from its text.
pub struct QueueRow<'a> {
    pub entry: &'a OwnerEntry,
    pub title: String,
}

#[derive(Template)]
#[template(path = "admin.html")]
pub struct AdminQueuePage<'a> {
    /// Each heading, with its entries: those in review, those held, then those
    /// frozen by a revoked invite code.
    pub sections: [(&'a str, Vec<&'a QueueRow<'a>>); 3],
    /// Entries that passed although the screen doubted they hold a hazard: the
    /// entry, its title, and the doubt.
    pub doubted: &'a [(&'a Doubted, String)],
    /// Series the screen refused as a likely hazard, waiting for the owner's call.
    pub refusals: &'a [Refused],
    pub guarded: &'a [Guarded],
}

#[derive(Template)]
#[template(path = "admin_account.html")]
pub struct AdminAccountPage<'a> {
    pub account: &'a OwnersAccount,
    pub entries: &'a [OwnersEntry],
}

#[derive(Template)]
#[template(path = "admin_invites.html")]
pub struct AdminInvitesPage<'a> {
    pub codes: &'a [crate::invites::Listed],
    pub error: Option<&'a str>,
    pub public_url: &'a str,
    pub max_uses: i64,
    pub max_days: i64,
}

#[derive(Template)]
#[template(path = "admin_entry.html")]
pub struct AdminEntryPage<'a> {
    pub title: &'a str,
    pub entry: &'a OwnerEntry,
    pub card: &'a str,
    /// The ID and declaration of each run it uses.
    pub runs: &'a [(String, String)],
    /// Another member of its batch was rejected, so publishing it rejects it too.
    pub batch_failed: bool,
    /// Its attached files, with their text and the screen's verdict so far.
    pub files: &'a [ShownFile],
    /// The earlier versions of its chain that wait on its verdict.
    pub held_back: &'a [EarlierVersion],
    /// Every other version of its chain, which the owner removes one by one.
    pub versions: &'a [ChainVersion],
    /// Whether the owner may remove or restore it.
    pub standing: &'a Standing,
}

/// An attached file as the owner's review page shows it.
pub struct ShownFile {
    pub sha256: String,
    pub path: String,
    pub verdict: Option<String>,
    pub removed: bool,
    pub banned: bool,
    /// `None` when its bytes are missing from disk.
    pub text: Option<String>,
}

#[derive(Template)]
#[template(path = "admin_file.html")]
pub struct AdminFilePage<'a> {
    pub file: &'a OwnersFile,
    /// `None` when its bytes are missing from disk.
    pub text: Option<&'a str>,
    /// The sets that hold it, each with an entry citing it, if any.
    pub uses: &'a [FileUse],
}
