//! Seam: `search::terms`, held to the full-text index's own tokenizer, the
//! `unicode61` of the SQLite the server links, over every code point.

use filedrawer_server::search;
use sqlx::{Connection, SqliteConnection};

/// `entries_fts`'s tokenizer less `porter`, which stems each token `unicode61`
/// gives it and never splits one.
const AS_THE_INDEX: &str = "CREATE VIRTUAL TABLE t USING fts5 (x, tokenize = 'unicode61')";
/// The same, keeping accents.
const KEEPING_ACCENTS: &str =
    "CREATE VIRTUAL TABLE t USING fts5 (x, tokenize = 'unicode61 remove_diacritics 0')";

/// A table made by `create`, and its instance vocabulary, which lists every token
/// with its document and position.
async fn tokenizer(create: &'static str) -> SqliteConnection {
    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    for statement in [
        create,
        "CREATE VIRTUAL TABLE v USING fts5vocab (t, instance)",
    ] {
        sqlx::query(statement)
            .execute(&mut connection)
            .await
            .unwrap();
    }
    connection
}

/// SQLite's tokens of each document, in order.
async fn tokens_of(connection: &mut SqliteConnection, documents: &[String]) -> Vec<Vec<String>> {
    sqlx::query("DELETE FROM t")
        .execute(&mut *connection)
        .await
        .unwrap();
    for (rowid, document) in documents.iter().enumerate() {
        sqlx::query("INSERT INTO t (rowid, x) VALUES (?, ?)")
            .bind(i64::try_from(rowid).unwrap())
            .bind(document)
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    let rows: Vec<(i64, String)> = sqlx::query_as("SELECT doc, term FROM v ORDER BY doc, offset")
        .fetch_all(&mut *connection)
        .await
        .unwrap();
    let mut tokens = vec![Vec::new(); documents.len()];
    for (doc, term) in rows {
        tokens[usize::try_from(doc).unwrap()].push(term);
    }
    tokens
}

/// Each code point between two letters either joins them in one term or splits
/// them in two, and `search::terms` says which as SQLite does. A failure here after
/// a SQLite upgrade means its tables changed: regenerate the server's from them,
/// and rebuild the index (`INSERT INTO entries_fts(entries_fts) VALUES('rebuild')`
/// in a migration), since entries indexed under the old tables split differently.
#[tokio::test]
async fn every_code_point_splits_a_word_as_the_index_does() {
    let mut connection = tokenizer(AS_THE_INDEX).await;
    let every: Vec<char> = (0..=u32::from(char::MAX))
        .filter_map(char::from_u32)
        .collect();
    let mut differ = Vec::new();
    for chunk in every.chunks(20_000) {
        // One document per thousand code points, each between `x` and `y`.
        let documents: Vec<String> = chunk
            .chunks(1000)
            .map(|some| {
                some.iter()
                    .map(|c| format!("x{c}y"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let tokens = tokens_of(&mut connection, &documents).await;
        for (some, tokens) in chunk.chunks(1000).zip(tokens) {
            let mut tokens = tokens.into_iter();
            for &c in some {
                let joined = match tokens.next().as_deref() {
                    Some("x") => {
                        assert_eq!(
                            tokens.next().as_deref(),
                            Some("y"),
                            "after U+{:04X}",
                            u32::from(c)
                        );
                        false
                    }
                    Some(_) => true,
                    None => panic!("SQLite ran out of tokens at U+{:04X}", u32::from(c)),
                };
                let ours = search::terms(&format!("x{c}y")).len() == 1;
                if ours != joined {
                    differ.push(format!("U+{:04X}", u32::from(c)));
                }
            }
            assert_eq!(tokens.next(), None);
        }
    }
    assert!(
        differ.is_empty(),
        "{} code points split differently, first {:?}",
        differ.len(),
        &differ[..differ.len().min(20)]
    );
}

/// A combining mark that SQLite keeps inside a term never starts one. The table
/// keeps accents here, so a term that started with the mark would show it.
#[tokio::test]
async fn a_mark_that_continues_a_term_starts_none() {
    let mut connection = tokenizer(KEEPING_ACCENTS).await;
    let marks: Vec<char> = ('\u{300}'..='\u{36f}').collect();
    let documents: Vec<String> = marks.iter().map(|c| format!("{c}y")).collect();
    let tokens = tokens_of(&mut connection, &documents).await;
    for (c, tokens) in marks.iter().zip(tokens) {
        assert_eq!(
            search::terms(&format!("{c}y")),
            tokens,
            "U+{:04X}",
            u32::from(*c)
        );
    }
}

#[test]
fn a_word_repeated_in_any_case_is_one_term() {
    assert_eq!(
        search::terms("Probe probe PROBE, probes"),
        ["Probe", "probes"]
    );
}
