//! The HTML pages for people (design.md, Pages for people), from askama templates
//! with escaping on.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use analyzer::{Choice, Outcome};
use time::Date;

use crate::accounts::{Guarded, TokenView};
use crate::entries::{ChainVersion, Doubted, Found, Order, OwnerEntry, PublicEntry};
use crate::reads;
use crate::removed_entries::Standing;
use crate::removed_files::{FileState, FileUse, OwnersFile};
use crate::removed_series::SeriesStanding;
use crate::series::{Owned, Refused};
use crate::suspension::{OwnersAccount, OwnersEntry};
use crate::withdraw::{Listed, MadePublic, Member};

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

/// The outcome stamped on a card: its word, and the class that colors it.
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
                word: "unrecognized",
                class: "unknown",
            },
        }
    }
}

/// A card in the drawer: the analyzer's card, line by line, under its stamp, and on
/// Most read, its readers in the window beside the stamp.
pub struct CardView {
    pub id: String,
    pub stamp: Stamp,
    pub lines: Vec<String>,
    pub readers: Option<i64>,
}

impl CardView {
    fn new(entry: &PublicEntry, readers: Option<i64>) -> CardView {
        let (card, outcome) = entry.card_and_outcome();
        CardView {
            id: entry.id.clone(),
            stamp: Stamp::of(&outcome),
            lines: card.lines().map(str::to_string).collect(),
            readers,
        }
    }
}

/// A divider card's tab: a way to browse the drawer.
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

/// A way the home page shows the drawer, for people browsing out of curiosity
/// (design.md, Pages for people); agents search instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The newest entries first.
    Recent,
    /// The entries with the most readers in the read counter's window, a page at a
    /// time.
    MostRead,
    /// A new random draw on each load.
    Random,
}

impl View {
    pub const ALL: [View; 3] = [View::Recent, View::MostRead, View::Random];

    /// The view `?view=` names, if the server knows it.
    pub fn parse(value: &str) -> Option<View> {
        View::ALL.into_iter().find(|view| view.name() == value)
    }

    /// Its `?view=` value.
    pub fn name(self) -> &'static str {
        match self {
            View::Recent => "recent",
            View::MostRead => "read",
            View::Random => "random",
        }
    }

    fn label(self) -> &'static str {
        match self {
            View::Recent => "RECENT",
            View::MostRead => "MOST READ",
            View::Random => "RANDOM",
        }
    }

    /// The order its entries are fetched in on UTC day `today`.
    pub fn order(self, today: Date) -> Order {
        match self {
            View::Recent => Order::Newest,
            View::MostRead => Order::MostRead {
                since: reads::window_start(today),
            },
            View::Random => Order::Random,
        }
    }

    /// Whether it shows page `?page=`; the others show only their first.
    pub fn pages(self) -> bool {
        self == View::MostRead
    }

    /// The links from page `page` back to the page before and on to the next, the
    /// next only when `more` follow. Recent goes on in `/search`'s second page, which
    /// lists the newest first, 20 a page.
    pub fn links(self, page: i64, more: bool) -> (Option<String>, Option<String>) {
        let href = |page: i64| match page {
            1 => format!("/?view={}", self.name()),
            _ => format!("/?view={}&page={page}", self.name()),
        };
        match self {
            View::Recent => (None, more.then(|| "/search?page=2".to_string())),
            View::MostRead => (
                (page > 1).then(|| href(page - 1)),
                more.then(|| href(page + 1)),
            ),
            View::Random => (None, None),
        }
    }

    /// What it says under the tabs, if anything.
    fn note(self) -> Option<&'static str> {
        (self == View::MostRead)
            .then_some("Readers in the last 30 days, through the skills or in a browser.")
    }

    /// What it says when it has no entries.
    fn empty(self) -> &'static str {
        match self {
            View::MostRead => "No reads counted yet.",
            View::Recent | View::Random => "No entries match.",
        }
    }
}

/// What `drawer.html` shows on the home and search pages: the browsing tabs, the
/// search box, and the cards.
pub struct Drawer<'a> {
    pub tabs: Vec<Tab>,
    /// The words searched for, kept in the box.
    pub query: &'a str,
    /// What the tab says under the tabs, if anything.
    pub note: Option<&'static str>,
    pub cards: Vec<CardView>,
    /// What it says with no cards.
    pub empty: &'static str,
    /// A tab's links back to the page before and on to the next, when there are any.
    /// The search page has its own links.
    pub back: Option<String>,
    pub more: Option<String>,
}

impl<'a> Drawer<'a> {
    /// The drawer for what `found` holds, under the tabs with `current` marked, or
    /// none on a search, with the words searched for, `query`, in the box.
    pub fn new(current: Option<View>, query: &'a str, found: &Found) -> Drawer<'a> {
        let cards = found
            .entries
            .iter()
            .map(|entry| CardView::new(entry, found.readers(entry.seq)))
            .collect();
        let tabs = View::ALL
            .into_iter()
            .map(|view| Tab {
                label: view.label(),
                href: format!("/?view={}", view.name()),
                current: Some(view) == current,
            })
            .collect();
        Drawer {
            tabs,
            query,
            note: current.and_then(View::note),
            cards,
            empty: current.map_or("No entries match.", View::empty),
            back: None,
            more: None,
        }
    }
}

#[derive(Template)]
#[template(path = "home.html")]
pub struct HomePage<'a> {
    pub public_url: &'a str,
    /// The README above the line to paste; a page opened on a tab leaves it out,
    /// for more room to browse.
    pub readme: bool,
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
    /// Whether this environment offers GitHub sign-in.
    pub github: bool,
}

/// The form that finishes a GitHub signup: the name to show, which of GitHub's
/// verified addresses to keep, and the invite code.
#[derive(Template)]
#[template(path = "github_signup.html")]
pub struct GithubSignupPage<'a> {
    pub error: Option<&'a str>,
    pub login: &'a str,
    pub display_name: &'a str,
    pub emails: &'a [String],
    /// The address selected, by its place in `emails`.
    pub chosen: usize,
    pub invite: &'a str,
}

#[derive(Template, Default)]
#[template(path = "login.html")]
pub struct LoginPage<'a> {
    pub error: Option<&'a str>,
    pub email: &'a str,
    /// The page on this site to land on once logged in; none lands on the account page.
    pub next: Option<&'a str>,
    /// Whether this environment offers GitHub sign-in.
    pub github: bool,
}

/// The page the link emailed to a new address opens (design.md, Accounts).
#[derive(Template)]
#[template(path = "email_change.html")]
pub struct EmailChangePage<'a> {
    pub token: &'a str,
    pub new_email: &'a str,
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

/// The page behind a claim's emailed link (design.md, Accounts; #130).
#[derive(Template)]
#[template(path = "claim.html")]
pub struct ClaimPage<'a> {
    pub token: &'a str,
    pub account_id: &'a str,
    /// The entries the account holds.
    pub entries: i64,
    pub display_name: &'a str,
    pub error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "delete.html")]
pub struct DeletePage<'a> {
    pub error: Option<&'a str>,
    pub unpublished: i64,
    pub public: i64,
    /// For an account with no password, the GitHub login it types to confirm.
    pub confirm_login: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "account.html")]
pub struct AccountPage<'a> {
    /// The linked GitHub account's login, as stored.
    pub github_login: Option<&'a str>,
    /// This environment's OAuth App's client ID, when it offers GitHub sign-in.
    pub github_app: Option<&'a str>,
    pub has_password: bool,
    pub display_name: &'a str,
    pub account_id: &'a str,
    pub email: Option<&'a str>,
    pub verified: bool,
    /// The address the account asked to change to, while that change waits.
    pub pending_email: Option<&'a str>,
    pub tokens: &'a [TokenView],
    /// The series this account made.
    pub series: &'a [Owned],
    /// A token just made, shown this once.
    pub new_token: Option<&'a str>,
    /// Its entries that may be withdrawn.
    pub withdrawable: &'a [Listed],
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
    /// The series it made.
    pub series: &'a [Owned],
}

#[derive(Template)]
#[template(path = "admin_series.html")]
pub struct AdminSeriesPage<'a> {
    pub view: &'a crate::removed_series::OwnersView,
}

#[derive(Template)]
#[template(path = "admin_unknown_fields.html")]
pub struct AdminUnknownFieldsPage<'a> {
    pub report: &'a crate::unknown_fields::Report,
    pub most_listed: usize,
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
