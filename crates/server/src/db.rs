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

/// Moves everything in the write-ahead log into the database file and empties the
/// log. Reason: the log keeps the pages a deleted row was written in until it is
/// next rewound, so text deleted from the database would stay on disk beside it.
/// Says whether the log was emptied: while a reader holds an older view of the
/// database, the checkpoint waits out the connection's busy timeout, holding up
/// writes meanwhile, and then leaves the log for the next call.
pub async fn trim_log(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    // Not a checked macro: the pragma names no table or column for one to check,
    // and its three counts come back untyped, which the macro refuses.
    let (busy, _, _): (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(pool)
        .await?;
    Ok(busy == 0)
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
    .foreign_keys(true)
    // A deleted row's bytes are overwritten with zeros, not left in the freed
    // pages, where a copy of the file would still hold them (design.md, Admin).
    .pragma("secure_delete", "ON");
    // Every connection to `:memory:` opens a database of its own, so tests use one.
    let pool = SqlitePoolOptions::new()
        .max_connections(if in_memory { 1 } else { 8 })
        .connect_with(options)
        .await?;
    // A rollback runs an older binary on the schema a newer one migrated, so the
    // migrations it doesn't know don't stop it; a known one whose text changed still
    // does (design.md, Hosting and operations).
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(&pool).await?;
    refresh_derived(&pool).await?;
    Ok(pool)
}

/// Rebuilds each table derived from entries' text whose format has changed since it
/// was filled, as a new analyzer's reference keys (`entries::rebuild_stale_references`).
pub async fn refresh_derived(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    crate::entries::rebuild_stale_references(pool).await
}
