//! Seam: the migrations, run against a database that holds data, as staging's does.

use std::borrow::Cow;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn migration_3_keeps_every_entry_and_its_rows_and_allows_pending() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut first_two = sqlx::migrate!("./migrations");
    first_two.migrations = Cow::Owned(first_two.migrations[..2].to_vec());
    first_two.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a-1', 'Ada', 't')",
        "INSERT INTO entries (seq, id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, outcome, kind) VALUES (7, 'e1', 'a-1', 'public', 'text', 't', 't', 1, 'no_effect', 'experiment')",
        "INSERT INTO entry_rows (entry_seq, position, model, claim) VALUES (7, 1, 'm', 'no_effect')",
        "INSERT INTO entry_tags (entry_seq, tag) VALUES (7, 'steering')",
        "INSERT INTO entries_fts (rowid, title, hypothesis, safety_motivation, body) VALUES (7, 'T', 'H', 'S', 'text')",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let (id, status): (String, String) =
        sqlx::query_as("SELECT id, status FROM entries WHERE seq = 7")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((id.as_str(), status.as_str()), ("e1", "public"));
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entry_rows WHERE entry_seq = 7")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    let violations: Vec<(String,)> =
        sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(violations, vec![]);
    sqlx::query(
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, hold_until) VALUES ('e2', 'a-1', 'pending', 'x', 't', 't', 1, 't')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let fk_on: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fk_on, 1, "the migration left foreign keys off");
}

#[tokio::test]
async fn migration_6_keeps_versions_and_batches_and_allows_waiting_on_series() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut first_five = sqlx::migrate!("./migrations");
    first_five.migrations = Cow::Owned(first_five.migrations[..5].to_vec());
    first_five.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a-1', 'Ada', 't')",
        "INSERT INTO batches (seq, id, account_id, created_at) VALUES (3, 'b1', 'a-1', 't')",
        "INSERT INTO entries (seq, id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, batch_seq) VALUES (7, 'e1', 'a-1', 'public', 'text', 't', 't', 1, 3)",
        "INSERT INTO entries (seq, id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, hold_until, previous_seq) VALUES (8, 'e2', 'a-1', 'pending', 'x', 't', 't', 1, 't', 7)",
        "INSERT INTO slugs (account_id, slug, entry_seq) VALUES ('a-1', 'pilot', 7)",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let kept: (i64, Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT seq, previous_seq, batch_seq FROM entries WHERE id = 'e2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kept, (8, Some(7), None));
    let batch: Option<i64> = sqlx::query_scalar("SELECT batch_seq FROM entries WHERE id = 'e1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(batch, Some(3));
    let violations: Vec<(String,)> =
        sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(violations, vec![]);
    // One live successor per entry still holds.
    let second_successor = sqlx::query(
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, previous_seq) VALUES ('e3', 'a-1', 'pending', 'x', 't', 't', 1, 7)",
    )
    .execute(&pool)
    .await;
    assert!(second_successor.is_err());
    sqlx::query(
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version) VALUES ('e4', 'a-1', 'waiting_on_series', 'x', 't', 't', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let fk_on: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fk_on, 1, "the migration left foreign keys off");
}
