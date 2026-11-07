//! Invite codes (design.md, Limits and abuse): made by the owner, with a label, a
//! number of uses and an expiry; an account that signs up with one skips the
//! per-IP signup and email limits and the new-account allowance while the code
//! isn't revoked.

use sqlx::{SqliteExecutor, SqlitePool};
use time::{Duration, OffsetDateTime};

use crate::ids;
use crate::time::stamp;

/// The most uses and days the owner's form takes: a large sprint, a long one.
pub const MAX_USES: i64 = 10_000;
pub const MAX_DAYS: i64 = 365;

/// A code as the owner's page lists it.
pub struct Listed {
    pub code: String,
    pub label: String,
    pub max_uses: i64,
    pub used: i64,
    pub expires_at: String,
    pub revoked_at: Option<String>,
}

/// Makes a code good for `uses` signups over `days` days from `now`; returns it.
pub async fn create(
    pool: &SqlitePool,
    label: &str,
    uses: i64,
    days: i64,
    now: OffsetDateTime,
) -> Result<String, sqlx::Error> {
    let code = ids::new_invite_code();
    let created_at = stamp(now);
    let expires_at = stamp(now + Duration::days(days));
    sqlx::query!(
        "INSERT INTO invite_codes (code, label, max_uses, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?)",
        code,
        label,
        uses,
        created_at,
        expires_at
    )
    .execute(pool)
    .await?;
    Ok(code)
}

/// Every code, newest first, with how many accounts used it.
pub async fn list(pool: &SqlitePool) -> Result<Vec<Listed>, sqlx::Error> {
    sqlx::query_as!(
        Listed,
        r#"SELECT c.code, c.label, c.max_uses,
                  (SELECT COUNT(*) FROM accounts a WHERE a.invite_seq = c.seq) AS "used!: i64",
                  c.expires_at, c.revoked_at
           FROM invite_codes c ORDER BY c.seq DESC"#
    )
    .fetch_all(pool)
    .await
}

/// Revokes `code` at `now`, and freezes every unscreened entry of the accounts made
/// with it: those in their hold (`pending`) and those in `screening` with no verdict.
/// Their accounts lose the code's exemptions with it. Returns how many entries froze,
/// or `None` if there is no such code or it was revoked already.
pub async fn revoke(
    pool: &SqlitePool,
    code: &str,
    now: OffsetDateTime,
) -> Result<Option<u64>, sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let at = stamp(now);
    let revoked = sqlx::query_scalar!(
        r#"UPDATE invite_codes SET revoked_at = ? WHERE code = ? AND revoked_at IS NULL
           RETURNING seq AS "seq!: i64""#,
        at,
        code
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(seq) = revoked else {
        return Ok(None);
    };
    let frozen = sqlx::query!(
        "UPDATE entries SET status = 'frozen'
         WHERE account_id IN (SELECT id FROM accounts WHERE invite_seq = ?)
           AND (status = 'pending' OR (status = 'screening' AND screen_verdict IS NULL))",
        seq
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(Some(frozen))
}

/// The code `code` names, if it can take a signup at `now`: not expired, not
/// revoked, with a use left.
pub async fn usable(
    conn: impl SqliteExecutor<'_>,
    code: &str,
    now: OffsetDateTime,
) -> Result<Option<i64>, sqlx::Error> {
    let now = stamp(now);
    sqlx::query_scalar!(
        r#"SELECT c.seq AS "seq!: i64" FROM invite_codes c
           WHERE c.code = ? AND c.revoked_at IS NULL AND c.expires_at > ?
             AND (SELECT COUNT(*) FROM accounts a WHERE a.invite_seq = c.seq) < c.max_uses"#,
        code,
        now
    )
    .fetch_optional(conn)
    .await
}

/// Whether `account_id` signed up with a code that isn't revoked, so keeps its
/// exemptions.
pub async fn invited(conn: impl SqliteExecutor<'_>, account_id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM accounts a JOIN invite_codes c ON c.seq = a.invite_seq
               WHERE a.id = ? AND c.revoked_at IS NULL
           ) AS "invited!: bool""#,
        account_id
    )
    .fetch_one(conn)
    .await
}
