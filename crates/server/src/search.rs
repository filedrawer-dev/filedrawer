//! What a stranger's search may cost (design.md, Agent interface; uncertainties.md,
//! How long a search's `q` may be, and how long a search may run): the terms the
//! full-text index will look up for a search's text, each once, and how many
//! searches run at once.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::error::AppError;

/// How long one search's statement may run (`db::with_deadline`). Searches take
/// milliseconds; this only keeps one that doesn't from holding a connection.
pub(crate) const STATEMENT_BUDGET: Duration = Duration::from_secs(5);

/// The most searches with words the site runs at once: half the database pool's 8
/// connections, so pages that search nothing always find one.
const AT_ONCE: usize = 4;

/// The most of those one client IP runs at once, so one client can't hold them all.
const PER_CLIENT: u32 = 2;

/// Places for searches that look words up in the index, the costly kind: so many at
/// once, and so many for each client. A search that finds none free is refused at
/// once; the full-text statement's own stop frees a place within
/// [`STATEMENT_BUDGET`].
#[derive(Debug)]
pub struct Places {
    at_once: usize,
    per_client: u32,
    /// How many places are taken, and by which client; no entry for none.
    taken: Mutex<(usize, HashMap<String, u32>)>,
}

/// No place was free, for the client or for anyone.
#[derive(Debug, PartialEq, Eq)]
pub enum Busy {
    Client,
    Site,
}

impl From<Busy> for AppError {
    fn from(busy: Busy) -> AppError {
        let what = match busy {
            Busy::Client => "searches in progress from this IP address",
            Busy::Site => "searches in progress",
        };
        AppError::TooManyRequests {
            what: what.to_string(),
            retry_after_secs: STATEMENT_BUDGET.as_secs(),
        }
    }
}

/// The right to run one search, given back when it is dropped.
#[derive(Debug)]
pub struct Place {
    places: Arc<Places>,
    client: String,
}

impl Places {
    /// Places for `at_once` searches, at most `per_client` of them for one client.
    pub fn new(at_once: usize, per_client: u32) -> Places {
        Places {
            at_once,
            per_client,
            taken: Mutex::new((0, HashMap::new())),
        }
    }

    /// A place for a search from `client` (a `ClientIp`), or why there is none.
    pub fn place(self: &Arc<Self>, client: &str) -> Result<Place, Busy> {
        let mut taken = self
            .taken
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (all, clients) = &mut *taken;
        let mine = clients.get(client).copied().unwrap_or(0);
        if mine >= self.per_client {
            return Err(Busy::Client);
        }
        if *all >= self.at_once {
            return Err(Busy::Site);
        }
        *all += 1;
        clients.insert(client.to_string(), mine + 1);
        Ok(Place {
            places: self.clone(),
            client: client.to_string(),
        })
    }
}

impl Default for Places {
    /// The site's: [`AT_ONCE`], [`PER_CLIENT`] each.
    fn default() -> Places {
        Places::new(AT_ONCE, PER_CLIENT)
    }
}

impl Place {
    /// Runs `search` on a task of its own, which holds the place: a statement goes on
    /// after the client that asked for it hangs up, so the place comes free when the
    /// statement ends, not when the request does.
    pub async fn run<T: Send + 'static>(
        self,
        search: impl Future<Output = T> + Send + 'static,
    ) -> T {
        let task = tokio::spawn(async move {
            let _place = self;
            search.await
        });
        match task.await {
            Ok(found) => found,
            Err(error) => std::panic::resume_unwind(error.into_panic()),
        }
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let mut taken = self
            .places
            .taken
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (all, clients) = &mut *taken;
        *all = all.saturating_sub(1);
        if let Some(mine) = clients.get_mut(&self.client) {
            *mine -= 1;
            if *mine == 0 {
                clients.remove(&self.client);
            }
        }
    }
}

/// The distinct terms of `text` as the index's tokenizer (`unicode61`, under
/// `porter`, migration 0001) splits it, in the order they first appear. Two terms
/// that differ only in case count once, as the tokenizer folds case; other
/// variants it merges, such as accents or a stem's endings, count apart.
///
/// A search costs about what its terms' entries in the index hold, so counting and
/// quoting the tokenizer's terms, and not words by some other rule, is what keeps a
/// search's size its cost: a word the tokenizer splits counts as every piece, and a
/// repeated word as one.
pub fn terms(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(is_token_character) {
        let term = &rest[start..];
        let end = term
            .char_indices()
            .find(|&(_, c)| !(is_token_character(c) || continues_a_token(c)))
            .map_or(term.len(), |(at, _)| at);
        let (term, after) = term.split_at(end);
        if seen.insert(term.to_lowercase()) {
            terms.push(term.to_string());
        }
        rest = after;
    }
    terms
}

/// Whether `unicode61` counts `c` as part of a token: its default categories, L*,
/// N* and Co, and every code point its tables don't list.
fn is_token_character(c: char) -> bool {
    TOKEN_CHARACTERS
        .binary_search_by(|&(low, high)| {
            if high < c {
                std::cmp::Ordering::Less
            } else if low > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// The combining marks `unicode61` keeps inside a token, though it never starts one
/// with them (SQLite's `sqlite3Fts5UnicodeIsdiacritic`).
fn continues_a_token(c: char) -> bool {
    const LOW: u32 = 0x08029FDF;
    const HIGH: u32 = 0x000361F8;
    match u32::from(c) {
        code @ 768..800 => LOW & (1 << (code - 768)) != 0,
        code @ 800..=817 => HIGH & (1 << (code - 800)) != 0,
        _ => false,
    }
}

/// The ranges of `unicode61`'s token characters, from the tables in the SQLite that
/// libsqlite3-sys bundles (3.54.1, `aFts5UnicodeData` and its neighbors in
/// `fts5_unicode2.c`). `tests/it/search_terms.rs` checks every code point against
/// that SQLite's own tokenizer, so an upgrade that changes them fails there.
const TOKEN_CHARACTERS: &[(char, char)] = &[
    ('\u{30}', '\u{39}'),
    ('\u{41}', '\u{5a}'),
    ('\u{61}', '\u{7a}'),
    ('\u{aa}', '\u{aa}'),
    ('\u{b2}', '\u{b3}'),
    ('\u{b5}', '\u{b5}'),
    ('\u{b9}', '\u{ba}'),
    ('\u{bc}', '\u{be}'),
    ('\u{c0}', '\u{d6}'),
    ('\u{d8}', '\u{f6}'),
    ('\u{f8}', '\u{2c1}'),
    ('\u{2c6}', '\u{2d1}'),
    ('\u{2e0}', '\u{2e4}'),
    ('\u{2ec}', '\u{2ec}'),
    ('\u{2ee}', '\u{2ee}'),
    ('\u{370}', '\u{374}'),
    ('\u{376}', '\u{37d}'),
    ('\u{37f}', '\u{383}'),
    ('\u{386}', '\u{386}'),
    ('\u{388}', '\u{3f5}'),
    ('\u{3f7}', '\u{481}'),
    ('\u{48a}', '\u{559}'),
    ('\u{560}', '\u{588}'),
    ('\u{58b}', '\u{58e}'),
    ('\u{590}', '\u{590}'),
    ('\u{5c8}', '\u{5f2}'),
    ('\u{5f5}', '\u{5ff}'),
    ('\u{605}', '\u{605}'),
    ('\u{61c}', '\u{61d}'),
    ('\u{620}', '\u{64a}'),
    ('\u{660}', '\u{669}'),
    ('\u{66e}', '\u{66f}'),
    ('\u{671}', '\u{6d3}'),
    ('\u{6d5}', '\u{6d5}'),
    ('\u{6e5}', '\u{6e6}'),
    ('\u{6ee}', '\u{6fc}'),
    ('\u{6ff}', '\u{6ff}'),
    ('\u{70e}', '\u{70e}'),
    ('\u{710}', '\u{710}'),
    ('\u{712}', '\u{72f}'),
    ('\u{74b}', '\u{7a5}'),
    ('\u{7b1}', '\u{7ea}'),
    ('\u{7f4}', '\u{7f5}'),
    ('\u{7fa}', '\u{815}'),
    ('\u{81a}', '\u{81a}'),
    ('\u{824}', '\u{824}'),
    ('\u{828}', '\u{828}'),
    ('\u{82e}', '\u{82f}'),
    ('\u{83f}', '\u{858}'),
    ('\u{85c}', '\u{85d}'),
    ('\u{85f}', '\u{8e3}'),
    ('\u{8ff}', '\u{8ff}'),
    ('\u{904}', '\u{939}'),
    ('\u{93d}', '\u{93d}'),
    ('\u{950}', '\u{950}'),
    ('\u{958}', '\u{961}'),
    ('\u{966}', '\u{96f}'),
    ('\u{971}', '\u{980}'),
    ('\u{984}', '\u{9bb}'),
    ('\u{9bd}', '\u{9bd}'),
    ('\u{9c5}', '\u{9c6}'),
    ('\u{9c9}', '\u{9ca}'),
    ('\u{9ce}', '\u{9d6}'),
    ('\u{9d8}', '\u{9e1}'),
    ('\u{9e4}', '\u{9f1}'),
    ('\u{9f4}', '\u{9f9}'),
    ('\u{9fc}', '\u{a00}'),
    ('\u{a04}', '\u{a3b}'),
    ('\u{a3d}', '\u{a3d}'),
    ('\u{a43}', '\u{a46}'),
    ('\u{a49}', '\u{a4a}'),
    ('\u{a4e}', '\u{a50}'),
    ('\u{a52}', '\u{a6f}'),
    ('\u{a72}', '\u{a74}'),
    ('\u{a76}', '\u{a80}'),
    ('\u{a84}', '\u{abb}'),
    ('\u{abd}', '\u{abd}'),
    ('\u{ac6}', '\u{ac6}'),
    ('\u{aca}', '\u{aca}'),
    ('\u{ace}', '\u{ae1}'),
    ('\u{ae4}', '\u{aef}'),
    ('\u{af2}', '\u{b00}'),
    ('\u{b04}', '\u{b3b}'),
    ('\u{b3d}', '\u{b3d}'),
    ('\u{b45}', '\u{b46}'),
    ('\u{b49}', '\u{b4a}'),
    ('\u{b4e}', '\u{b55}'),
    ('\u{b58}', '\u{b61}'),
    ('\u{b64}', '\u{b6f}'),
    ('\u{b71}', '\u{b81}'),
    ('\u{b83}', '\u{bbd}'),
    ('\u{bc3}', '\u{bc5}'),
    ('\u{bc9}', '\u{bc9}'),
    ('\u{bce}', '\u{bd6}'),
    ('\u{bd8}', '\u{bf2}'),
    ('\u{bfb}', '\u{c00}'),
    ('\u{c04}', '\u{c3d}'),
    ('\u{c45}', '\u{c45}'),
    ('\u{c49}', '\u{c49}'),
    ('\u{c4e}', '\u{c54}'),
    ('\u{c57}', '\u{c61}'),
    ('\u{c64}', '\u{c7e}'),
    ('\u{c80}', '\u{c81}'),
    ('\u{c84}', '\u{cbb}'),
    ('\u{cbd}', '\u{cbd}'),
    ('\u{cc5}', '\u{cc5}'),
    ('\u{cc9}', '\u{cc9}'),
    ('\u{cce}', '\u{cd4}'),
    ('\u{cd7}', '\u{ce1}'),
    ('\u{ce4}', '\u{d01}'),
    ('\u{d04}', '\u{d3d}'),
    ('\u{d45}', '\u{d45}'),
    ('\u{d49}', '\u{d49}'),
    ('\u{d4e}', '\u{d56}'),
    ('\u{d58}', '\u{d61}'),
    ('\u{d64}', '\u{d78}'),
    ('\u{d7a}', '\u{d81}'),
    ('\u{d84}', '\u{dc9}'),
    ('\u{dcb}', '\u{dce}'),
    ('\u{dd5}', '\u{dd5}'),
    ('\u{dd7}', '\u{dd7}'),
    ('\u{de0}', '\u{df1}'),
    ('\u{df5}', '\u{e30}'),
    ('\u{e32}', '\u{e33}'),
    ('\u{e3b}', '\u{e3e}'),
    ('\u{e40}', '\u{e46}'),
    ('\u{e50}', '\u{e59}'),
    ('\u{e5c}', '\u{eb0}'),
    ('\u{eb2}', '\u{eb3}'),
    ('\u{eba}', '\u{eba}'),
    ('\u{ebd}', '\u{ec7}'),
    ('\u{ece}', '\u{f00}'),
    ('\u{f20}', '\u{f33}'),
    ('\u{f40}', '\u{f70}'),
    ('\u{f88}', '\u{f8c}'),
    ('\u{f98}', '\u{f98}'),
    ('\u{fbd}', '\u{fbd}'),
    ('\u{fcd}', '\u{fcd}'),
    ('\u{fdb}', '\u{102a}'),
    ('\u{103f}', '\u{1049}'),
    ('\u{1050}', '\u{1055}'),
    ('\u{105a}', '\u{105d}'),
    ('\u{1061}', '\u{1061}'),
    ('\u{1065}', '\u{1066}'),
    ('\u{106e}', '\u{1070}'),
    ('\u{1075}', '\u{1081}'),
    ('\u{108e}', '\u{108e}'),
    ('\u{1090}', '\u{1099}'),
    ('\u{10a0}', '\u{10fa}'),
    ('\u{10fc}', '\u{135c}'),
    ('\u{1369}', '\u{138f}'),
    ('\u{139a}', '\u{13ff}'),
    ('\u{1401}', '\u{166c}'),
    ('\u{166f}', '\u{167f}'),
    ('\u{1681}', '\u{169a}'),
    ('\u{169d}', '\u{16ea}'),
    ('\u{16ee}', '\u{1711}'),
    ('\u{1715}', '\u{1731}'),
    ('\u{1737}', '\u{1751}'),
    ('\u{1754}', '\u{1771}'),
    ('\u{1774}', '\u{17b3}'),
    ('\u{17d7}', '\u{17d7}'),
    ('\u{17dc}', '\u{17dc}'),
    ('\u{17de}', '\u{17ff}'),
    ('\u{180f}', '\u{18a8}'),
    ('\u{18aa}', '\u{191f}'),
    ('\u{192c}', '\u{192f}'),
    ('\u{193c}', '\u{193f}'),
    ('\u{1941}', '\u{1943}'),
    ('\u{1946}', '\u{19af}'),
    ('\u{19c1}', '\u{19c7}'),
    ('\u{19ca}', '\u{19dd}'),
    ('\u{1a00}', '\u{1a16}'),
    ('\u{1a1c}', '\u{1a1d}'),
    ('\u{1a20}', '\u{1a54}'),
    ('\u{1a5f}', '\u{1a5f}'),
    ('\u{1a7d}', '\u{1a7e}'),
    ('\u{1a80}', '\u{1a9f}'),
    ('\u{1aa7}', '\u{1aa7}'),
    ('\u{1aae}', '\u{1aff}'),
    ('\u{1b05}', '\u{1b33}'),
    ('\u{1b45}', '\u{1b59}'),
    ('\u{1b7d}', '\u{1b7f}'),
    ('\u{1b83}', '\u{1ba0}'),
    ('\u{1bae}', '\u{1be5}'),
    ('\u{1bf4}', '\u{1bfb}'),
    ('\u{1c00}', '\u{1c23}'),
    ('\u{1c38}', '\u{1c3a}'),
    ('\u{1c40}', '\u{1c7d}'),
    ('\u{1c80}', '\u{1cbf}'),
    ('\u{1cc8}', '\u{1ccf}'),
    ('\u{1ce9}', '\u{1cec}'),
    ('\u{1cee}', '\u{1cf1}'),
    ('\u{1cf5}', '\u{1dbf}'),
    ('\u{1de7}', '\u{1dfb}'),
    ('\u{1e00}', '\u{1fbc}'),
    ('\u{1fbe}', '\u{1fbe}'),
    ('\u{1fc2}', '\u{1fcc}'),
    ('\u{1fd0}', '\u{1fdc}'),
    ('\u{1fe0}', '\u{1fec}'),
    ('\u{1ff0}', '\u{1ffc}'),
    ('\u{1fff}', '\u{1fff}'),
    ('\u{2065}', '\u{2069}'),
    ('\u{2070}', '\u{2079}'),
    ('\u{207f}', '\u{2089}'),
    ('\u{208f}', '\u{209f}'),
    ('\u{20ba}', '\u{20cf}'),
    ('\u{20f1}', '\u{20ff}'),
    ('\u{2102}', '\u{2102}'),
    ('\u{2107}', '\u{2107}'),
    ('\u{210a}', '\u{2113}'),
    ('\u{2115}', '\u{2115}'),
    ('\u{2119}', '\u{211d}'),
    ('\u{2124}', '\u{2124}'),
    ('\u{2126}', '\u{2126}'),
    ('\u{2128}', '\u{2128}'),
    ('\u{212a}', '\u{212d}'),
    ('\u{212f}', '\u{2139}'),
    ('\u{213c}', '\u{213f}'),
    ('\u{2145}', '\u{2149}'),
    ('\u{214e}', '\u{214e}'),
    ('\u{2150}', '\u{218f}'),
    ('\u{23f4}', '\u{23ff}'),
    ('\u{2427}', '\u{243f}'),
    ('\u{244b}', '\u{249b}'),
    ('\u{24ea}', '\u{24ff}'),
    ('\u{2700}', '\u{2700}'),
    ('\u{2776}', '\u{2793}'),
    ('\u{2b4d}', '\u{2b4f}'),
    ('\u{2b5a}', '\u{2ce4}'),
    ('\u{2ceb}', '\u{2cee}'),
    ('\u{2cf2}', '\u{2cf8}'),
    ('\u{2cfd}', '\u{2cfd}'),
    ('\u{2d00}', '\u{2d6f}'),
    ('\u{2d71}', '\u{2d7e}'),
    ('\u{2d80}', '\u{2ddf}'),
    ('\u{2e2f}', '\u{2e2f}'),
    ('\u{2e3c}', '\u{2e7f}'),
    ('\u{2e9a}', '\u{2e9a}'),
    ('\u{2ef4}', '\u{2eff}'),
    ('\u{2fd6}', '\u{2fef}'),
    ('\u{2ffc}', '\u{2fff}'),
    ('\u{3005}', '\u{3007}'),
    ('\u{3021}', '\u{3029}'),
    ('\u{3031}', '\u{3035}'),
    ('\u{3038}', '\u{303c}'),
    ('\u{3040}', '\u{3098}'),
    ('\u{309d}', '\u{309f}'),
    ('\u{30a1}', '\u{30fa}'),
    ('\u{30fc}', '\u{318f}'),
    ('\u{3192}', '\u{3195}'),
    ('\u{31a0}', '\u{31bf}'),
    ('\u{31e4}', '\u{31ff}'),
    ('\u{321f}', '\u{3229}'),
    ('\u{3248}', '\u{324f}'),
    ('\u{3251}', '\u{325f}'),
    ('\u{3280}', '\u{3289}'),
    ('\u{32b1}', '\u{32bf}'),
    ('\u{32ff}', '\u{32ff}'),
    ('\u{3400}', '\u{4dbf}'),
    ('\u{4e00}', '\u{a48f}'),
    ('\u{a4c7}', '\u{a4fd}'),
    ('\u{a500}', '\u{a60c}'),
    ('\u{a610}', '\u{a66e}'),
    ('\u{a67f}', '\u{a69e}'),
    ('\u{a6a0}', '\u{a6ef}'),
    ('\u{a6f8}', '\u{a6ff}'),
    ('\u{a717}', '\u{a71f}'),
    ('\u{a722}', '\u{a788}'),
    ('\u{a78b}', '\u{a801}'),
    ('\u{a803}', '\u{a805}'),
    ('\u{a807}', '\u{a80a}'),
    ('\u{a80c}', '\u{a822}'),
    ('\u{a82c}', '\u{a835}'),
    ('\u{a83a}', '\u{a873}'),
    ('\u{a878}', '\u{a87f}'),
    ('\u{a882}', '\u{a8b3}'),
    ('\u{a8c5}', '\u{a8cd}'),
    ('\u{a8d0}', '\u{a8df}'),
    ('\u{a8f2}', '\u{a8f7}'),
    ('\u{a8fb}', '\u{a925}'),
    ('\u{a930}', '\u{a946}'),
    ('\u{a954}', '\u{a95e}'),
    ('\u{a960}', '\u{a97f}'),
    ('\u{a984}', '\u{a9b2}'),
    ('\u{a9ce}', '\u{a9dd}'),
    ('\u{a9e0}', '\u{aa28}'),
    ('\u{aa37}', '\u{aa42}'),
    ('\u{aa44}', '\u{aa4b}'),
    ('\u{aa4e}', '\u{aa5b}'),
    ('\u{aa60}', '\u{aa76}'),
    ('\u{aa7a}', '\u{aa7a}'),
    ('\u{aa7c}', '\u{aaaf}'),
    ('\u{aab1}', '\u{aab1}'),
    ('\u{aab5}', '\u{aab6}'),
    ('\u{aab9}', '\u{aabd}'),
    ('\u{aac0}', '\u{aac0}'),
    ('\u{aac2}', '\u{aadd}'),
    ('\u{aae0}', '\u{aaea}'),
    ('\u{aaf2}', '\u{aaf4}'),
    ('\u{aaf7}', '\u{abe2}'),
    ('\u{abee}', '\u{d7ff}'),
    ('\u{e000}', '\u{fb1d}'),
    ('\u{fb1f}', '\u{fb28}'),
    ('\u{fb2a}', '\u{fbb1}'),
    ('\u{fbc2}', '\u{fd3d}'),
    ('\u{fd40}', '\u{fdfb}'),
    ('\u{fdfe}', '\u{fdff}'),
    ('\u{fe1a}', '\u{fe1f}'),
    ('\u{fe27}', '\u{fe2f}'),
    ('\u{fe53}', '\u{fe53}'),
    ('\u{fe67}', '\u{fe67}'),
    ('\u{fe6c}', '\u{fefe}'),
    ('\u{ff00}', '\u{ff00}'),
    ('\u{ff10}', '\u{ff19}'),
    ('\u{ff21}', '\u{ff3a}'),
    ('\u{ff41}', '\u{ff5a}'),
    ('\u{ff66}', '\u{ffdf}'),
    ('\u{ffe7}', '\u{ffe7}'),
    ('\u{ffef}', '\u{fff8}'),
    // SQLite reads U+FFFE and U+FFFF as U+FFFD, a symbol.
    ('\u{10000}', '\u{100ff}'),
    ('\u{10103}', '\u{10136}'),
    ('\u{10140}', '\u{10178}'),
    ('\u{1018a}', '\u{1018f}'),
    ('\u{1019c}', '\u{101cf}'),
    ('\u{101fe}', '\u{1039e}'),
    ('\u{103a0}', '\u{103cf}'),
    ('\u{103d1}', '\u{10856}'),
    ('\u{10858}', '\u{1091e}'),
    ('\u{10920}', '\u{1093e}'),
    ('\u{10940}', '\u{10a00}'),
    ('\u{10a04}', '\u{10a04}'),
    ('\u{10a07}', '\u{10a0b}'),
    ('\u{10a10}', '\u{10a37}'),
    ('\u{10a3b}', '\u{10a3e}'),
    ('\u{10a40}', '\u{10a4f}'),
    ('\u{10a59}', '\u{10a7e}'),
    ('\u{10a80}', '\u{10b38}'),
    ('\u{10b40}', '\u{10fff}'),
    ('\u{11003}', '\u{11037}'),
    ('\u{1104e}', '\u{1107f}'),
    ('\u{11083}', '\u{110af}'),
    ('\u{110c2}', '\u{110ff}'),
    ('\u{11103}', '\u{11126}'),
    ('\u{11135}', '\u{1113f}'),
    ('\u{11144}', '\u{1117f}'),
    ('\u{11183}', '\u{111b2}'),
    ('\u{111c1}', '\u{111c4}'),
    ('\u{111c9}', '\u{116aa}'),
    ('\u{116b8}', '\u{1246f}'),
    ('\u{12474}', '\u{16f50}'),
    ('\u{16f7f}', '\u{16f8e}'),
    ('\u{16f93}', '\u{1cfff}'),
    ('\u{1d0f6}', '\u{1d0ff}'),
    ('\u{1d127}', '\u{1d128}'),
    ('\u{1d1de}', '\u{1d1ff}'),
    ('\u{1d246}', '\u{1d2ff}'),
    ('\u{1d357}', '\u{1d6c0}'),
    ('\u{1d6c2}', '\u{1d6da}'),
    ('\u{1d6dc}', '\u{1d6fa}'),
    ('\u{1d6fc}', '\u{1d714}'),
    ('\u{1d716}', '\u{1d734}'),
    ('\u{1d736}', '\u{1d74e}'),
    ('\u{1d750}', '\u{1d76e}'),
    ('\u{1d770}', '\u{1d788}'),
    ('\u{1d78a}', '\u{1d7a8}'),
    ('\u{1d7aa}', '\u{1d7c2}'),
    ('\u{1d7c4}', '\u{1eeef}'),
    ('\u{1eef2}', '\u{1efff}'),
    ('\u{1f02c}', '\u{1f02f}'),
    ('\u{1f094}', '\u{1f09f}'),
    ('\u{1f0af}', '\u{1f0b0}'),
    ('\u{1f0bf}', '\u{1f0c0}'),
    ('\u{1f0d0}', '\u{1f0d0}'),
    ('\u{1f0e0}', '\u{1f10f}'),
    ('\u{1f12f}', '\u{1f12f}'),
    ('\u{1f16c}', '\u{1f16f}'),
    ('\u{1f19b}', '\u{1f1e5}'),
    ('\u{1f203}', '\u{1f20f}'),
    ('\u{1f23b}', '\u{1f23f}'),
    ('\u{1f249}', '\u{1f24f}'),
    ('\u{1f252}', '\u{1f2ff}'),
    ('\u{1f321}', '\u{1f32f}'),
    ('\u{1f336}', '\u{1f336}'),
    ('\u{1f37d}', '\u{1f37f}'),
    ('\u{1f394}', '\u{1f39f}'),
    ('\u{1f3c5}', '\u{1f3c5}'),
    ('\u{1f3cb}', '\u{1f3df}'),
    ('\u{1f3f1}', '\u{1f3ff}'),
    ('\u{1f43f}', '\u{1f43f}'),
    ('\u{1f441}', '\u{1f441}'),
    ('\u{1f4f8}', '\u{1f4f8}'),
    ('\u{1f4fd}', '\u{1f4ff}'),
    ('\u{1f53e}', '\u{1f53f}'),
    ('\u{1f544}', '\u{1f54f}'),
    ('\u{1f568}', '\u{1f5fa}'),
    ('\u{1f641}', '\u{1f644}'),
    ('\u{1f650}', '\u{1f67f}'),
    ('\u{1f6c6}', '\u{1f6ff}'),
    ('\u{1f774}', '\u{e0000}'),
    ('\u{e0002}', '\u{e001f}'),
    ('\u{e0080}', '\u{e00ff}'),
    ('\u{e01f0}', '\u{10ffff}'),
];
