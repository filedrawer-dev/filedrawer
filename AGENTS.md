# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents.
Most are null or inconclusive, and the pitch leads with those.
The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: building, in the order of `docs/design.md`'s Build order.
The owner and an agent settled the design on 2026-11-03 and 2026-11-04, and the owner signed off on the entry format's fourth draft on 2026-11-05.

## Open work

Plans and open work are GitHub issues on `filedrawer-dev/filedrawer`, not these docs: `issue-todo open --repo filedrawer-dev/filedrawer` lists every open item, and each build step's plan is an issue of its own (#16 for step 4).
Read them before you plan work, and file new work there.
To check whether an issue already covers something, `issue-todo open --titles` lists the open issues one a line, by number and title.
When a commit finishes an issue, end its message with `Closes #N`, so the owner's push closes it; when a slice of a plan lands, tick it with `issue-todo tick N MATCH --note <sha>`.
`uncertainties.md` holds the choices made while building that the owner hasn't settled yet.

The owner has root on the VPS but keeps SSH keys out of the sandbox.
Anything done on the server is a tool the owner runs from their own machine, such as `ops/provision-vps`.

## Deploys

A push to `main` that passes CI deploys to staging (`build-staging` and `deploy-staging` in `.github/workflows/ci.yml`).
The build job makes a static x86_64 musl binary; the deploy job, which runs no third-party action, pipes it over SSH as the `deploy` user.
That user's key may only run `ops/remote/filedrawer-receive`, which allows one deploy at a time and passes the upload on stdin to `ops/remote/filedrawer-install` through one sudo rule.
The install script keeps its own root-owned copy, checks the size and hash, runs the binary's `-h` as the service user with a timeout, keeps the previous binary, and rolls back if `/tags` doesn't answer within 30 s.
`ops/provision-vps` sets up the user and key, stores `DEPLOY_SSH_KEY` as a secret of the `staging` environment (main only), and writes the variables `VPS_HOST` and `VPS_KNOWN_HOSTS`; rerunning it replaces the key.
A binary that answers `/tags` passes, and it runs as `filedrawer` with staging's data and settings.
Production deploys come at launch.

The field test's entries and logs are in `field-test/`, git-excluded and local only; never commit or publish them without the owner's say.

## Code

- `crates/analyzer`: reads an entry by `docs/entry-format.md`, lists what would refuse it and what the server couldn't read, and renders its card with analyzer v1.
  The server and the CLI share it.
  Reason: in the field test every agent wrote its own parser and card renderer, a dozen in all, and each read the spec differently.
- `entry-check FILE...` checks entries offline, for contributing agents and field tests: `cargo run -q --bin entry-check -- FILE...`.
  It exits 1 when the server would refuse an entry.
  Run `bin/field-test-diff save` before an analyzer change and `bin/field-test-diff` after it (`-h`): it lists the field-test entries whose outcome changed, without their contents.
- `crates/server`: the `filedrawer` binary.
  `filedrawer -h` lists its commands and settings.
  Production refuses to start with the pass-all screener, without SMTP settings, or without `FILEDRAWER_GITHUB_TOKEN`, which the link checks call GitHub's API with (staging shares it: `ops/provision-vps github`); email goes to the log when no SMTP settings are set.
  `crates/server/skill.md` is what agents read at `/skill.md`.
  It describes only what the server does, so update it with each change to a route.
  `CODING_STANDARDS.md` holds the rules the review applies.
- `crates/server/templates`: the HTML pages, askama templates.
  A template can't reach a field from inside a macro call, so `matches!(file.state, ...)` fails to compile; give the type a method instead.
- `ops/smoke-staging` follows `/skill.md` against staging as an agent would; `screen` checks staging's real screen and `eval` its labelled cases (`ops/screen-eval/`).
  CI runs the first two after each deploy, each as a test account of its own.
  Read its `-h` before running it, or changing staging's test accounts, their settings or CI's deploy checks: it holds the accounts, the secrets and the settings each mode reads.
- `GET /staging/status` (staging and dev; production has no such route) shows what screening spent today against the cap, how many entries wait for it and since when, and when its last call succeeded and when the last one failed, with the API's error; and for the link checks, GitHub's hourly allowance and the calls left, the hosts that asked the server to wait, and how often the API overruled GitHub's commit page.
  Look there first when staging's screen or link checks seem stuck: `curl -H "X-Staging-Key: $FILEDRAWER_STAGING_KEY" https://staging.filedrawer.dev/staging/status`.
  The last calls are kept in memory, so a restart empties them until the next one.

### SQL

Queries go through sqlx's checked macros (`CODING_STANDARDS.md`), which compile against `.sqlx/`, the committed query data, when `DATABASE_URL` is unset, as in CI and the commit hook: `bin/agent-pre-commit` builds a copy of the index with no `.env`, so a stale `.sqlx/` refuses the commit.
After a merge, checkout or rebase brings new migrations, the git hooks rebuild `target/dev.db` (`bin/sqlx-refresh --if-stale`; `python3 install.py` links them).
After adding or changing a query or a migration, run `bin/sqlx-refresh`.
It rebuilds `target/dev.db` from the migrations (writing `.env` if it's missing), so an edited, unpushed migration needs no cleanup, then runs `cargo sqlx prepare --workspace`.

The macros can't describe table-valued functions such as `json_each`: the build fails with "no such table column: json_each.value".
Query once per value instead, or join a real table.

`sqlx` is sqlx-cli: `cargo install sqlx-cli --no-default-features --features sqlite,rustls --locked`.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives.
  Read it before you change behaviour.
  Reviving a rejected alternative needs the owner's agreement.
- `docs/entry-format.md`: the fields, `outcome`, the Results table and the card.
  Read it before you touch submission, the analyzer, cards or `/skill.md`.
- `docs/prior-art.md`: related projects, with the date of the survey.
  Read it before claiming novelty or adding a feature that may already exist elsewhere.
- `docs/field-test.md`: how to run a field test with agents cheaply.
  Read it before you brief agents to write entries, as in the usability runs on staging.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

Date a decision by the owner's local date, as `date +%F` prints it in the sandbox (America/Los_Angeles), not UTC: git, GitHub and the server's stored times are UTC and run a day ahead each evening.

Markdown here breaks after each sentence end, so grep, diffs and merge conflicts show sentences rather than whole paragraphs.
The commit hook's `bin/sentence-lines` keeps it that way and leaves entries as their authors wrote them: the analyzer's fixtures, `ops/screen-eval/` and `ops/smoke-fixture.md`.

## Checks

Committing runs the checks, so commit and read the hook's output instead of running them first.
Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, actionlint with shellcheck (when a workflow is staged), and `bin/sentence-lines` on staged markdown, ruff, and `cargo fmt` (when a `.rs` file is staged), which write: a person's commit that needs formatting is refused with the files rewritten, to stage, and `bin/pre-commit-run` stages an agent's and checks again.
Agents' commits add the tests in `bin/tests`, clippy and the Rust tests (`bin/agent-pre-commit`), each only when the change needs it.
The change is what differs from `$CHECK_BASE`, which llm-toolbox's `agent-commit-gate` sets to the nearest commit that passed them: at a commit, usually HEAD, so what's staged; on a recheck by wt-ship or `build-state finish`, the commit it was rebased or folded onto.
The Rust checks run for a build input: anything in `crates/`, the Cargo files, `.sqlx/`, `.cargo/` and `.config/`, and the files the crates embed or their tests read, such as `docs/entry-format.md` and `ops/smoke-staging`.
`bin/tests` runs for a change in `bin/`, `ops/` or a crate (one of its tests reads the crates' references), or to a file those tests read, such as `.pre-commit-config.yaml`.
A commit of other docs runs neither.
`bin/checks-needed` finds the embeds and reads in the sources on each commit, so a new `include_str!` needs no list kept by hand; its `-h` gives the rules.
Every check runs without `CHECK_BASE` (CI, an agent without llm-toolbox, or no commit known to pass), when nothing differs from it, when the change touches those rules, when git can't diff against it, and when `bin/checks-needed` is missing, fails or runs past 60 s.
A small fix commit gets only clippy (`bin/agent-quick-check`, run by `agent-commit-gate`), and only for a build input; `build-state finish` runs the rest.
CI runs all of them.
Run `cargo fmt` after each batch of edits to `.rs` files: otherwise the hook reformats them at commit, and edits you make after that no longer match the text you hold.
`python3 install.py` links the hooks once per clone and installs cargo-nextest and sqlx-cli; sandbox-setup runs it after a recreate.

The Rust tests run under cargo-nextest, set in `.config/nextest.toml` to print only the tests that fail, with their output, and a summary line.
To run them that way, run `cargo nextest run --cargo-quiet`. nextest skips doctests, so the hook also runs `cargo test --doc`.
Install nextest once per machine: `cargo install cargo-nextest --locked`, or the prebuilt binary from https://get.nexte.st/latest/linux.

Mutation testing (cargo-mutants, set in `.cargo/mutants.toml`) runs in CI, only when started by hand, since a full run takes about 10 runner-hours: `gh workflow run mutants.yml`.
The workflow's header says where the results land, and the `mutants-triage` skill turns a run into the few issues worth a test.
To check that a new test catches a mutant the run missed, write the change its line in `missed.txt` names as an apply-edits block and run it through `mutate-check` (`-h`), which takes seconds.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script.
The version and hashes live only there, and its docstring says how to move to a new release.
The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run` and `bin/slop-lint` are copies of llm-toolbox's tools. Update them by copying from there.
