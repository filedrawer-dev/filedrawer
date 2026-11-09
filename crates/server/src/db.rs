//! The SQLite database: opening it and running the migrations.

use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Sqlite, SqlitePool, Transaction};

/// A transaction that holds SQLite's write lock from its first statement (`BEGIN
/// IMMEDIATE`). Every transaction is one of these: a deferred transaction that reads
/// and then writes fails with SQLITE_BUSY when another writer commits in between,
/// and two can both pass a check before either writes. clippy.toml refuses the other
/// ways to begin one.
pub async fn write_tx(pool: &SqlitePool) -> Result<Transaction<'static, Sqlite>, sqlx::Error> {
    #[allow(clippy::disallowed_methods)]
    pool.begin_with("BEGIN IMMEDIATE").await
}

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
    refresh_derived(&pool).await?;
    Ok(pool)
}

/// Rebuilds each table derived from entries' text whose format has changed since it
/// was filled, as a new analyzer's reference keys (`entries::rebuild_stale_references`).
pub async fn refresh_derived(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    crate::entries::rebuild_stale_references(pool).await
}
