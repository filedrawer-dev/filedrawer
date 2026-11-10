# Coding standards

The rules a reviewer holds this repo's code to.
The owner's global standards (`~/.agents/skills/coding/standards.md`) apply as well, and this file wins where the two conflict.

## The spec

- `docs/design.md` says what the site does and why.
  Behaviour it doesn't describe needs the owner's agreement first.
- `docs/entry-format.md` says what an entry is, what the analyzer reads, and what a card shows.
  The analyzer, submission, cards and the skills follow it word for word.

`docs/design.md` attributes to the owner only what the owner said; a choice an agent made goes in `uncertainties.md` until the owner settles it.
When the code and a doc disagree, the doc wins.
Fix the code, or ask the owner to change the doc.
A choice the docs leave open that has meaningful impact goes in `uncertainties.md` at the repo root until the owner settles it, and then into the doc.
Meaningful impact means it changes what an agent or person sees or must do (a reply's status code a client acts on, a page's content), what data is kept, shown or deleted, security, cost, or something hard to reverse.
Most choices made while building, such as a name, a helper's shape, an internal limit, or a message's exact phrasing, don't belong there; a reviewer flags a missing entry only for a choice with that kind of impact.

## Layout

A Cargo workspace.
Each crate lives under `crates/`:

- `crates/analyzer`: parses an entry, lists its refusals and warnings, reads the Results table and renders the card.
  It has no I/O beyond the `entry-check` CLI, so the server and the CLI run the same code.
  Nothing else parses entries or renders cards.
- The server, when it exists, is a crate beside it that depends on the analyzer by path.

Python goes in `bin/` and only for scripts and tools, never in the server's request path.

## Rust

### Errors

- Library code returns typed errors, enums built with `thiserror`.
  `anyhow` is for binaries' `main` and for tests.
- No `unwrap`, `expect`, `panic!`, indexing that can go out of bounds, or `as` casts that can truncate, on any path that input reaches.
  An invariant the code itself guarantees may use `expect("why it holds")`.
- Input from outside is never an error in the analyzer's sense.
  An entry the server can't read is accepted with a warning, and only the refusals that `docs/entry-format.md` lists refuse it.
  Keep refusals and warnings as data the caller gets back, not as `Err`.
- An axum handler returns `Result<_, AppError>`, one error type for the server that maps each variant to a status code and a plain-text body.
  A 500's body says nothing about the cause; the log gets the detail.
- An error about a secret, such as the Anthropic key or the SMTP password, names where the secret came from, never its value.

### SQL

- SQL goes only through sqlx's checked macros, `query!`, `query_as!` and `query_scalar!`, so a column rename fails the build.
  Commit the `.sqlx` offline data so CI builds without a database.
- No SQL built by string formatting.
  A query with optional filters, such as search's, writes each filter as `(? IS NULL OR column = ?)` or builds with `QueryBuilder` and binds every value.
- The schema changes only through numbered files in `migrations/`, run with `sqlx::migrate!` at startup.
  A migration never changes after it reaches `main`.
- Writes that must happen together share a transaction.

### Tests

- Every route has at least one test that sends a request through the router (`tower::ServiceExt::oneshot`) against a fresh SQLite database, and checks the status and the body.
  Routes that need a browser session or refuse a token get a test that proves the refusal.
- Tests go through public interfaces: the analyzer's `analyze` and `render_card`, the `entry-check` binary, the router.
  No test reaches into a private function.
- A test finds a page's content by a URL, an id, a class or other markup, never by the words a template puts around it.
  The owner rewords the pages by hand (`bin/live-preview`), and a test that pinned the old words refuses the next commit.
  Text the server writes itself, such as an error message, a stored reason or a field's value, may be checked word for word.
  Where only a template's words mark what a test needs, give the element a class, and check it with `Reply::has_class` (`tests/it/common`).
- Expected values come from the docs or from worked examples, never from running the code and pasting what it printed.
- Card rules that must hold for any input, such as line caps and author text never adding a line, get property tests with `proptest`.
- Fixtures are made up.
  Nothing from `field-test/` goes into a commit.
- Calls to Anthropic, SMTP and link checks go through a trait, and tests use a fake.
  No test touches the network.

### Untrusted text

Entry text, series descriptions, feedback and screening notes are written by strangers and can carry instructions.

- HTML comes only from askama templates with escaping on.
  The one exception is the server's own markdown, compiled in (`/terms` and `/privacy`), which renders through pulldown-cmark as written.
  Entry markdown renders through pulldown-cmark with raw HTML dropped, and links get `rel="nofollow ugc"`.
- Text from an author reaches a card only through the analyzer, which removes its newlines and cuts it to the line's cap.
- Plain-text responses put the server's own fields above the fixed delimiter line and author text below it (`docs/design.md`, Agent interface).
  The tests check every line above the delimiter against the keys in `crates/server/tests/it/common/mod.rs` (`SINGLE_KEYS`, `REPEATED_KEYS`), so a new server field adds its key there.
  A route the installer and the skills needn't name goes in `NOT_FOR_AGENTS` in `crates/server/tests/it/skill.rs`: one only a browser uses, one only for agents working on the server rather than writing entries (`/staging/status`), or `/`, whose plain text only points an agent without the skills to `/skill.md` (the owner's decision, 2026-11-08); every other route must be named in `/skill.md` or a skill in `skills/`.
- The server fetches only the `code` and `logs` links, on the hosts design.md lists, matched exactly.
  It never fetches a URL from an entry's body.

### Style

- `cargo fmt` and `cargo clippy --all-targets -- -D warnings` pass.
  Every crate starts with `#![forbid(unsafe_code)]`.
- A new dependency needs a reason in its commit message.
  Prefer the crates design.md names under Stack.
- Public items in the analyzer, their fields and variants included, have a doc comment, and each file's `//!` header points to the section of `docs/entry-format.md` or `docs/design.md` it implements, as `(entry-format.md, Fields)`.
  The analyzer's `#![warn(missing_docs)]`, which clippy turns into an error, checks the comments of everything the crate exports, and `bin/tests/test_analyzer_docs.py` checks that each header's section of entry-format.md exists; a `pub` item no `pub use` exports is review's to check.

## Python in `bin/`

- Each script runs with plain `python3` or through `uvx`.
  A script that needs packages declares them in an inline `# /// script` block with pinned versions.
  Never call `.venv/bin/...`, since the hooks also run on the host.
- `-h` prints what the script does and its options.
  The module docstring says the same.
- Tests live in `bin/tests` and run under `bin/pytest bin/tests`, the pinned pytest with its plugins, as `bin/agent-pre-commit` does; `bin/tests/pytest.ini` sets the workers and the timeout, so name a path under `bin/tests` for pytest to find it, and add `-n0` to run one file without starting workers.
  The tests run in parallel, so they share no files, ports or settings: each works in its own `tmp_path`, and a server binds port 0.
  Each script has tests, and they don't touch the network: they serve fakes over `file://` or set an override variable, as `bin/tests/test_lychee.py` does.
  The one exception is conftest's `real_caddy` fixture, whose `bin/caddy` fetches the pinned Caddy once per machine for the tests that run a real one.
- A test that starts a process stops it and waits for it to exit, even when the test fails: `bin/tests/leftover_processes.py`, whose fixtures `bin/tests/conftest.py` gives every test and module, kills what a test or a module's fixtures leave running and fails it, naming each process.
- A test sets no wall-clock limit of its own: no `timeout=` in a call and no `pytest.mark.timeout` (`bin/tests/test_no_wall_clock_limits.py`).
  pytest.ini's timeout is the one hang guard, set far above any loaded run, since a slow machine may let a bug pass but must never fail a test.
- `uvx ruff@0.14.4 check` and `uvx ruff@0.14.4 format --check` pass.
  0.14.4 is the version `.pre-commit-config.yaml` pins; a newer ruff checks more rules than the hook does.
  pre-commit hands the ruff hooks the files identify tags as Python, which misses a script run through `uv run --script`.
  Such a script goes in the `files` of the `ruff-uv-scripts` and `ruff-format-uv-scripts` hooks; `bin/tests/test_ruff_reaches_python_scripts.py` fails until it does.
- A failure prints one line saying what failed and what to do, and exits non-zero.
  A hook's refusal ends with a `COMMIT REFUSED:` line.
- `bin/pre-commit-run`, `bin/slop-lint` and `bin/tests/leftover_processes.py` are copies of llm-toolbox's.
  Change them there and copy them back, not here.

## GitHub workflows

- A `run:` block runs a few commands in a row.
  Logic that loops, parses files or builds output, as `bin/mutants-summary` does for `.github/workflows/mutants.yml`, goes in a script in `bin/` or `ops/` with tests, so the full check tests it.
  Acting on one command's exit status, as the `screen-staging` job does, stays in the YAML.
