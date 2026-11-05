# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents. Most are null or inconclusive, and the pitch leads with those. The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: designed, not built. The owner and an agent settled the design on 2026-11-03 and 2026-11-04. There is no code yet.

## Next steps

1. **Get the owner's sign-off on the fourth draft** of `docs/entry-format.md`, after a cold review of it. The field test that shaped it ran three rounds over the owner's five Apart repos; its entries and logs are in `field-test/` (git-excluded, local only, never to be committed or published without the owner's say).
2. **Build** in the order given in `docs/design.md`, under Build order.

Open work items, such as checks to run before launch, are GitHub issues on `filedrawer-dev/filedrawer`. Read them before you plan work, and file new ones there rather than in these docs.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives. Read it before you change behaviour. Reviving a rejected alternative needs the owner's agreement.
- `docs/entry-format.md`: the fields, `outcome` and the numbers check. Read it before you touch submission, outcomes or `/skill.md`.
- `docs/prior-art.md`: related projects, with the date of the survey. Read it before claiming novelty or adding a feature that may already exist elsewhere.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

## Checks

Committing runs the checks, so commit and read the hook's output instead of running them first. Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, and `cargo fmt --check` when a `.rs` file is staged. Agents' commits add the tests in `bin/tests`, clippy and `cargo test` (`bin/agent-pre-commit`). CI runs all of them. Link the hooks once per clone with the two `ln -sf` lines at the top of `bin/git-pre-commit`.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script. The version and hashes live only there, and its docstring says how to move to a new release. The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run` and `bin/slop-lint` are copies of llm-toolbox's tools. Update them by copying from there.
