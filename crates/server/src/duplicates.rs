//! Identical text from an unclaimed account (design.md, Limits and abuse;
//! #130). A run with no lasting home folder makes a new account each time and can't
//! see what the last one posted, so the server refuses an unclaimed account text
//! that is already public, that the account holds itself, or that another unclaimed
//! account has waiting. An account with an owner isn't checked: its agent reads
//! what it posted from `/entries/mine`.

use analyzer::without_attachments;
use sqlx::SqlitePool;

use crate::accounts;

/// How many bytes longer or shorter a stored text may be and still be compared: the
/// `attachments` field, which names a set and so differs between two posts of one
/// entry, is left out of the comparison, with any comment on its line. A set's ID
/// is 13 bytes, so this leaves room for a comment of a sentence or two; a longer
/// one hides a duplicate, and never refuses text that is new.
const ATTACHMENTS_FIELD_BYTES: i64 = 300;

/// Why `account_id` may not post `text`, as stored: none for text it may post, and
/// always none for an account with an owner. `replacing` is the entry a PUT
/// replaces, whose own text it may repeat.
///
/// A refusal names the entry only when the account may read it: a public one, or
/// its own. A hidden entry of an account with an owner refuses nothing, since that
/// would say the text exists to an account that can't read it.
pub async fn refusal(
    pool: &SqlitePool,
    public_url: &str,
    account_id: &str,
    text: &str,
    replacing: Option<i64>,
) -> Result<Option<String>, sqlx::Error> {
    if !accounts::is_unclaimed(pool, account_id).await? {
        return Ok(None);
    }
    let wanted = without_attachments(text);
    // In bytes, as the index has it: SQLite's length of TEXT stops at a NUL.
    let bytes = i64::try_from(text.len()).unwrap_or(i64::MAX);
    let (shortest, longest) = (
        bytes.saturating_sub(ATTACHMENTS_FIELD_BYTES),
        bytes.saturating_add(ATTACHMENTS_FIELD_BYTES),
    );
    // Those the account may read come first, so the reason can name one. The index
    // is named because, with no statistics, SQLite would search by status and read
    // every public entry's text to measure it.
    let candidates = sqlx::query!(
        r#"SELECT e.seq AS "seq!", e.id, e.source,
                  (e.status = 'public' OR e.account_id = ?3) AS "readable!: bool"
           FROM entries e INDEXED BY entries_source_length
           JOIN accounts a ON a.id = e.account_id
           WHERE length(CAST(e.source AS BLOB)) BETWEEN ?1 AND ?2
             AND (e.status = 'public'
                  OR ((e.account_id = ?3 OR a.unclaimed)
                      AND e.status IN ('pending', 'screening', 'review', 'held', 'frozen')))
           ORDER BY 4 DESC, e.seq"#,
        shortest,
        longest,
        account_id
    )
    .fetch_all(pool)
    .await?;
    let same = candidates
        .iter()
        .filter(|entry| Some(entry.seq) != replacing)
        .find(|entry| without_attachments(&entry.source) == wanted);
    Ok(same.map(|entry| {
        if entry.readable {
            format!(
                "this text is already posted, as {id} ({public_url}/e/{id}); an account \
                 nobody has claimed can't post the same text again",
                id = entry.id
            )
        } else {
            "this text already waits to publish from another account an agent made; an \
             account nobody has claimed can't post the same text again"
                .to_string()
        }
    }))
}
