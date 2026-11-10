# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents.
Most are null or inconclusive, and the pitch leads with those.
The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: building, in the order of `docs/design.md`'s Build order.
The owner and an agent settled the design on 2026-11-03 and 2026-11-04, and the owner signed off on the entry format's fourth draft on 2026-11-05.

## Open work

Plans and open work are GitHub issues on `filedrawer-dev/filedrawer`, not these docs: `issue-todo open --repo filedrawer-dev/filedrawer` lists every open item, and each build step's plan is an issue of its own (#16 for step 4).
Read them before you plan work, and file new work there.
To check whether an issue already covers something, `issue-todo open --titles` lists the open issues one a line, by number and title, with each one's count of open items and the session that claimed it, if any.
`issue-todo show N` prints one issue's items, from its body and comments together, ticked or not.
Other sessions build beside you, so claim an issue before you start on it, `issue-todo claim N --repo filedrawer-dev/filedrawer`, and release it (`issue-todo release N`) if you stop before it lands; skip one another session holds, which `open --titles` shows as `[claimed by ID]`.
When you choose what to build next, prefer, among the issues the plan allows, the one whose files overlap least with what the other sessions claimed or have in their worktrees (`git worktree list` names their topics), so the landings don't conflict.
When a commit finishes an issue, end its message with `Closes #N`, so the owner's push closes it; when a slice of a plan lands, tick it with `issue-todo tick N MATCH --note <sha>`.
`uncertainties.md` holds the choices made while building that the owner hasn't settled yet.

The owner has root on the VPS but keeps SSH keys out of the sandbox.
Anything done on the server is a tool the owner runs from their own machine, such as `ops/provision-vps`.

## Deploys

A push to `main` that passes CI deploys to staging (`build-staging` and `deploy-staging` in `.github/workflows/ci.yml`).
The build job makes a static x86_64 musl binary; the deploy job, which runs no third-party action, pipes it over SSH as the `deploy` user.
That user's key may only run `ops/remote/filedrawer-receive`, which allows one deploy at a time and passes the upload on stdin to `ops/remote/filedrawer-install` through one sudo rule.
The install script keeps its own root-owned copy, checks the size and hash, runs the binary's `-h` as the environment's user in a transient unit with no network and a 10 s limit, keeps the previous binary, and rolls back if `/tags` doesn't answer within 30 s.
`ops/provision-vps` sets up the user and key, stores `DEPLOY_SSH_KEY` as a secret of the `staging` environment (main only), and writes the variables `VPS_HOST` and `VPS_KNOWN_HOSTS`; rerunning it replaces the key, and `--only-new` keeps it but still rewrites the sudo rule.
A binary that answers `/tags` passes, and it runs as `filedrawer` with staging's data and settings.
`ops/provision-vps production` sets up production beside staging, as `filedrawer-prod` on port 8080, and `ops/provision-vps production account` follows its first deploy (design.md, Hosting and operations); the deploy key can't reach production.
Production deploys come with #42.

The field test's entries and logs are in `field-test/`, git-excluded and local only; never commit or publish them without the owner's say.

## Code

- `crates/analyzer`: reads an entry by `docs/entry-format.md`, lists what would refuse it and what the server couldn't read, and renders its card with analyzer v1.
  The server and the CLI share it.
  Reason: in the field test every agent wrote its own parser and card renderer, a dozen in all, and each read the spec differently.
- `entry-check FILE...` checks entries offline, for contributing agents and field tests: `cargo run -q --bin entry-check -- FILE...`.
  It exits 1 when the server would refuse an entry.
  After an analyzer change, run `bin/field-test-diff` through `heavy` (`-h`): it builds entry-check at the commit your branch forked from, or at `--against REV`, and lists the field-test entries whose outcome changed, without their contents.
- `crates/server`: the `filedrawer` binary.
  `filedrawer -h` lists its commands and every setting, with what production refuses; a test (`cli::help_names_every_setting_serve_reads`) fails when it misses one `config.rs` reads, so read it there rather than keep a list here.
  The GitHub token is shared with staging (`ops/provision-vps github`).
  Email goes to the log when no SMTP settings are set, but staging has real ones (`ops/remote/mail.sh`), so a feature that emails the owner reaches their inbox from staging too, where CI publishes test entries after each deploy.
  `crates/server/install.md` is what agents read at `/skill.md`: it installs the two skills in `skills/`.
  A new route is named there or in a skill, or listed in `NOT_FOR_AGENTS` in `crates/server/tests/it/skill.rs` (CODING_STANDARDS.md, Untrusted text, says which); a test refuses one that is neither.
  `CODING_STANDARDS.md` holds the rules the review applies.
- `crates/server/templates`: the HTML pages, askama templates.
  A template can't reach a field from inside a macro call, so `matches!(file.state, ...)` fails to compile; give the type a method instead.
- `crates/server/legal`: the text of `/terms` and `/privacy`, one markdown file each, which a browser gets rendered and an agent gets as it is.
- `crates/server/static`: the pages' one stylesheet, the home page's one script (its copy button), and the Courier Prime fonts, with their licence, compiled into the binary.
  Colours are tokens on `:root`, light by default and dark under `prefers-color-scheme: dark`; a change of look changes the tokens, not the rules.
- `bin/live-preview` (`-h`): a live preview of the home page for drafting its text, as the owner does by hand, at http://localhost:8090 once the port is published from the host.
  It renders `GET /` from `base.html` and `home.html` on disk on each request, around the entries of a dev server it builds and seeds, serves `site.css` and `copy.js` from disk, and reloads the page when any of the four changes; other pages' markup is the dev server's, as built.
  Run it without `heavy`, which it refuses: it queues its own build, and inside a heavy job it would hold the lane every commit check waits for until it stopped.
- `skills/`: `filedrawer-search` and `filedrawer-submit`, the skills agents install from this repo's git tags and the server serves at `/skills/<name>/` (design.md, Skills).
  They aren't skills for working on this repo, which live in `.claude/skills/`.
  They describe only what the server does, so update them with each change to a route.
  Then run `bin/skills sync`, which also follows a change to `docs/entry-format.md`: it copies the format into each skill and records each version in `skills/versions.txt`, bumping past every version origin's tags or origin/main already hold; a test refuses a skill changed without it.
  Commits and landings run `bin/skills sync --check`, which fetches origin's tags first and refuses a version tagged with other contents; so does a push to main, through the pre-push hook (`bin/git-pre-push`).
  CI's `release-skills` job tags each new version before staging deploys, with a deploy key `ops/protect-skill-tags` sets up (`-h`).
- `ops/smoke-staging` follows the submit skill against staging as an agent would, and checks that each skill's tag holds what the site serves; `screen` checks staging's real screen and `eval` its labelled cases (`ops/screen-eval/`).
  CI runs the first two after each deploy, each as a test account of its own.
  Read its `-h` before running it, or changing staging's test accounts, their settings or CI's deploy checks: it holds the accounts, the secrets and the settings each mode reads.
- `GET /staging/status` (staging and dev; production has no such route) shows what screening spent today against the cap, how many entries wait for it and since when, and when its last call succeeded and when the last one failed, with the API's error; and for the link checks, GitHub's hourly allowance and the calls left, the hosts that asked the server to wait, and how often the API overruled GitHub's commit page.
  Look there first when staging's screen or link checks seem stuck: `curl -H "X-Staging-Key: $FILEDRAWER_STAGING_KEY" https://staging.filedrawer.dev/staging/status`.
  The last calls are kept in memory, so a restart empties them until the next one.
- `GET /staging/headers` (staging and dev; production has no such route, and only staging records) groups the requests staging has seen by the headers they sent, `Accept`, `User-Agent` and the two `Sec-Fetch-*`, with each client's routes and how many got HTML, to check the HTML rule against agents' fetch tools (#28).
  The record is a file beside staging's database and survives deploys.
- Staging's Caddy (`ops/remote/caddy.sh`) refuses every request but `robots.txt` without `X-Staging-Key` or the owner's basic auth, so a skill path that sends no header file reaches production but not staging.
  For a run on staging, put the key line in `~/.config/filedrawer/staging.filedrawer.dev.header` (mode 600) first; the skills send that file whenever it exists, and their token commands keep the key line.
  For fetch tools that can't send a header, the owner opens a field-test key with `ops/field-test-key open` (`-h`), which lets GETs of pages through as `?key=`; `docs/field-test.md` says how a run uses it.

### SQL

Queries go through sqlx's checked macros (`CODING_STANDARDS.md`), which compile against `.sqlx/`, the committed query data, when `DATABASE_URL` is unset, as in CI and the commit hook: `bin/agent-pre-commit` builds a copy of the index with no `.env`, so a stale `.sqlx/` refuses the commit.
After a merge, checkout or rebase brings new migrations, the git hooks rebuild `target/dev.db` (`bin/sqlx-refresh --if-stale`; `python3 install.py` enables them).
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
  Read it before you touch submission, the analyzer, cards or the skills.
- `docs/prior-art.md`: related projects, with the date of the survey.
  Read it before claiming novelty or adding a feature that may already exist elsewhere.
- `docs/field-test.md`: how to run a field test with agents cheaply.
  Read it before you brief agents to write entries, as in the usability runs on staging.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

Date a decision by the owner's local date, as `date +%F` prints it in the sandbox (America/Los_Angeles), not UTC: git, GitHub and the server's stored times are UTC and run a day ahead each evening.

Markdown here breaks after each sentence end, so grep, diffs and merge conflicts show sentences rather than whole paragraphs.
The commit hook's `bin/sentence-lines` keeps it that way and leaves entries as their authors wrote them: the analyzer's fixtures, `ops/screen-eval/` and `ops/smoke-fixture.md`.

## Checks

Start a worktree with `bin/new-worktree TOPIC` (`-h`): it adds `../filedrawer-wt-TOPIC` beside the main tree and starts its warm build in the background, which sccache makes quick.
For a change with no build input (below), such as most of `docs/`, `bin/` and `ops/`, add `--no-build`, since its commits run no Rust check and so need no build.
`bin/checks-needed --why PATH` says whether a path is a build input.

Committing runs the hook's checks, so read its output rather than running them first; the full check runs when wt-ship lands each commit.
Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, actionlint with shellcheck (when a workflow is staged), shellcheck on staged shell scripts in `bin/` and `ops/`, and `bin/sentence-lines` on staged markdown, ruff, and `cargo fmt` (when a `.rs` file is staged), which write: a person's commit that needs formatting is refused with the files rewritten, to stage, and `bin/pre-commit-run` stages an agent's and checks again.
An agent's commit then runs clippy, for a build input, the skills check, for a change to the skills, the one test that every route is named for agents or listed in `NOT_FOR_AGENTS`, for a change to the router, a document naming routes or `skill.rs`, and the rule that no test in `bin/tests` sets a wall-clock limit, for a change there (`bin/agent-commit-check`, which `agent-commit-gate` runs in the full check's place because `bin/git-pre-commit` sets `AGENT_CHECK_AT_COMMIT`): run the tests you wrote or touched before you commit.
wt-ship runs the full check, `bin/agent-pre-commit`, on each commit when it lands, while it holds the main tree: the skills check, the tests in `bin/tests`, clippy and the Rust tests, each only when the change needs it.
The change is what differs from `$CHECK_BASE`, which llm-toolbox's `agent-commit-gate` sets to the nearest commit that passed: at a commit, usually HEAD, so what's staged; on wt-ship's recheck, the commit before, or for the first, the tip it rebased onto.
The Rust checks run for a build input: anything in `crates/`, the Cargo files, `.sqlx/`, `.cargo/` and `.config/`, and the files the crates embed or their tests read, such as `docs/entry-format.md` and `ops/smoke-staging`.
`bin/tests` runs for a change in `bin/`, `ops/` or a crate (one of its tests reads the crates' references), or to a file those tests read, such as `.pre-commit-config.yaml`.
A commit of other docs runs neither, and doesn't wait in heavy's queue either.
Each check asks `agent-commit-gate` for a place there only as it starts (`$CHECK_QUEUE_EXIT`, `agent-commit-gate -h`), so a commit that needs none passes in seconds.
`bin/checks-needed` finds the embeds and reads in the sources on each commit, so a new `include_str!` needs no list kept by hand; its `-h` gives the rules.
Every check runs without `CHECK_BASE` (CI, an agent without llm-toolbox, or no commit known to pass), when nothing differs from it, when the change touches those rules, when git can't diff against it, and when `bin/checks-needed` is missing, fails or runs past 60 s.
A small fix commit gets clippy and the skills check through `bin/agent-quick-check`.
CI runs all of them.
A push to main runs `.githooks/pre-push` (`bin/git-pre-push`), which refuses a commit whose skills are out of step with origin's tags.
Run `cargo fmt` after each batch of edits to `.rs` files: otherwise the hook reformats them at commit, and edits you make after that no longer match the text you hold.
`python3 install.py` enables the hooks once per clone and installs cargo-nextest and sqlx-cli; sandbox-setup runs it after a recreate.
The hooks are links in `.githooks/` to scripts in `bin/`, and `core.hooksPath` is `.githooks`, which git reads from the top of the worktree that runs the hook.
So each worktree runs its own copies, and a commit that changes a hook script is checked by its own version, at commit and when wt-ship lands it.
A tree checked out at a commit older than `.githooks/` runs no hooks: rebase it onto main before you commit there.
A clone set up by an older `install.py` keeps its links in `.git/hooks` to the main tree's scripts, which still work, until its next `python3 install.py` removes them.

The Rust tests run under cargo-nextest, set in `.config/nextest.toml` to print only the tests that fail, with their output, and a summary line.
To run them that way, run `cargo nextest run --cargo-quiet`. nextest skips doctests, so the full check also runs `cargo test --doc`.
Each crate's integration tests build as one binary, `tests/it/main.rs`, with each file in `tests/it/` a module of it, so a new test file needs a `mod` line there; a test in `main.rs` fails without one.
To run one file's tests, filter on its module: `cargo nextest run --cargo-quiet -p filedrawer-server -E 'test(/^read::/)'`.
The Python tests in `bin/tests` run through `bin/pytest`, the pinned pytest with pytest.ini's plugins: `heavy --quick -- bin/pytest -n0 bin/tests/test_x.py`; plain `uv run pytest` lacks pytest-xdist, which pytest.ini's `-n` needs.
Run a targeted run like that, a few tests that take seconds, through `heavy --quick -- CMD`, which starts it at once on quiet cores and otherwise next in heavy's queue (`heavy -h`); a whole suite goes through plain `heavy`.
After an edit to a crate, that run first rebuilds the test binary, a minute or more, which overruns `--quick`: build it first as a background call, `bgrun --wait -o LOG -- heavy --share -- cargo nextest run --cargo-quiet --no-run`, then run the tests with `--quick`.
A `mutate-check` rebuilds for each mutant, so it goes through plain `heavy` the same way.
Give a foreground `heavy --quick` call a Bash timeout of 600000 ms, since it may wait behind a running job; llm-toolbox's heavy-gate refuses one without it.
Install nextest once per machine: `cargo install cargo-nextest --locked`, or the prebuilt binary from https://get.nexte.st/latest/linux.

Mutation testing (cargo-mutants, set in `.cargo/mutants.toml`) runs in CI, in `.github/workflows/mutants.yml`.
Each day it tests the mutants in the lines changed since the last run that tested all of its own, and skips when nothing changed.
A full run takes about 10 runner-hours, so it runs only when started by hand: `gh workflow run mutants.yml`.
The workflow's header says where the results land, and the `mutants-triage` skill turns a run into the few issues worth a test, then marks the run triaged in a comment on #19.
To check that a new test catches a mutant the run missed, write the change its line in `missed.txt` names as an apply-edits block and run it through `mutate-check` (`-h`), which takes seconds.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script.
The version and hashes live only there, and its docstring says how to move to a new release.
The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.
So is a commit there that stages a shell script while uv's cache lacks the pinned shellcheck; `SKIP=shellcheck git commit` skips only that one, and CI still runs it.

`bin/caddy` does the same for Caddy, pinned by SHA-512, and runs it with the given arguments.
The tests in `bin/tests` that run a real Caddy against `ops/remote/caddy.sh`'s and `ops/remote/field-test-key.sh`'s config get it through conftest's `real_caddy` fixture, or from `$FILEDRAWER_TEST_CADDY` when that is set.
Without network access they skip and say why, except in CI, where they fail.

shellcheck refuses errors, warnings and infos, not style.
To keep a finding that's intended, put `# shellcheck disable=SCnnnn  # why` on the line before it, as `ops/smoke-staging` does; a directive right after the shebang covers the file, as its SC2016 one does, since its `check` calls single-quote their conditions on purpose.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run`, `bin/slop-lint` and `bin/tests/leftover_processes.py` are copies of llm-toolbox's. Update them by copying from there.
