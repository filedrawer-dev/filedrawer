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

#[tokio::test]
async fn migration_9_guards_every_account_with_an_entry_already_in_review() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut first_eight = sqlx::migrate!("./migrations");
    first_eight.migrations = Cow::Owned(first_eight.migrations[..8].to_vec());
    first_eight.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a-1', 'Ada', 't'), ('a-2', 'Bo', 't')",
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version) VALUES
             ('e1', 'a-1', 'review', 'x', '2026-11-02T00:00:00Z', 't', 1),
             ('e2', 'a-1', 'held', 'x', '2026-11-01T00:00:00Z', 't', 1),
             ('e3', 'a-2', 'public', 'x', '2026-11-01T00:00:00Z', 't', 1)",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let guards: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, review_guard_at FROM accounts ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        guards,
        vec![
            ("a-1".to_string(), Some("2026-11-01T00:00:00Z".to_string())),
            ("a-2".to_string(), None),
        ]
    );
}

#[tokio::test]
async fn migration_11_keeps_only_guards_the_owner_confirmed() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut first_ten = sqlx::migrate!("./migrations");
    first_ten.migrations = Cow::Owned(first_ten.migrations[..10].to_vec());
    first_ten.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at, review_guard_at) VALUES
             ('a-review', 'R', 't', 't'), ('a-held', 'H', 't', 't'),
             ('a-rejected', 'J', 't', 't'), ('a-screen', 'S', 't', 't')",
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, rejection_reasons) VALUES
             ('e1', 'a-review', 'review', 'x', 't', 't', 1, NULL),
             ('e2', 'a-held', 'held', 'x', 't', 't', 1, NULL),
             ('e3', 'a-rejected', 'rejected', 'x', 't', 't', 1, '[\"rejected in review\"]'),
             ('e4', 'a-screen', 'rejected', 'x', 't', 't', 1, '[\"It is about cooking.\"]')",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let guarded: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM accounts WHERE review_guard_at IS NOT NULL ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        guarded,
        vec![("a-held".to_string(),), ("a-rejected".to_string(),)]
    );
}

#[tokio::test]
async fn migration_14_drops_stagings_attached_files_and_their_text_column() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut first_thirteen = sqlx::migrate!("./migrations");
    first_thirteen.migrations = Cow::Owned(first_thirteen.migrations[..13].to_vec());
    first_thirteen.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a-1', 'Ada', 't')",
        "INSERT INTO files (sha256, text, bytes, created_at, screen_verdict) VALUES
             ('ab12', 'print(1)', 8, 't', 'pass')",
        "INSERT INTO attachment_sets (seq, id, account_id, bytes, created_at) VALUES
             (3, 'as-1', 'a-1', 8, 't')",
        "INSERT INTO attachment_files (set_seq, path, origin, sha256) VALUES
             (3, 'run.py', 'author:run.py', 'ab12')",
        "INSERT INTO entries (seq, id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, attachment_set_seq) VALUES (7, 'e1', 'a-1', 'public', 'x', 't', 't', 1, 3)",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let left: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM files), (SELECT COUNT(*) FROM attachment_sets),
             (SELECT COUNT(*) FROM attachment_files)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, (0, 0, 0));
    let (id, set): (String, Option<i64>) =
        sqlx::query_as("SELECT id, attachment_set_seq FROM entries WHERE seq = 7")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((id.as_str(), set), ("e1", None));
    let columns: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('files')")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(!columns.iter().any(|(name,)| name == "text"), "{columns:?}");
    let violations: Vec<(String,)> =
        sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(violations, vec![]);
}

#[tokio::test]
async fn migration_21_times_the_next_check_of_every_link_staging_has_checked() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut first_twenty = sqlx::migrate!("./migrations");
    first_twenty.migrations = Cow::Owned(first_twenty.migrations[..20].to_vec());
    first_twenty.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a1', 'A', 't')",
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version) VALUES ('e1', 'a1', 'public', 'x', 't', 't', 1)",
        "INSERT INTO link_checks (entry_seq, field, url, reach, pin, checked_at) VALUES
             (1, 'logs', 'dead', 'dead', NULL, '2026-11-07T12:00:00Z'),
             (1, 'logs', 'busy', 'retry', NULL, '2026-11-07T12:00:00Z'),
             (1, 'code', 'unverified', 'live', 'not_verified', '2026-11-07T12:00:00Z'),
             (1, 'code', 'fork', 'live', 'failed', '2026-11-07T12:00:00Z'),
             (1, 'code', 'fine', 'live', 'verified', '2026-11-07T12:00:00Z')",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let next: Vec<(String, Option<String>, i64)> =
        sqlx::query_as("SELECT url, next_check_at, retries FROM link_checks ORDER BY url")
            .fetch_all(&pool)
            .await
            .unwrap();
    let week = Some("2026-11-14T12:00:00Z".to_string());
    let hour = Some("2026-11-07T13:00:00Z".to_string());
    assert_eq!(
        next,
        vec![
            ("busy".into(), hour.clone(), 0),
            ("dead".into(), week.clone(), 0),
            ("fine".into(), None, 0),
            ("fork".into(), week, 0),
            ("unverified".into(), hour, 0),
        ]
    );
}

#[tokio::test]
async fn migration_22_keeps_every_link_check_and_allows_a_commit_on_no_branch() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut first = sqlx::migrate!("./migrations");
    first.migrations = Cow::Owned(first.migrations[..21].to_vec());
    first.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a1', 'A', 't')",
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version) VALUES ('e1', 'a1', 'public', 'x', 't', 't', 1)",
        "INSERT INTO link_checks (entry_seq, field, url, reach, reach_detail, pin, pin_detail,
             checked_at, next_check_at, retries) VALUES
             (1, 'logs', 'l', 'retry', 'busy', NULL, NULL, 'c2', 'n2', 1),
             (1, 'code', 'u', 'live', 'rd', 'not_verified', 'why', 'c', 'n', 3),
             (1, 'code', 'https://github.com/a/b/tree/x', 'live', NULL, 'failed', 'fork', 'c3',
              '2099-01-01T00:00:00Z', 0)",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let kept: Vec<(String,)> = sqlx::query_as(
        "SELECT field || '|' || url || '|' || reach || '|' || IFNULL(reach_detail, '-') || '|'
             || IFNULL(pin, '-') || '|' || IFNULL(pin_detail, '-') || '|' || checked_at || '|'
             || IFNULL(next_check_at, '-') || '|' || retries
         FROM link_checks WHERE url != 'https://github.com/a/b/tree/x' ORDER BY rowid",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        kept,
        vec![
            ("logs|l|retry|busy|-|-|c2|n2|1".to_string(),),
            ("code|u|live|rd|not_verified|why|c|n|3".to_string(),),
        ]
    );
    // A GitHub commit an earlier recheck called a bad pin is checked again at once,
    // so its card takes the new flag.
    let due: (String,) = sqlx::query_as(
        "SELECT next_check_at FROM link_checks WHERE url = 'https://github.com/a/b/tree/x'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(due.0.as_str() < "2099", "{due:?}");
    sqlx::query("UPDATE link_checks SET pin = 'no_branch'")
        .execute(&pool)
        .await
        .unwrap();
    let flagged: (String,) = sqlx::query_as("SELECT pin_flag FROM link_flags")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(flagged, ("no_branch".to_string(),));
}
