# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents. Most are null or inconclusive, and the pitch leads with those. The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: designed, not built. The owner and an agent settled the design on 2026-11-03 and 2026-11-04. There is no code yet.

## Next steps

1. **Build** in the order given in `docs/design.md`, under Build order, starting at step 2. The owner signed off on the entry format's fourth draft on 2026-11-05, after a three-round field test over the owner's five Apart repos. That test's entries and logs are in `field-test/`, git-excluded and local only; never commit or publish them without the owner's say. Do these two first:
   - **Write `CODING_STANDARDS.md`**, the rules `/build`'s reviewer holds the code to, which the repo doesn't have yet: for the Rust server (error handling, SQL only through sqlx, a test for every route) and for the Python in `bin/`. Say which docs are the spec.
   - **Build the analyzer first, as a library with a CLI** (`entry-check FILE...`): it parses an entry by `docs/entry-format.md`, renders its card with analyzer v1, and lists its warnings. The server's submission path and cards use the same code. Reason: in the field test every agent wrote its own parser and card renderer, a dozen in all, and each read the spec differently. The CLI also serves contributing agents checking an entry before they submit, and the next field test (`docs/field-test.md`).

Open work items, such as checks to run before launch, are GitHub issues on `filedrawer-dev/filedrawer`. Read them before you plan work, and file new ones there rather than in these docs.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives. Read it before you change behaviour. Reviving a rejected alternative needs the owner's agreement.
- `docs/entry-format.md`: the fields, `outcome`, the Results table and the card. Read it before you touch submission, the analyzer, cards or `/skill.md`.
- `docs/prior-art.md`: related projects, with the date of the survey. Read it before claiming novelty or adding a feature that may already exist elsewhere.
- `docs/field-test.md`: how to run a field test with agents cheaply. Read it before you brief agents to write entries, as in the usability runs on staging.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

## Checks

Committing runs the checks, so commit and read the hook's output instead of running them first. Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, and `cargo fmt --check` when a `.rs` file is staged. Agents' commits add the tests in `bin/tests`, clippy and `cargo test` (`bin/agent-pre-commit`). CI runs all of them. Link the hooks once per clone with the two `ln -sf` lines at the top of `bin/git-pre-commit`.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script. The version and hashes live only there, and its docstring says how to move to a new release. The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run` and `bin/slop-lint` are copies of llm-toolbox's tools. Update them by copying from there.
