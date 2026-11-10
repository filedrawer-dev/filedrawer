//! Seam: the migration files and `migration-rollback.txt`, read as text. An older
//! binary runs on a newer schema after a rollback (design.md, Hosting and operations),
//! so a migration may only add what an older binary can ignore, unless
//! `migration-rollback.txt` says whether it is safe anyway or breaks a rollback
//! (CODING_STANDARDS.md, SQL).

use std::collections::{BTreeMap, BTreeSet};

/// Migrations up to this version ran before production's first deploy, so no rollback
/// crosses them.
const LAST_BEFORE_PRODUCTION: u32 = 31;

const SIDECAR: &str = include_str!("../../migration-rollback.txt");

fn migration_files() -> BTreeMap<u32, (String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .map(|path| {
            let name = path.file_name().unwrap().to_str().unwrap().to_string();
            let version = name.split('_').next().unwrap().parse().unwrap();
            (version, (name, std::fs::read_to_string(&path).unwrap()))
        })
        .collect()
}

#[test]
fn each_migration_an_older_binary_cant_ignore_says_whether_it_breaks_a_rollback() {
    let found = problems(&migration_files(), SIDECAR);
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// What's wrong with these migration files, by version, and this sidecar.
fn problems(files: &BTreeMap<u32, (String, String)>, sidecar: &str) -> Vec<String> {
    let lines = match rollback_lines(sidecar) {
        Ok(lines) => lines,
        Err(problem) => return vec![format!("migration-rollback.txt: {problem}")],
    };
    let mut problems = Vec::new();
    for version in lines.keys() {
        if !files.contains_key(version) {
            problems.push(format!(
                "migration-rollback.txt names {version}, which has no migration file"
            ));
        }
    }
    for (version, (name, sql)) in files.range(LAST_BEFORE_PRODUCTION + 1..) {
        let statements = needs_a_line(sql);
        if !statements.is_empty() && !lines.contains_key(version) {
            problems.push(format!(
                "{name}: an older binary can't ignore {statements:?}. Say in \
                 crates/server/migration-rollback.txt whether that breaks a rollback, as \
                 `{version} safe <why>` or `{version} breaks <why>` \
                 (CODING_STANDARDS.md, SQL)"
            ));
        }
    }
    problems
}

/// The sidecar's lines by version: `<version> safe|breaks <why>`, with `#` comments
/// and blank lines skipped.
fn rollback_lines(text: &str) -> Result<BTreeMap<u32, bool>, String> {
    let mut lines = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.splitn(3, ' ');
        let version = words.next().and_then(|v| v.parse::<u32>().ok());
        let breaks = match words.next() {
            Some("safe") => false,
            Some("breaks") => true,
            _ => return Err(format!("{line:?} isn't `<version> safe|breaks <why>`")),
        };
        let why = words.next().unwrap_or("").trim();
        match version {
            Some(version) if !why.is_empty() => {
                if lines.insert(version, breaks).is_some() {
                    return Err(format!("{version} has two lines"));
                }
            }
            _ => return Err(format!("{line:?} isn't `<version> safe|breaks <why>`")),
        }
    }
    Ok(lines)
}

/// The statements of `sql` that an older binary can't simply ignore: everything but
/// `CREATE TABLE`, a non-unique `CREATE INDEX`, and `ALTER TABLE ... ADD COLUMN`
/// that is nullable or has a default, and among those, a reference to a table the
/// migration didn't create that doesn't say what a delete does (`foreign_keys` is on,
/// so an older binary's deletes would fail).
fn needs_a_line(sql: &str) -> Vec<String> {
    let statements = statements(sql);
    let created: BTreeSet<String> = statements
        .iter()
        .filter_map(|tokens| created_table(tokens))
        .collect();
    statements
        .iter()
        .filter(|tokens| !ignorable(tokens, &created))
        .map(|tokens| tokens.join(" "))
        .collect()
}

fn ignorable(tokens: &[String], created: &BTreeSet<String>) -> bool {
    let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let kind_ok = match words.as_slice() {
        ["CREATE", "TABLE", ..] | ["CREATE", "INDEX", ..] => true,
        ["ALTER", "TABLE", _, "ADD", ..] => {
            !contains(&words, &["NOT", "NULL"]) || words.contains(&"DEFAULT")
        }
        _ => false,
    };
    kind_ok && references_say_what_a_delete_does(&words, created)
}

fn created_table(tokens: &[String]) -> Option<String> {
    let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["CREATE", "TABLE", "IF", "NOT", "EXISTS", name, ..] | ["CREATE", "TABLE", name, ..] => {
            Some((*name).to_string())
        }
        _ => None,
    }
}

fn contains(words: &[&str], run: &[&str]) -> bool {
    words.windows(run.len()).any(|window| window == run)
}

/// Each `REFERENCES t` to a table not in `created` is followed, within its column's
/// clause, by `ON DELETE CASCADE`. (`SET NULL` would fail on a NOT NULL column.)
fn references_say_what_a_delete_does(words: &[&str], created: &BTreeSet<String>) -> bool {
    words
        .iter()
        .enumerate()
        .filter(|(_, word)| **word == "REFERENCES")
        .all(|(at, _)| {
            let Some(table) = words.get(at + 1) else {
                return false;
            };
            if created.contains(*table) {
                return true;
            }
            let mut rest = words[at + 2..].iter().copied().peekable();
            // The referenced columns, in parentheses.
            if rest.peek() == Some(&"(") {
                rest.by_ref().take_while(|word| *word != ")").for_each(drop);
            }
            let clause: Vec<&str> = rest
                .take_while(|word| *word != "," && *word != ")")
                .collect();
            contains(&clause, &["ON", "DELETE", "CASCADE"])
        })
}

/// `sql`'s statements, each as uppercased tokens, with comments and string contents
/// removed and punctuation as tokens of its own.
fn statements(sql: &str) -> Vec<Vec<String>> {
    let mut statements = vec![Vec::new()];
    let mut word = String::new();
    let mut chars = sql.chars().peekable();
    let end_word = |word: &mut String, statements: &mut Vec<Vec<String>>| {
        if !word.is_empty() {
            statements.last_mut().unwrap().push(word.to_uppercase());
            word.clear();
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '-' if chars.peek() == Some(&'-') => {
                end_word(&mut word, &mut statements);
                chars.by_ref().take_while(|c| *c != '\n').for_each(drop);
            }
            '/' if chars.peek() == Some(&'*') => {
                end_word(&mut word, &mut statements);
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            '"' | '`' | '[' => {
                // A quoted name is the name: "accounts" is accounts.
                end_word(&mut word, &mut statements);
                let close = if c == '[' { ']' } else { c };
                let name: String = chars.by_ref().take_while(|c| *c != close).collect();
                statements.last_mut().unwrap().push(name.to_uppercase());
            }
            '\'' => {
                end_word(&mut word, &mut statements);
                // A doubled quote stays inside the string; contents don't matter.
                let quote = c;
                loop {
                    match chars.next() {
                        Some(c) if c == quote => {
                            if chars.peek() == Some(&quote) {
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
                statements
                    .last_mut()
                    .unwrap()
                    .push(format!("{quote}{quote}"));
            }
            ';' => {
                end_word(&mut word, &mut statements);
                statements.push(Vec::new());
            }
            c if c.is_alphanumeric() || c == '_' => word.push(c),
            c if c.is_whitespace() => end_word(&mut word, &mut statements),
            c => {
                end_word(&mut word, &mut statements);
                statements.last_mut().unwrap().push(c.to_string());
            }
        }
    }
    end_word(&mut word, &mut statements);
    statements.retain(|tokens| !tokens.is_empty());
    statements
}

#[test]
fn what_an_older_binary_can_ignore_needs_no_line() {
    let sql = "
        -- A comment that says DROP TABLE entries; or 'UPDATE'.
        CREATE TABLE notes (
            seq INTEGER PRIMARY KEY,
            entry_seq INTEGER NOT NULL REFERENCES entries (seq) ON DELETE CASCADE,
            note_seq INTEGER REFERENCES notes (seq),
            body TEXT NOT NULL CHECK (body <> 'DROP')
        ) STRICT;
        CREATE INDEX notes_entry ON notes (entry_seq) WHERE body IS NOT NULL;
        ALTER TABLE entries ADD COLUMN note TEXT;
        ALTER TABLE entries ADD COLUMN tries INTEGER NOT NULL DEFAULT 0;
        /* DELETE FROM entries; */
    ";
    assert_eq!(needs_a_line(sql), Vec::<String>::new());
}

#[test]
fn what_an_older_binary_cant_ignore_each_needs_a_line() {
    for sql in [
        "DROP INDEX link_checks_flagged;",
        "DROP TABLE entries;",
        "ALTER TABLE link_checks_new RENAME TO link_checks;",
        "ALTER TABLE files DROP COLUMN text;",
        "CREATE UNIQUE INDEX one_slug ON slugs (name);",
        "CREATE VIEW authors AS SELECT id FROM accounts;",
        "CREATE VIRTUAL TABLE entries_fts USING fts5 (title);",
        "CREATE TRIGGER no_edits BEFORE UPDATE ON entries BEGIN SELECT RAISE(ABORT, 'no'); END;",
        "UPDATE entries SET kind = 'experiment';",
        "DELETE FROM link_checks;",
        "INSERT INTO derived_versions (name, version) VALUES ('x', 1);",
        "CREATE TABLE notes (entry_seq INTEGER REFERENCES entries (seq));",
        "ALTER TABLE notes ADD COLUMN account_id TEXT REFERENCES accounts;",
        // The older binary's delete of the account fails on a NOT NULL column.
        "CREATE TABLE notes (account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE SET NULL);",
        // Quoted names are names: "accounts" is no table this migration made.
        "CREATE TABLE \"notes\" (x INTEGER); CREATE TABLE b (a TEXT REFERENCES \"accounts\" (id));",
    ] {
        assert!(!needs_a_line(sql).is_empty(), "{sql}");
    }
}

#[test]
fn only_migrations_after_productions_first_need_a_line() {
    let files = BTreeMap::from([
        (
            31,
            ("0031_old.sql".to_string(), "DROP INDEX x;".to_string()),
        ),
        (
            32,
            ("0032_new.sql".to_string(), "DROP INDEX y;".to_string()),
        ),
        (
            33,
            (
                "0033_add.sql".to_string(),
                "ALTER TABLE e ADD COLUMN n TEXT;".to_string(),
            ),
        ),
    ]);
    let found = problems(&files, "");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("0032_new.sql:"), "{found:?}");
    assert_eq!(
        problems(&files, "32 safe drops an index nothing reads\n"),
        Vec::<String>::new()
    );
}

#[test]
fn a_line_for_a_migration_that_doesnt_exist_is_refused() {
    let found = problems(&BTreeMap::new(), "40 breaks a typo for 41\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("names 40"), "{found:?}");
}

#[test]
fn the_rollback_lines_say_safe_or_breaks_and_why() {
    let lines = rollback_lines(
        "# Comment\n\n32 safe rebuilds link_checks with every column it had\n33 breaks adds a status value\n",
    )
    .unwrap();
    assert_eq!(lines, BTreeMap::from([(32, false), (33, true)]));
    for bad in [
        "32 safe",
        "32 maybe why",
        "x safe why",
        "32 safe a\n32 breaks b",
    ] {
        assert!(rollback_lines(bad).is_err(), "{bad:?}");
    }
}
