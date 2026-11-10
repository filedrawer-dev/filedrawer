//! The search log (#45, design.md, Agent interface): what people search for and
//! whether the archive held anything, for the owner to read through SQL. A row
//! holds the UTC day, never a time, an IP, an account or a token; when one of the
//! skills searched, it also holds their User-Agent, which names the agent.

use axum::http::HeaderMap;
use sqlx::SqlitePool;
use time::{Duration, OffsetDateTime};

use crate::provenance::Provenance;

/// How long a row is kept.
const KEPT_DAYS: i64 = 90;

/// The most rows one UTC day takes, so a flood of searches can't fill the disk.
const MAX_PER_DAY: i64 = 10_000;

/// How much of a query is kept, in characters.
const MAX_QUERY: usize = 500;

/// The User-Agent File Drawer's own skills send begins with this.
const SKILL_CLIENT: &str = "filedrawer-skill";

/// The client a search's row keeps: the User-Agent when one of the skills sent it,
/// which names the agent's harness and model, and nothing for any other client.
pub fn client(headers: &HeaderMap) -> Option<String> {
    Provenance::of(headers)
        .user_agent
        .filter(|agent| agent.starts_with(SKILL_CLIENT))
}

/// Logs one search of `query` that matched `hits` entries, from `client`, at
/// `now`, unless the day already holds its most.
pub async fn record(
    pool: &SqlitePool,
    query: &str,
    hits: i64,
    client: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let day = now.date().to_string();
    let query: String = query.chars().take(MAX_QUERY).collect();
    sqlx::query!(
        "INSERT INTO search_log (day, query, hits, client)
         SELECT ?1, ?2, ?3, ?4 WHERE (SELECT COUNT(*) FROM search_log WHERE day = ?1) < ?5",
        day,
        query,
        hits,
        client,
        MAX_PER_DAY
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes the rows older than [`KEPT_DAYS`].
pub async fn tidy(pool: &SqlitePool, now: OffsetDateTime) -> Result<(), sqlx::Error> {
    let before = (now - Duration::days(KEPT_DAYS)).date().to_string();
    sqlx::query!("DELETE FROM search_log WHERE day < ?", before)
        .execute(pool)
        .await?;
    Ok(())
}
