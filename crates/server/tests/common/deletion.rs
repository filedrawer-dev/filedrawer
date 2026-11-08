//! What a refused deletion must leave as it was, for the tests of deleting and
//! withdrawing entries.

use super::TestApp;

/// An entry's seq, status, text, token, batch and attachment set.
type EntryRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// Everything a deletion of entries `ids` would touch, as text to compare: each
/// entry's row, slug, runs, attachment set and its files, allowance unit and
/// deletion record. A refusal leaves all of it as it was.
pub async fn state_of(app: &TestApp, ids: &[&str]) -> Vec<String> {
    let mut state = Vec::new();
    for id in ids {
        let row: Option<EntryRow> = sqlx::query_as(
            "SELECT seq, status, source, token_id, batch_seq, attachment_set_seq
                 FROM entries WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&app.pool)
        .await
        .unwrap();
        state.push(format!("{id}: {row:?}"));
        let deleted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM deleted_entries WHERE id = ?")
            .bind(id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
        state.push(format!("{id} deletion records: {deleted}"));
        let Some((seq, _, _, _, batch, set)) = row else {
            continue;
        };
        for (what, query) in [
            ("slug", "SELECT slug FROM slugs WHERE entry_seq = ?"),
            (
                "runs",
                "SELECT er.run_id || ' ' || r.declaration FROM entry_runs er
                 JOIN runs r ON r.account_id = er.account_id AND r.id = er.run_id
                 WHERE er.entry_seq = ? ORDER BY er.run_id",
            ),
            (
                "unit",
                "SELECT day || ' ' || kind FROM allowance_units WHERE entry_seq = ? ORDER BY seq",
            ),
        ] {
            let rows: Vec<String> = sqlx::query_scalar(query)
                .bind(seq)
                .fetch_all(&app.pool)
                .await
                .unwrap();
            state.push(format!("{id} {what}: {rows:?}"));
        }
        let set: Vec<(String, Option<String>, i64)> = sqlx::query_as(
            "SELECT s.id, s.deleted_at,
                    (SELECT COUNT(*) FROM attachment_files f WHERE f.set_seq = s.seq)
             FROM attachment_sets s WHERE s.seq = ?",
        )
        .bind(set)
        .fetch_all(&app.pool)
        .await
        .unwrap();
        state.push(format!("{id} set: {set:?}"));
        let batch: Vec<String> = sqlx::query_scalar("SELECT id FROM batches WHERE seq = ?")
            .bind(batch)
            .fetch_all(&app.pool)
            .await
            .unwrap();
        state.push(format!("{id} batch: {batch:?}"));
    }
    state
}
