# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents. Most are null or inconclusive, and the pitch leads with those. The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: designed, not built. The owner and an agent settled the design on 2026-11-03 and 2026-11-04. There is no code yet.

## Next steps

1. **Field test the entry format.** Before writing any server code, use `docs/entry-format.md` to write entries as markdown files under `field-test/`. Take them from the owner's Apart sprint work: `../apart-collusion-sprint`, `../apart-oneshot-collusion`, `../apart-research-collusion-sprint-2026`, `../watched-coalitions` and `../action-channels`. Read those repos without changing them. Keep the entries local until the owner says otherwise.
   - Done when every experiment there that didn't become a paper has an entry, or a line under "Field test findings" in `docs/entry-format.md` saying why it doesn't fit. That covers nulls, inconclusive results and unrefined effects.
   - Record every place the format fought you in that section. Then propose format changes to the owner, revise, and run the test again.
2. **Resolve the field test's format questions** with the owner (open question 1 in `docs/design.md`).
3. **Build** in the order given in `docs/design.md`, under Build order.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives. Read it before you change behaviour. Reviving a rejected alternative needs the owner's agreement.
- `docs/entry-format.md`: the fields, `outcome` and the numbers check. Read it before you touch submission, outcomes or `/skill.md`.
- `docs/prior-art.md`: related projects, with the date of the survey. Read it before claiming novelty or adding a feature that may already exist elsewhere.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

## Checks

Committing runs the checks, so commit and read the hook's output instead of running them first. Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, and `cargo fmt --check` when a `.rs` file is staged. Agents' commits add the tests in `bin/tests`, clippy and `cargo test` (`bin/agent-pre-commit`). CI runs all of them. Link the hooks once per clone with the two `ln -sf` lines at the top of `bin/git-pre-commit`.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script. The version and hashes live only there, and its docstring says how to move to a new release. The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run` and `bin/slop-lint` are copies of llm-toolbox's tools. Update them by copying from there.
