//! The owner's report of front-matter fields the format doesn't name (design.md,
//! Admin; Feedback and usability cycles): a field that keeps coming back becomes a
//! real field.

use sqlx::SqlitePool;

/// The most fields the report lists.
pub const MOST_LISTED: usize = 200;

/// The most characters of a field's name the report shows.
const NAME_SHOWN: usize = 80;

/// A field no entry format names, as entries used it.
pub struct Reported {
    /// The key as the author wrote it: author text.
    pub name: String,
    /// How many accounts have an entry with it.
    pub accounts: i64,
    /// How many stored entries, each version counted, have it.
    pub entries: i64,
    /// When the earliest entry with it was submitted.
    pub first_seen: String,
    /// When the latest entry with it was submitted.
    pub last_seen: String,
}

/// The report: the fields listed, and whether there are more than it lists.
pub struct Report {
    pub fields: Vec<Reported>,
    pub more: bool,
}

impl Reported {
    /// The name cut to a line's worth, with `…` when it was longer.
    pub fn shown_name(&self) -> String {
        match self.name.char_indices().nth(NAME_SHOWN) {
            Some((end, _)) => format!("{}…", &self.name[..end]),
            None => self.name.clone(),
        }
    }
}

/// The unknown fields of every stored entry, the ones most accounts use first, then
/// the ones in most entries, at most [`MOST_LISTED`].
pub async fn report(pool: &SqlitePool) -> Result<Report, sqlx::Error> {
    // One more than listed, to know whether any are left out.
    let fetched = i64::try_from(MOST_LISTED + 1).expect("201 fits in an i64");
    let mut fields = sqlx::query_as!(
        Reported,
        r#"SELECT u.name AS "name!",
                  COUNT(DISTINCT e.account_id) AS "accounts!: i64",
                  COUNT(DISTINCT u.entry_seq) AS "entries!: i64",
                  MIN(e.submitted_at) AS "first_seen!: String",
                  MAX(e.submitted_at) AS "last_seen!: String"
           FROM entry_unknown_fields u JOIN entries e ON e.seq = u.entry_seq
           GROUP BY u.name
           ORDER BY 2 DESC, 3 DESC, u.name
           LIMIT ?"#,
        fetched
    )
    .fetch_all(pool)
    .await?;
    let more = fields.len() > MOST_LISTED;
    fields.truncate(MOST_LISTED);
    Ok(Report { fields, more })
}
