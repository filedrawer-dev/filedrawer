//! The SQLite database: opening it and running the migrations.

use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

/// Opens (creating if needed) the database at `path` and runs the migrations.
/// `:memory:` gives a private in-memory database, for tests.
pub async fn open(path: &str) -> Result<SqlitePool, sqlx::Error> {
    let in_memory = path == ":memory:";
    let options = if in_memory {
        SqliteConnectOptions::from_str("sqlite::memory:")?
    } else {
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
    }
    .foreign_keys(true);
    // Every connection to `:memory:` opens a database of its own, so tests use one.
    let pool = SqlitePoolOptions::new()
        .max_connections(if in_memory { 1 } else { 8 })
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
