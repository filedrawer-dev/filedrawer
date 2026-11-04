# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents. Most are null or inconclusive, and the pitch leads with those. The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: designed, not built. The owner and an agent settled the design on 2026-11-03 and 2026-11-04. There is no code yet.

## Next steps

1. **Build the pinned lychee hook.** The owner chose this on 2026-11-04. Replace the hook from lychee's own pre-commit repo, which installs cargo-binstall by piping an unpinned script into bash, with a local hook script. It downloads the lychee v0.24.2 release archive for the machine's platform, checks it against a SHA-256 kept in the repo, and caches it. Each release asset has a `.sha256` file beside it. Copy those values into the repo, so the check doesn't trust the download host. Done when the script handles Linux (x86_64, aarch64) and macOS (Arm, Intel), the hooks are linked (see [Checks](#checks)), and a commit runs lychee through the script.
2. **Field test the entry format.** Before writing any server code, use `docs/entry-format.md` to write entries as markdown files under `field-test/`. Take them from the owner's Apart sprint work: `../apart-collusion-sprint`, `../apart-oneshot-collusion`, `../apart-research-collusion-sprint-2026`, `../watched-coalitions` and `../action-channels`. Read those repos without changing them. Keep the entries local until the owner says otherwise.
   - Done when every experiment there that didn't become a paper has an entry, or a line under "Field test findings" in `docs/entry-format.md` saying why it doesn't fit. That covers nulls, inconclusive results and unrefined effects.
   - Record every place the format fought you in that section. Then propose format changes to the owner, revise, and run the test again.
3. **Resolve the field test's format questions** with the owner (open question 1 in `docs/design.md`).
4. **Build** in the order given in `docs/design.md`, under Build order.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives. Read it before you change behaviour. Reviving a rejected alternative needs the owner's agreement.
- `docs/entry-format.md`: the fields and the label rule. Read it before you touch submission, labels or `/skill.md`.
- `docs/prior-art.md`: related projects, with the date of the survey. Read it before claiming novelty or adding a feature that may already exist elsewhere.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

## Checks

Committing runs the checks, so commit and read the hook's output instead of running them first. Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, and `cargo fmt --check` when a `.rs` file is staged. Agents' commits add clippy and `cargo test` (`bin/agent-pre-commit`). CI runs all of them, with lychee from `lycheeverse/lychee-action` pinned to v0.24.2.

Link the hooks once per clone with the two `ln -sf` lines at the top of `bin/git-pre-commit`, once next step 1 is done. `.git/hooks` is shared with the owner's computer, and the current lychee hook would run the unpinned install there. Until then no hook runs, so run `uvx pre-commit@4.7.0 run --all-files` in the sandbox before committing. The sandbox's pre-commit cache already holds lychee, so that installs nothing.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run` and `bin/slop-lint` are copies of llm-toolbox's tools. Update them by copying from there.
