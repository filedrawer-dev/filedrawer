//! Seam: `db::with_deadline`, the time limit on a connection's statements.

use std::time::Duration;

use filedrawer_server::db;

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
