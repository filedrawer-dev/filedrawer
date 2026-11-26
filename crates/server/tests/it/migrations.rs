//! Seam: the migrations, run against a database that holds data, as staging's does.

use std::borrow::Cow;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::common::scratch::Scratch;

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
    // Up to migration 32: 33 retires `waiting_on_series` again.
    let mut first_six = sqlx::migrate!("./migrations");
    first_six.migrations = Cow::Owned(first_six.migrations[..6].to_vec());
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

    first_six.run(&pool).await.unwrap();

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

#[tokio::test]
async fn migration_23_marks_each_unit_with_no_entry_as_a_series_unit() {
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
    first.migrations = Cow::Owned(first.migrations[..22].to_vec());
    first.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a1', 'A', 't')",
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version) VALUES ('e1', 'a1', 'pending', 'x', 't', 't', 1)",
        "INSERT INTO allowance_units (account_id, day, entry_seq) VALUES
             ('a1', '2026-11-08', 1), ('a1', '2026-11-08', NULL)",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let kinds: Vec<(Option<i64>, String)> =
        sqlx::query_as("SELECT entry_seq, kind FROM allowance_units ORDER BY seq")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        kinds,
        vec![(Some(1), "entry".to_string()), (None, "series".to_string())]
    );
    let token: (Option<String>,) = sqlx::query_as("SELECT token_id FROM entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(token, (None,), "an older entry has no token to delete it");
}

/// A file database opened by `db::open`, then closed, for a test to change behind its
/// back and open again.
async fn opened_once(name: &str) -> Scratch {
    let scratch = Scratch::database(&format!("migrations-{name}"));
    filedrawer_server::db::open(scratch.to_str().unwrap())
        .await
        .unwrap()
        .close()
        .await;
    scratch
}

async fn execute_behind(path: &str, statement: &'static str) {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::from_str(&format!("sqlite:{path}")).unwrap())
        .await
        .unwrap();
    sqlx::query(statement).execute(&pool).await.unwrap();
    pool.close().await;
}

#[tokio::test]
async fn a_binary_opens_a_database_a_newer_binary_migrated() {
    // A rollback runs the older binary on the newer schema (design.md, Hosting and
    // operations), whose migrations it doesn't know.
    let scratch = opened_once("newer").await;
    let path = scratch.display().to_string();
    execute_behind(
        &path,
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
         VALUES (99999, 'from a newer binary', CURRENT_TIMESTAMP, 1, x'00', 0)",
    )
    .await;
    let pool = filedrawer_server::db::open(&path).await.unwrap();
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(accounts, 0);
}

#[tokio::test]
async fn a_known_migration_whose_text_changed_is_refused() {
    let scratch = opened_once("changed").await;
    let path = scratch.display().to_string();
    execute_behind(
        &path,
        "UPDATE _sqlx_migrations SET checksum = x'00' WHERE version = 1",
    )
    .await;
    let error = filedrawer_server::db::open(&path).await.unwrap_err();
    assert!(error.to_string().contains("modified"), "{error}");
}

/// A database migrated to 32 that holds what migration 33 retires, as staging's may:
/// entries 2 (embargoed, with what was read from it) and 3 (waiting on hidden series
/// s-1), each followed or cited by kept entries, and entries the owner removed from
/// each status.
async fn held_entries_at_32() -> sqlx::SqlitePool {
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
    first.migrations = Cow::Owned(first.migrations[..32].to_vec());
    first.run(&pool).await.unwrap();
    for statement in [
        "INSERT INTO accounts (id, display_name, created_at) VALUES ('a-1', 'Ada', 't'), ('a-2', 'Bo', 't')",
        // s-1 hidden and unread, s-2 hidden but read, s-3 public, s-4 hidden and
        // removed by the owner.
        "INSERT INTO series (seq, id, account_id, key_hash, created_at, public_at, removed_at) VALUES
             (1, 's-1', 'a-1', x'01', 't', NULL, NULL), (2, 's-2', 'a-1', x'02', 't', NULL, NULL),
             (3, 's-3', 'a-1', x'03', 't', 't', NULL), (4, 's-4', 'a-1', x'04', 't', NULL, 't')",
        "INSERT INTO series_versions (series_seq, version, title, description, created_at, readable_at) VALUES
             (1, 1, 'S1', 'd', 't', NULL), (2, 1, 'S2', 'd', 't', 't'),
             (3, 1, 'S3', 'd', 't', NULL), (4, 1, 'S4', 'd', 't', NULL)",
        "INSERT INTO series_members (series_seq, account_id, joined_at) VALUES
             (1, 'a-1', 't'), (1, 'a-2', 't'), (2, 'a-1', 't')",
        "INSERT INTO batches (seq, id, account_id, created_at) VALUES (5, 'b-held', 'a-1', 't')",
        "INSERT INTO runs (account_id, id, declaration) VALUES ('a-1', 'r-held', '{}')",
        "INSERT INTO entries (seq, id, account_id, status, source, submitted_at, first_submitted_at,
             embargo_until, published_at, analyzer_version, outcome, kind, batch_seq, series_seq,
             series_version, removed_from, removed_at, skill, user_agent) VALUES
             (1, 'e-public', 'a-1', 'public', 'public text', 't1', 't0', NULL, 't2', 1, 'no_effect',
              'experiment', NULL, 2, 1, NULL, NULL, 'submit@1', 'agent/1'),
             (2, 'e-embargoed', 'a-1', 'embargoed', 'held text', 't', 't', '2026-12-20', NULL, 1,
              NULL, NULL, 5, NULL, NULL, NULL, NULL, NULL, NULL),
             (3, 'e-waiting', 'a-2', 'waiting_on_series', 'waiting text', 't', 't', NULL, NULL, 1,
              NULL, NULL, NULL, 1, 1, NULL, NULL, NULL, NULL),
             (4, 'e-removed-embargoed', 'a-1', 'removed', 'x', 't', 't', '2026-12-20', NULL, 1,
              NULL, NULL, NULL, NULL, NULL, 'embargoed', 't', NULL, NULL),
             (5, 'e-removed-waiting', 'a-1', 'removed', 'x', 't', 't', NULL, NULL, 1,
              NULL, NULL, NULL, NULL, NULL, 'waiting_on_series', 't', NULL, NULL),
             (6, 'e-removed-public', 'a-1', 'removed', 'x', 't', 't', NULL, 't', 1,
              NULL, NULL, NULL, NULL, NULL, 'public', 't', NULL, NULL),
             (8, 'e-rejected', 'a-2', 'rejected', 'x', 't', 't', NULL, NULL, 1,
              NULL, NULL, NULL, 1, 1, NULL, NULL, NULL, NULL)",
        // A pending new version of the embargoed entry.
        "INSERT INTO entries (seq, id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, hold_until, previous_seq) VALUES
             (7, 'e-next', 'a-1', 'pending', 'next text', 't', 't', 1, 't', 2)",
        "INSERT INTO entry_models (entry_seq, name, role) VALUES (1, 'm', 'subject'), (2, 'm', 'subject')",
        "INSERT INTO entry_tags (entry_seq, tag) VALUES (1, 'kept'), (2, 'held'), (3, 'waiting')",
        "INSERT INTO entry_rows (entry_seq, position, model, claim) VALUES (1, 1, 'm', NULL), (2, 1, 'm', NULL)",
        "INSERT INTO entry_unknown_fields (entry_seq, name) VALUES (2, 'odd')",
        "INSERT INTO entry_references (entry_seq, relation, position, key) VALUES (2, 'replicates', 1, 'k')",
        "INSERT INTO entry_runs (entry_seq, account_id, run_id) VALUES (2, 'a-1', 'r-held')",
        "INSERT INTO slugs (account_id, slug, entry_seq) VALUES ('a-1', 'held', 2), ('a-1', 'kept', 1)",
        "INSERT INTO link_checks (entry_seq, field, url, reach, checked_at) VALUES (2, 'code', 'u', 'live', 't')",
        "INSERT INTO allowance_units (account_id, day, entry_seq) VALUES ('a-1', 'd', 2), ('a-1', 'd', 1)",
        "INSERT INTO screenings (entry_seq, account_id, model, input_tokens, output_tokens, cost_nano_usd, at)
             VALUES (2, 'a-1', 'm', 1, 1, 5, 't')",
        "INSERT INTO entries_fts (rowid, title, hypothesis, safety_motivation, body)
             VALUES (1, 'Kept', 'H', 'S', 'public text')",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    pool
}

/// The one number `query`, a test's own SQL, counts.
async fn count(pool: &sqlx::SqlitePool, query: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(query.to_string()))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn migration_33_deletes_held_entries_and_unread_hidden_series_and_keeps_the_rest() {
    let pool = held_entries_at_32().await;

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM entries ORDER BY seq")
        .fetch_all(&pool)
        .await
        .unwrap();
    let ids: Vec<&str> = ids.iter().map(|(id,)| id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "e-public",
            "e-removed-embargoed",
            "e-removed-waiting",
            "e-removed-public",
            "e-rejected"
        ]
    );
    for (table, column) in [
        ("entry_models", "entry_seq"),
        ("entry_tags", "entry_seq"),
        ("entry_rows", "entry_seq"),
        ("entry_unknown_fields", "entry_seq"),
        ("entry_references", "entry_seq"),
        ("entry_runs", "entry_seq"),
        ("slugs", "entry_seq"),
        ("link_checks", "entry_seq"),
        ("allowance_units", "entry_seq"),
        ("screenings", "entry_seq"),
    ] {
        let held = count(
            &pool,
            &format!("SELECT COUNT(*) FROM {table} WHERE {column} IN (2, 3)"),
        )
        .await;
        assert_eq!(held, 0, "{table} still names a held entry");
    }
    // The spend and the allowance unit stay, with no entry, as a deletion keeps them.
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM screenings WHERE entry_seq IS NULL"
        )
        .await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM allowance_units").await,
        2
    );
    // What only the held entry used goes with it.
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM batches").await, 0);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM runs").await, 0);
    // The public entry keeps everything read from it.
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM entry_tags WHERE entry_seq = 1").await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM slugs WHERE entry_seq = 1").await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM entries_fts WHERE entries_fts MATCH 'kept'"
        )
        .await,
        1
    );
    let public: (
        String,
        String,
        String,
        Option<String>,
        i64,
        i64,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT source, submitted_at, first_submitted_at, published_at, series_seq,
                    series_version, skill, user_agent
             FROM entries WHERE seq = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        public,
        (
            "public text".into(),
            "t1".into(),
            "t0".into(),
            Some("t2".into()),
            2,
            1,
            "submit@1".into(),
            "agent/1".into()
        )
    );
    let removed_from: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, removed_from FROM entries WHERE status = 'removed' ORDER BY seq",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        removed_from,
        vec![
            ("e-removed-embargoed".into(), Some("withdrawn".into())),
            ("e-removed-waiting".into(), Some("withdrawn".into())),
            ("e-removed-public".into(), Some("public".into())),
        ]
    );
    // Only the hidden series no one could read goes, and the entries citing it lose
    // the cite; one the owner removed stays, as removed data does.
    let series: Vec<(String,)> = sqlx::query_as("SELECT id FROM series ORDER BY seq")
        .fetch_all(&pool)
        .await
        .unwrap();
    let series: Vec<&str> = series.iter().map(|(id,)| id.as_str()).collect();
    assert_eq!(series, ["s-2", "s-3", "s-4"]);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM series_versions WHERE series_seq = 1"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM series_members WHERE series_seq = 1"
        )
        .await,
        0
    );
    let rejected: (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT series_seq, series_version FROM entries WHERE id = 'e-rejected'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rejected, (None, None));
    let violations: Vec<(String,)> =
        sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(violations, vec![]);
    let fk_on: i64 = count(&pool, "PRAGMA foreign_keys").await;
    assert_eq!(fk_on, 1, "the migration left foreign keys off");
}

#[tokio::test]
async fn migration_33_rebuilds_entries_without_embargoes_and_keeps_its_rules() {
    let pool = held_entries_at_32().await;

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    for (table, column) in [("entries", "embargo_until"), ("series", "public_at")] {
        let found = count(
            &pool,
            &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = '{column}'"),
        )
        .await;
        assert_eq!(found, 0, "{table}.{column} is still there");
    }
    for index in [
        "entries_status",
        "entries_hold",
        "entries_one_successor",
        "entries_series",
    ] {
        let found = count(
            &pool,
            &format!(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name = '{index}'"
            ),
        )
        .await;
        assert_eq!(found, 1, "index {index} is gone");
    }
    let insert = |id: &'static str, account: &'static str, status: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO entries (id, account_id, status, source, submitted_at,
                     first_submitted_at, analyzer_version) VALUES (?, ?, ?, 'x', 't', 't', 1)",
            )
            .bind(id)
            .bind(account)
            .bind(status)
            .execute(&pool)
            .await
        }
    };
    for retired in ["embargoed", "waiting_on_series"] {
        assert!(insert("e-new", "a-1", retired).await.is_err(), "{retired}");
    }
    insert("e-new", "a-1", "pending").await.unwrap();
    // One live successor per entry still holds.
    sqlx::query(
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, previous_seq) VALUES ('e-v2', 'a-1', 'pending', 'x', 't', 't', 1, 1),
             ('e-v2b', 'a-1', 'rejected', 'x', 't', 't', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let second = sqlx::query(
        "INSERT INTO entries (id, account_id, status, source, submitted_at, first_submitted_at,
             analyzer_version, previous_seq) VALUES ('e-v3', 'a-1', 'pending', 'x', 't', 't', 1, 1)",
    )
    .execute(&pool)
    .await;
    assert!(second.is_err(), "a second live successor was let in");
    // A foreign key still holds.
    assert!(insert("e-nobody", "a-none", "pending").await.is_err());
    // The suspended account's trigger (migration 26) still refuses.
    sqlx::query("UPDATE accounts SET suspended_at = 't' WHERE id = 'a-2'")
        .execute(&pool)
        .await
        .unwrap();
    let error = insert("e-suspended", "a-2", "pending").await.unwrap_err();
    assert!(error.to_string().contains("suspended"), "{error}");
}

#[tokio::test]
async fn migration_44_starts_the_60_days_of_every_entry_already_rejected_now() {
    // So the first tidy after the deploy deletes none of them (#132).
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = Cow::Owned(before.migrations[..43].to_vec());
    before.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO accounts (id, display_name, created_at) VALUES ('a-1', 'Ada', 't')")
        .execute(&pool)
        .await
        .unwrap();
    for (seq, status) in [(1, "rejected"), (2, "public")] {
        sqlx::query(
            "INSERT INTO entries (seq, id, account_id, status, source, submitted_at,
                 first_submitted_at, analyzer_version, outcome, kind)
             VALUES (?1, 'e' || ?1, 'a-1', ?2, 'text', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z', 2, 'no_effect', 'experiment')",
        )
        .bind(seq)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
    }
    let started = crate::common::stamp(time::OffsetDateTime::now_utc());

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let stamped: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT status, rejected_at FROM entries ORDER BY seq")
            .fetch_all(&pool)
            .await
            .unwrap();
    let rejected_at = stamped[0]
        .1
        .as_deref()
        .expect("the rejected entry is stamped");
    assert!(rejected_at >= started.as_str(), "{rejected_at} < {started}");
    assert_eq!(stamped[1], ("public".to_string(), None));
}
