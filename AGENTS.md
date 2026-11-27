# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents.
Most are null or inconclusive, and the pitch leads with those.
The GitHub org is `filedrawer-dev`, and the repo is `filedrawer-dev/filedrawer`.

Status: building, in the order of `docs/design.md`'s Build order.
The owner and an agent settled the design on 2026-11-03 and 2026-11-04, and the owner signed off on the entry format's fourth draft on 2026-11-05.

## Open work

Plans and open work are GitHub issues on `filedrawer-dev/filedrawer`, not these docs: `issue-todo open --repo filedrawer-dev/filedrawer` lists every open item, and each build step's plan is an issue of its own (#16 for step 4).
Read them before you plan work, and file new work there.
A bug the owner reports in chat and you fix in that same session needs no issue.
To check whether an issue already covers something, `issue-todo open --titles` lists the open issues one a line, by number and title, with each one's count of open items and the session that claimed it, if any.
`issue-todo show N` prints one issue's items, from its body and comments together, ticked or not.
Other sessions build beside you, so claim an issue before you start on it, `issue-todo claim N --repo filedrawer-dev/filedrawer`, and release it (`issue-todo release N`) if you stop before it lands; skip one another session holds, which `open --titles` shows as `[claimed by ID]`.
`issue-todo claims --repo filedrawer-dev/filedrawer` lists only the claimed issues, with each claim's session and age.
When you choose what to build next, prefer, among the issues the plan allows, the one whose files overlap least with what the other sessions claimed or have in their worktrees (`git worktree list` names their topics), so the landings don't conflict.
When a commit finishes an issue, end its message with `Closes #N`, so the owner's push closes it; when a slice of a plan lands, tick it with `issue-todo tick N MATCH --note <sha>`.
`uncertainties.md` holds the choices made while building that the owner hasn't settled yet.

The owner has root on the VPS but keeps SSH keys out of the sandbox.
The VPS runs Debian and the sandbox Ubuntu, so check a system path or a package's behaviour the server relies on against Debian's own sources (salsa.debian.org), not the sandbox: Ubuntu's `update-success-stamp`, which Debian never writes, once kept the Caddy guard failing.
Anything done on the server is a tool the owner runs from their own machine, such as `ops/provision-vps`.

## Deploys

A push to `main` that passes CI deploys to staging (`build-staging` and `deploy-staging` in `.github/workflows/ci.yml`).
The build job makes a static x86_64 musl binary; the deploy job, which runs no third-party action, pipes it over SSH as the `deploy` user.
That user's key may only run `ops/remote/filedrawer-receive`, which allows one deploy at a time and passes the upload on stdin to `ops/remote/filedrawer-install` through one sudo rule.
The server runs its own root-owned copy of `filedrawer-install`: `ops/deploy-production` replaces production's when it differs from `origin/main`'s, but a staging deploy can't, so a change to the script reaches staging's deploys only once the owner reruns `ops/provision-vps` (or its `smoke`, `screening`, `github`, `github-oauth` or `mail` stage); say so when you land one.
The install script keeps its own root-owned copy, checks the size and hash, runs the binary's `-h` as the environment's user in a transient unit with no network and a 10 s limit, keeps the previous binary, and rolls back if `/tags` doesn't answer within 30 s.
Then it probes the server's way out with the binary's `check-outbound`, under the unit's own sandbox, and rolls back when only the new binary can't get out; when the previous one can't either, the new one stays and the deploy exits 3, which fails CI's job (design.md, Hosting and operations; #102).
`ops/provision-vps` holds the flags and the run order; its stages are in `ops/provision-vps.d/`, `staging.sh` and `production.sh` each in the order they run, beside `helpers.sh` and the /wizard skill's `library.sh`, so a review of one environment's stage reads that environment's file and `helpers.sh`.
`ops/provision-vps` sets up the user and key, stores `DEPLOY_SSH_KEY` as a secret of the `staging` environment (main only), and writes the variables `VPS_HOST` and `VPS_KNOWN_HOSTS`; rerunning it keeps the key but still rewrites the sudo rule, and `--ask-all` offers to replace it.
A rerun asks for nothing `ops/.vps.env`, the server or GitHub already holds: a secret the server holds is sent empty, and its `ops/remote/` script keeps the copy it has (#112).
`--ask-all` asks everything again, and for each secret the server holds asks whether to change it before showing its tab; `--only-new`, the old name for a rerun, is still taken.
A binary that answers `/tags` passes, and it runs as `filedrawer` with staging's data and settings.
`ops/provision-vps production` sets up production beside staging, as `filedrawer-prod` on port 8080, and `ops/provision-vps production account` follows its first deploy (design.md, Hosting and operations); the deploy key can't reach production.
The server takes each Caddy release through unattended-upgrades, and `ops/remote/caddy-upgrade-check` (`-h`), which `ops/provision-vps caddy-upgrades` installs, checks the sites after each one and rolls back and holds one that breaks them (#100).
It takes kernels the same way, and `ops/remote/filedrawer-reboot-check` (`-h`), which `ops/provision-vps reboots` installs, mails the owner when one waits for a reboot and reboots the server at 05:00 UTC 4 days later, unless a deploy, a backup or a login is under way then; `--dry-run` on the server mails a test and says what it would do.
Both source `ops/remote/filedrawer-alerts.sh`, their mail, ping and site check, and both stages install all three, so a change to any of them reaches the server with either stage.
The owner deploys production from their own machine with `ops/deploy-production [COMMIT]` (`-h`), which promotes the binary CI built for the commit staging runs, or for an older one to roll back to, waiting first while CI still runs on main's tip, so a deploy can follow a push at once (#118); the install copies production's database first and puts it back if the new binary doesn't answer.
An older binary runs on a newer schema, so a migration adds only what it can ignore unless `crates/server/migration-rollback.txt` says otherwise (CODING_STANDARDS.md, SQL).
`ops/audit-vps` (`-h`), which the owner also runs from their own machine, prints a report of the server's state for a security review and changes no file the server keeps: sockets, firewall, sshd, sudo rules, owners and modes, unit hardening, staging's memory and disk use beside production's, and pending upgrades, with no secret in it, and whether each root-owned script on the server is `origin/main`'s.

The field test's entries and logs are in `field-test/`, git-excluded and local only; never commit or publish them without the owner's say.

## Code

- `crates/analyzer`: reads an entry by `docs/entry-format.md`, lists what would refuse it and what the server couldn't read, and renders its card with analyzer v2 (every line starts with a server label).
  The server and the CLI share it.
  Reason: in the field test every agent wrote its own parser and card renderer, a dozen in all, and each read the spec differently.
  Its library is `filedrawer_analyzer`, as on crates.io; the server's Cargo.toml renames it `analyzer`.
- `crates/cli`: the package `filedrawer`, published on crates.io and PyPI (#74), and as `@filedrawer/cli` on npm (#84), which holds the `filedrawer` binary, whose `check` command is `entry-check` under a new name.
  `entry-check` stays, since the published submit skill runs it.
  Each published crate holds a copy of the repo's `LICENSE`, since cargo and maturin package only files inside the crate, and `bin/tests/test_license_copies.py` fails when a copy differs.
  It is a crate of its own so that a later CLI's HTTP dependencies stay out of the analyzer the server builds.
  `crates/cli/pyproject.toml` has maturin wrap both binaries in a wheel, and `crates/cli/npm/pack.mjs` packs the `filedrawer` binary in an npm package per platform beside the launcher `@filedrawer/cli` (`crates/cli/npm/filedrawer.js`); `.github/workflows/release-cli.yml` builds the wheels, the sdist and the npm packages on each pull request that touches them.
  To release, the owner runs `ops/release-cli [patch|minor|major]` (`-h`) from their own checkout: it bumps the CLI's version, and the analyzer's when it differs from crates.io's at its version, README and dev-dependencies included, commits, asks before pushing main, waits for CI's `check` job, starts `release-cli.yml` on main, watches it, and checks that each registry lists the version; a rerun picks up where it stopped.
  The workflow publishes main's version to crates.io, PyPI and npm, wherever it's missing or, on PyPI and npm, only partly there, through each registry's trusted publishing (`bin/release-plan`, `-h`, says what and when).
  A push doesn't start it: crates.io's trusted publishing refuses a run that CI's completion started (design.md, Distribution).
  Agents don't release: the tool refuses to run with `AI_AGENT` set, and its `-h` gives the steps by hand.
  Between releases the analyzer changes freely.
  Each crate's crates.io settings name `release-cli.yml` and the environment `crates-io` as its trusted publisher, which the owner sets up by hand once, with that environment open to main only.
  `ops/claim-names` (`-h`) made the first release: the crates.io names, PyPI's pending publisher, and the npm packages as placeholders with a trusted publisher each.
- `entry-check FILE...` checks entries offline, for contributing agents and field tests: `cargo run -q --bin entry-check -- FILE...`.
  It exits 1 when the server would refuse an entry, and a debug build older than its sources exits 3 without checking, naming the rebuild, so a stale `target/debug/entry-check` can't give old answers (design.md, Distribution).
  After an analyzer change, run `bin/field-test-diff` through `heavy` (`-h`): it builds entry-check at the commit your branch forked from, or at `--against REV`, and lists the field-test entries whose outcome changed, without their contents.
- `crates/server`: the `filedrawer-server` binary, which the VPS installs as `bin/filedrawer` (design.md, Hosting and operations).
  `filedrawer-server -h` lists its commands and every setting, with what production refuses; a test (`cli::help_names_every_setting_serve_reads`) fails when it misses one `config.rs` reads, so read it there rather than keep a list here.
  The GitHub token is shared with staging (`ops/provision-vps github`).
  Email goes to the log when no SMTP settings are set, but staging has real ones (`ops/remote/mail.sh`), so a feature that emails the owner reaches their inbox from staging too, where CI publishes test entries after each deploy.
  `crates/server/install.md` is what agents read at `/skill.md`: it has the agent ask its human, then download and run `crates/server/install.sh`, served at `/install.sh`, which installs the two skills in `skills/` from their tags.
  Production serves the script's bytes as they are, so a reader can compare it with the repo; staging and dev put their own address on its `base=` line.
  A new route is named there or in a skill, or listed in `NOT_FOR_AGENTS` in `crates/server/tests/it/skill.rs` (CODING_STANDARDS.md, Untrusted text, says which); a test refuses one that is neither.
  `CODING_STANDARDS.md` holds the rules the review applies.
- `crates/server/templates`: the HTML pages, askama templates.
  A template can't reach a field from inside a macro call, so `matches!(file.state, ...)` fails to compile; give the type a method instead.
  The server sends a page only to a request with `Sec-Fetch-Dest: document` (design.md, Agent interface): `curl -H 'Sec-Fetch-Dest: document' URL` gets the HTML, and a curl without it, `Accept: text/html` or not, gets the text agents read.
- `crates/server/legal`: the text of `/terms` and `/privacy`, one markdown file each, which a browser gets rendered and an agent gets as it is.
- `crates/server/static`: the pages' one stylesheet, the home page's one script (its copy button), and the Courier Prime fonts, with their license, compiled into the binary.
  Colors are tokens on `:root`, light by default and dark under `prefers-color-scheme: dark`; a change of look changes the tokens, not the rules.
  No test runs `copy.js` or lays the page out, since the checks open no browser (the owner's rule, 2026-11-25): after a change to `home.html`'s line to paste, to `copy.js` or to the slip's rules in `site.css`, run `bin/probe-copy` (`-h`), which clicks the slip in headless Chromium, at its edges too, and takes about 10 s.
- `bin/live-preview` (`-h`): a live preview of the site's pages for drafting their text, as the owner does by hand, at http://localhost:8090 once the port is published from the host.
  It renders `GET /` from `base.html` and `home.html` on disk on each request, around the entries of a dev server it builds and seeds, serves `site.css` and `copy.js` from disk, and reloads the page when any of the four changes.
  Other pages are the dev server's: a save of any other template or of `crates/server/legal/` rebuilds and restarts it, and its pages reload then.
  The dev server screens with the scripted screen, which only dev takes (`filedrawer-server -h`), so seeding fills each section of `/admin`, and the preview signs every browser in as its admin; `crates/server/tests/it/live_preview.rs` runs that seeding against the real server.
  Anyone who can reach the published port is then that admin, of seeded dev data only, and a logout from a browser with no session of its own ends the admin's session for every browser until the preview restarts.
  It also serves `/skill.md`, `/install.sh` and the skills' files from the checkout, as plain text, filled in by its own copy of the server's rules, which a test in `crates/server/tests/it/skill.rs` holds to the server's (#71).
  Run it without `heavy`, which it refuses: it queues its own build, and inside a heavy job it would hold the lane every commit check waits for until it stopped.
  `bin/live-preview --static OUT` writes the home page, down to its README card and without the drawer, to one file that opens from disk with the site's own fonts, and starts nothing: use it to screenshot the page, rather than building a render by hand.
- `skills/`: `filedrawer-search` and `filedrawer-submit`, the skills agents install from this repo's git tags and the server serves at `/skills/<name>/` (design.md, Skills).
  Their files name `https://filedrawer.dev` outright, with nothing to fill; staging and dev point them at themselves where they serve or install them.
  They aren't skills for working on this repo, which live in `.claude/skills/`.
  They describe only what the server does, plus where else the search skill has the agent look and the Alignment Forum tags the submit skill has it check (design.md, Skills and Agent interface), so update them with each change to a route.
  Then run `bin/skills sync`, which also follows a change to `docs/entry-format.md`: it copies the format into each skill and records each version in `skills/versions.txt`, bumping past every version origin's tags or origin/main already hold; a test refuses a skill changed without it.
  A rebase, merge or cherry-pick recomputes `skills/versions.txt` rather than stopping on it: `.gitattributes` names `bin/skills merge` as its merge driver, which `python3 install.py` registers in the clone's config, so every worktree gets it (`bin/skills -h` says when it still leaves a conflict).
  To resolve a conflict there by hand, as in a clone set up before the driver, take either side, run `bin/skills sync`, and stage what it writes.
  Commits and landings run `bin/skills sync --check`, which fetches origin's tags first and refuses a version tagged with other contents; so does a push to main, through the pre-push hook (`bin/git-pre-push`).
  CI's `release-skills` job tags each new version before staging deploys, with a deploy key `ops/protect-skill-tags` sets up (`-h`).
- `ops/smoke-staging` follows the submit skill against staging as an agent would, and checks that each skill's tag holds what the site serves; `screen` checks staging's real screen and `eval` its labeled cases (`ops/screen-eval/`); `read` runs only the checks that write nothing, which `ops/deploy-production` runs against production.
  CI runs the first two after each deploy, each as a test account of its own.
  Read its `-h` before running it, or changing staging's test accounts, their settings or CI's deploy checks: it holds the accounts, the secrets and the settings each mode reads.
- `ops/usability-run` (`-h`) runs one scripted usability run of the skills on staging, an agent's own TUI driven through llm-toolbox's `drive-agent`, as a fresh staging account it deletes at the end, or several at once with `batch`; `report RUN_DIR...` sums up each run in at most 15 lines, with keys and tokens blanked, so read it before any transcript; `ops/remove-entries` takes down, as the owner, the entries a run left public.
  `docs/field-test.md`, Scripted usability runs, says how; the scenarios live in `keep/filedrawer/usability/`, out of this public repo.
- `ops/first-screen-bakeoff` (`-h`) measured a cheaper screen in front of Haiku (#131), which the owner then decided not to build (design.md, Rejected alternatives): it sends the labeled cases in `ops/first-screen-eval/`, `ops/screen-eval/` and the local field test to Jev (key in `JEV_API_KEY`), keeps the scores in the workspace's `keep/filedrawer/first-screen/`, and reports what a rule would reject.
  The server calls no such model; rerun the script only if that decision is reopened.
- `ops/b2-probe` (`-h`), which the owner runs from their own machine with B2's master key, answers what #4's backups need from a live B2 (#156): it makes two throwaway buckets, one under a compliance lock, and a restricted key for each, checks what those keys may delete, runs the pinned Litestream 0.5.18 against the locked one, and deletes the keys; `recheck`, 3 days later, says whether the lifecycle deletes a version once its lock lapses and removes the buckets.
  Its tests run it against `bin/tests/fake_b2.py` and `fake_litestream.py`.
- `GET /staging/status` (staging and dev; production has no such route) shows what screening spent today against the cap, how many entries wait for it and since when, and when its last call succeeded and when the last one failed, with the API's error; for the link checks, GitHub's hourly allowance and the calls left, the hosts that asked the server to wait, and how often the API overruled GitHub's commit page; and how many emails wait to be sent and since when, with the last send and the last failure's class (`connect`, `smtp 535`, …).
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
After adding or changing a query or a migration, run `bin/sqlx-refresh`; a commit that forgot is refused with a line naming it.
It rebuilds `target/dev.db` from the migrations (writing `.env` if it's missing), so an edited, unpushed migration needs no cleanup, then runs `cargo sqlx prepare --workspace`.

The macros can't describe table-valued functions such as `json_each`: the build fails with "no such table column: json_each.value".
Query once per value instead, or join a real table.

`sqlx` is sqlx-cli: `cargo install sqlx-cli --no-default-features --features sqlite,rustls --locked`.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives.
  Read it before you change behavior.
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

Start a worktree with `bin/new-worktree TOPIC` (`-h`): it adds `../filedrawer-wt-TOPIC` beside the main tree and starts its warm build in the background, which sccache makes quick, then, while the commit checks' slot (below) holds no build, a second one there (`bin/agent-pre-commit --warm`), so the first commit doesn't build cold.
It starts at the main tree's HEAD, which may hold landings not yet pushed; for a pull request's branch, `git fetch` first and add `--at origin/main`, so the branch holds only its own commits.
For a change with no build input (below), such as most of `docs/`, `bin/` and `ops/`, add `--no-build`, since its commits run no Rust check and so need no build.
`bin/checks-needed --why PATH` says whether a path is a build input.

Committing runs the hook's checks, so read its output rather than running them first; the full check runs on the tip when wt-ship lands, and a landing of two or more commits lands as a merge, so `git bisect --first-parent` walks only checked commits.
Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, actionlint with shellcheck (when a workflow is staged), zizmor over the workflows and Dependabot's settings (when a file in `.github/` is staged), shellcheck on staged shell scripts in `bin/` and `ops/`, on `crates/server/install.sh` and on the skills' `upgrade.sh`, and `bin/sentence-lines` on staged markdown, ruff, and `cargo fmt` (when a `.rs` file is staged), which write: a person's commit that needs formatting is refused with the files rewritten, to stage, and `bin/pre-commit-run` stages an agent's and checks again.
An agent's commit then runs clippy, for a build input, the skills check, for a change to the skills, the one test that every route is named for agents or listed in `NOT_FOR_AGENTS`, for a change to the router, a document naming routes or `skill.rs`, with the `skill_folders` tests beside it for a change to the skills, the rule that no test in `bin/tests` sets a wall-clock limit and the table of which checks each path needs (`bin/tests/test_checks_needed.py`), for a change there, the rules that no `pipefail` script pipes into a reader that stops early, that a workflow's `run:` blocks hold no parsing and at most 5 command lines, and that a workflow names each action by a commit, for a change in `bin/`, `ops/` or `.github/workflows/`, and, for a change in a crate's `tests/it/`, `bin/test-modules`, which refuses a file there that the staged `main.rs` has no `mod` line for (`bin/agent-commit-check`, which `agent-commit-gate` runs in the full check's place because `bin/git-pre-commit` sets `AGENT_CHECK_AT_COMMIT`): run the tests you wrote or touched before you commit.
It also runs the real-Caddy tests, which `bin/tests/conftest.py` marks `real_caddy`, in heavy's queue, for a change to `ops/remote/caddy.sh`, `ops/remote/field-test-key.sh`, `bin/caddy`, the test files that hold them or `conftest.py`; without a network they skip, which passes.
wt-ship runs the full check, `bin/agent-pre-commit`, on the tip of each landing, while it holds the main tree: the skills check, the tests in `bin/tests`, clippy and the Rust tests, each only when the change needs it.
Each check's seconds show in the gate's pass line, and every run's timings go to `.git/agent-check-timings.tsv` (columns in `agent-commit-gate -h`), so `tail` it before benchmarking a slow landing.
The change is what differs from `$CHECK_BASE`, which llm-toolbox's `agent-commit-gate` sets to the nearest commit that passed: at a commit, usually HEAD, so what's staged; on wt-ship's recheck of a landing's tip, the tip of the branch it rebased onto.
The Rust checks run for a build input: anything in `crates/`, the Cargo files, `.sqlx/`, `.cargo/` and `.config/`, and the files the crates embed or their tests read, such as `docs/entry-format.md` and `ops/smoke-staging`.
`bin/tests` runs for a change in `bin/`, `ops/` or a crate (one of its tests reads the crates' references), or to a file those tests read, such as `.pre-commit-config.yaml`.
Outside `bin/` and `ops/`, it runs only the test files that reach the change, so an analyzer change runs three of them, not the whole suite (`bin/checks-needed -h`, `--bin-tests`).
A commit of other docs runs neither, and doesn't wait in heavy's queue either.
Each check asks `agent-commit-gate` for a place there only as it starts (`$CHECK_QUEUE_EXIT`, `agent-commit-gate -h`), so a commit that needs none passes in seconds.
One whose queued part takes seconds, such as a clippy the check slot has built for this toolchain and `Cargo.lock`, asks for a `--few-seconds` place instead (`$CHECK_SECONDS_EXIT`), which skips the line on quiet cores.
A commit whose check will wait there (clippy, the real-Caddy tests, or a change to the check rules, as `bin/commit-needs-queue` judges) is refused as a foreground Bash call: run it as a background call, `bgrun --wait -o LOG -- git commit ...`.
`bin/checks-needed` finds the embeds and reads in the sources on each commit, so a new `include_str!` needs no list kept by hand; its `-h` gives the rules.
Every check runs without `CHECK_BASE` (CI, an agent without llm-toolbox, or no commit known to pass), when nothing differs from it, when the change touches those rules, when git can't diff against it, and when `bin/checks-needed` is missing, fails or runs past 60 s.
A small fix commit gets clippy and the skills check through `bin/agent-quick-check`.
CI runs all of them.
The checks run on a copy of what's staged in `$TMPDIR/filedrawer-check-<hash>/`, one per worktree, which each run updates in place, and removes once its worktree is gone; a check of a worktree waits, saying so, while another check of it runs.
The cargo checks build in one check slot for the repo, `../cache/filedrawer/check-slot-<machine>/` beside the main tree, a tree and a target of its own that every worktree's checks share, brought to what's staged the same way, so cargo rebuilds only what differs from the last check (#146): a target that two trees built into leaked builds between them, so a check could pass on unstaged code.
A check waits, saying who holds it, while another worktree's check builds there, and a commit that builds nothing never waits on it; `bin/agent-pre-commit --slot-cold` prints its path, and `bin/agent-pre-commit --reset-slot` empties it once no check holds it, at the cost of one cold build.
A push to main runs `.githooks/pre-push` (`bin/git-pre-push`), which refuses a commit whose skills are out of step with origin's tags.
Run `cargo fmt` after each batch of edits to `.rs` files: otherwise the hook reformats them at commit, and edits you make after that no longer match the text you hold.
`python3 install.py` enables the hooks once per clone and installs cargo-nextest and sqlx-cli; sandbox-setup runs it after a recreate.
The hooks are links in `.githooks/` to scripts in `bin/`, and `core.hooksPath` is `.githooks`, which git reads from the top of the worktree that runs the hook.
So each worktree runs its own copies, and a commit that changes a hook script is checked by its own version, at commit and when wt-ship lands it.
A tree checked out at a commit older than `.githooks/` runs no hooks: rebase it onto main before you commit there.
A clone set up by an older `install.py` keeps its links in `.git/hooks` to the main tree's scripts, which still work, until its next `python3 install.py` removes them.

`rust-toolchain.toml` pins Rust to the version that built production's binary, so the sandbox, the owner's host and CI build with one compiler; a bump changes it and every workflow's `toolchain:` input together (`bin/tests/test_rust_toolchain.py`), and rebuilds every target once.

The Rust tests run under cargo-nextest, set in `.config/nextest.toml` to print only the tests that fail, with their output, and a summary line.
To run them that way, run `cargo nextest run --cargo-quiet`. nextest skips doctests, so the full check also runs `cargo test --doc`.
Each crate's integration tests build as one binary, `tests/it/main.rs`, with each file in `tests/it/` a module of it, so a new test file needs a `mod` line there; a test in `main.rs` fails without one.
To run one file's tests, filter on its module: `cargo nextest run --cargo-quiet -p filedrawer-server -E 'test(/^read::/)'`.
To see what each test took, run `NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 cargo nextest run --cargo-quiet --message-format libtest-json-plus`, whose lines give each test's `exec_time`; a profile with a `junit` table also gives when each test started.
The run is bound by its total CPU, not by its slowest test (#138): to shorten it, profile the whole run (`profile-top`) rather than reorder tests, though nextest's `priority` overrides can start chosen tests first.
The Python tests in `bin/tests` run through `bin/pytest`, the pinned pytest with pytest.ini's plugins: `heavy --few-seconds -- bin/pytest bin/tests/test_x.py`, in parallel, or `-n0 -k NAME` for a few tests, where starting the workers costs more than they save; plain `uv run pytest` lacks pytest-xdist, which pytest.ini's `-n` needs.
The property tests (hypothesis) run 100 examples each; `HYPOTHESIS_PROFILE=dev` runs 20 while you write one, and `deep` 3000 before you trust a change (`bin/tests/conftest.py`).
A whole file such as `test_provision_vps.py` run with `-n0` takes past a minute, which overruns `--few-seconds`; in parallel, six such files take about 16 s.
Run a targeted run like that, a few tests that take seconds, through `heavy --few-seconds -- CMD`, which starts it at once on quiet cores and otherwise next in heavy's queue (`heavy -h`); a whole suite goes through plain `heavy`.
After an edit to a crate, that run first rebuilds the test binary, a minute or more, which overruns `--few-seconds`: build it first as a background call, `bgrun --wait -o LOG -- heavy --share -- cargo nextest run --cargo-quiet --no-run`, then run the tests with `--few-seconds`.
A `mutate-check --diff` run rebuilds for each mutant, so it goes through `heavy --long`, as a background call: `bgrun --wait -o LOG -- heavy --long -- mutate-check --diff ...`; llm-toolbox's heavy-gate refuses it in another lane.
Give a foreground `heavy --few-seconds` call a Bash timeout of 600000 ms, since it may wait behind a running job; llm-toolbox's heavy-gate refuses one without it.
Install nextest once per machine: `cargo install cargo-nextest --locked`, or the prebuilt binary from https://get.nexte.st/latest/linux.

A workflow names each action by a commit, with its version in a comment: `uses: actions/checkout@<40 hex> # v7.0.1`.
`bin/tests/test_workflow_supply_chain.py` refuses a tag or a branch there, an action that installs a tool at no named version, and a cache in a job that builds what ships, publishes or deploys (the test's `SHIPPED`): its docstring says why.
To add an action or move a pin by hand, resolve the tag with `gh api repos/OWNER/REPO/git/ref/tags/TAG`, following an annotated tag (`object.type` is `tag`) through `git/tags/SHA` to its commit, and read the action at that commit (`gh-src OWNER/REPO SHA`).
`.github/dependabot.yml` moves the pins and `Cargo.lock` by pull request, at most one a week for each, of releases at least a week old.
A merged one reaches `origin/main` without wt-ship, deploys staging as any push does, and leaves the main tree behind until it is pulled; one that changes `crates/analyzer/Cargo.toml` is an analyzer change at the next release (Code, `crates/cli`).
Dependabot doesn't see the tools pinned in scripts and workflow inputs, which move by hand: `bin/lychee`, `bin/caddy`, `bin/cargo-audit`, each `uvx tool@version`, and `taiki-e/install-action`'s `tool:` versions, which must be ones the pinned action's `manifests/` list.

`.github/workflows/advisories.yml` checks `Cargo.lock` against RustSec's advisories each day, with `bin/advisories` (`-h`): cargo-audit, which `bin/cargo-audit` downloads pinned by SHA-256 as `bin/lychee` does lychee, and one issue, "Cargo.lock holds crates with security advisories", that the workflow opens, keeps current and closes.
To see what applies now, run `bin/advisories check`.
To accept an advisory, list its id under `[advisories]`, `ignore`, in `.cargo/audit.toml`, with why in a comment; closing the issue by hand accepts the ones it lists.
Both reporters, this one and `bin/caddy-latest`, count only issues and comments `github-actions[bot]` wrote, since anyone can open an issue with a title a report will use.

Mutation testing (cargo-mutants, set in `.cargo/mutants.toml`) runs in CI, in `.github/workflows/mutants.yml`.
Each day it tests the mutants in the lines changed since the last run that tested all of its own, and skips when nothing changed.
A full run takes about 10 runner-hours, so it runs only when started by hand: `gh workflow run mutants.yml`.
The workflow's header says where the results land, and the `mutants-triage` skill turns a run into the few issues worth a test, then marks the run triaged in a comment on #19.
To check that a new test catches a mutant the run missed, write the change its line in `missed.txt` names as an apply-edits block and run it through `mutate-check` (`-h`), which takes seconds.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script.
The version and hashes live only there, and its docstring says how to move to a new release.
The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.
So is a commit there that stages a shell script while uv's cache lacks the pinned shellcheck; `SKIP=shellcheck git commit` skips only that one, and CI still runs it.
The same goes for zizmor and a staged workflow: `SKIP=zizmor git commit`.
To run one of the commit's checks by hand at the version the hook pins, such as shellcheck, actionlint or zizmor, run it through pre-commit: `uvx pre-commit@4.7.0 run <hook id> --files PATH...`, with the hook id from `.pre-commit-config.yaml`.
Run ruff by hand as `bin/ruff` (`bin/ruff check --fix PATH`, `bin/ruff format PATH`), which runs the version the hooks pin, never a bare `uvx ruff`: a newer ruff checks rules the hook doesn't, and its `--fix` once rewrote 28 files no change touched.

`bin/caddy` does the same for Caddy, pinned by SHA-512, and runs it with the given arguments.
The tests in `bin/tests` that run a real Caddy against `ops/remote/caddy.sh`'s and `ops/remote/field-test-key.sh`'s config get it through conftest's `real_caddy` fixture, or from `$FILEDRAWER_TEST_CADDY` when that is set.
Without network access they skip and say why, except in CI, where they fail.

shellcheck refuses errors, warnings and infos, not style.
To keep a finding that's intended, put `# shellcheck disable=SCnnnn  # why` on the line before it, as `ops/smoke-staging` does; a directive right after the shebang covers the file, as its SC2016 one does, since its `check` calls single-quote their conditions on purpose.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run`, `bin/slop-lint` and `bin/tests/leftover_processes.py` are copies of llm-toolbox's. Update them by copying from there.
