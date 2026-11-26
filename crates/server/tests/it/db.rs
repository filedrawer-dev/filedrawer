//! Seams: `db::with_deadline`, the time limit on a connection's statements, and
//! `db::open_image`, a database loaded from a serialized one.

use std::time::Duration;

use filedrawer_server::db;
use sqlx::Connection;

/// Counts to `to` in SQL, one row a step, which no index shortens.
async fn count_to(connection: &mut sqlx::SqliteConnection, to: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?)
         SELECT COUNT(*) FROM n",
    )
    .bind(to)
    .fetch_one(connection)
    .await
}

#[tokio::test]
async fn a_statement_past_its_deadline_is_stopped_and_the_next_user_of_the_connection_has_none() {
    let pool = db::open(":memory:").await.unwrap();
    {
        let mut limited = db::with_deadline(&pool, Duration::ZERO).await.unwrap();
        let stopped = count_to(&mut limited, 50_000_000).await;
        let error = stopped.expect_err("a statement ran past its deadline");
        assert!(db::timed_out(&error), "{error}");
    }
    // The pool has one connection, so this is the one that carried the deadline.
    let mut plain = pool.acquire().await.unwrap();
    assert_eq!(count_to(&mut plain, 200_000).await.unwrap(), 200_000);
}

#[tokio::test]
async fn a_statement_inside_its_deadline_runs_to_its_end() {
    let pool = db::open(":memory:").await.unwrap();
    let mut limited = db::with_deadline(&pool, Duration::from_secs(3600))
        .await
        .unwrap();
    assert_eq!(count_to(&mut limited, 200_000).await.unwrap(), 200_000);
}

/// The image of a database `db::open` made, holding one account.
async fn image_with_an_account() -> (Vec<u8>, String) {
    let pool = db::open(":memory:").await.unwrap();
    let account = filedrawer_server::admin::create_account(&pool, "Ada")
        .await
        .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    let image = connection.serialize(None).await.unwrap().to_vec();
    (image, account)
}

#[tokio::test]
async fn a_database_from_an_image_holds_what_the_image_did_with_every_migration_recorded() {
    let (image, account) = image_with_an_account().await;
    let pool = db::open_image(&image).await.unwrap();
    // A token needs its account, so this works only on the image's data.
    filedrawer_server::admin::create_token(&pool, &account, "laptop")
        .await
        .unwrap();
    let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    let migrations = sqlx::migrate!("./migrations").iter().count();
    assert_eq!(usize::try_from(recorded).unwrap(), migrations);
}

#[tokio::test]
async fn a_database_from_an_image_enforces_foreign_keys() {
    let (image, _) = image_with_an_account().await;
    let pool = db::open_image(&image).await.unwrap();
    let orphan = sqlx::query(
        "INSERT INTO sessions (hash, account_id, created_at, expires_at)
         VALUES (x'00', 'no-such-account', '', '')",
    )
    .execute(&pool)
    .await;
    let error = orphan.expect_err("a session for no account was stored");
    assert!(error.to_string().contains("FOREIGN KEY"), "{error}");
}

#[tokio::test]
async fn two_databases_from_one_image_are_apart() {
    let (image, _) = image_with_an_account().await;
    let one = db::open_image(&image).await.unwrap();
    let other = db::open_image(&image).await.unwrap();
    filedrawer_server::admin::create_account(&one, "Grace")
        .await
        .unwrap();
    let count = |pool: sqlx::SqlitePool| async move {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    assert_eq!(count(one).await, 2);
    assert_eq!(count(other).await, 1);
}

#[tokio::test]
async fn an_image_of_an_empty_database_comes_out_migrated() {
    let mut blank = sqlx::SqliteConnection::connect("sqlite::memory:")
        .await
        .unwrap();
    let image = blank.serialize(None).await.unwrap().to_vec();
    let pool = db::open_image(&image).await.unwrap();
    let account = filedrawer_server::admin::create_account(&pool, "Ada")
        .await
        .unwrap();
    filedrawer_server::admin::create_token(&pool, &account, "laptop")
        .await
        .unwrap();
}

/// The tests' databases come from a template that the first one leaves on disk, so
/// the rest skip the migrations.
#[tokio::test]
async fn a_fresh_test_database_leaves_its_template_for_the_next() {
    crate::common::template::fresh_pool().await;
    let image = std::fs::read(crate::common::template::template_path()).unwrap();
    db::open_image(&image).await.unwrap();
}

#[tokio::test]
async fn an_image_that_isnt_a_database_is_an_error() {
    assert!(
        db::open_image(b"not a database, just some bytes")
            .await
            .is_err()
    );
    assert!(db::open_image(b"").await.is_err());
}
