# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents. Most are null or inconclusive, and the pitch leads with those. The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: building. The owner and an agent settled the design on 2026-11-03 and 2026-11-04, and the owner signed off on the entry format's fourth draft on 2026-11-05. The analyzer and the server's agent read/write loop exist; accounts, `/skill.md` and staging are next.

## Next steps

1. **Build** in the order given in `docs/design.md`, under Build order, continuing step 2 in slices the owner chose. Done: accounts, submission with the hour's hold, versions, batches with slugs and runs, series (create, join, cite a version, versions of the description, made public from a browser session, `waiting_on_series`, key rotation, removing members and entries), account deletion. Backups wait for launch (issue #4). Build step 3 is planned in issue #3; the owner split its slice 1 in two. Done: 1a, the screener for entries and batches (Haiku and Sonnet, verdicts as statuses, the cost log and caps, the queue worker), slice 4, the owner's review queue at `/admin` (`filedrawer admin grant` makes the admin), slice 3 apart from what waits on invite codes (the fair queue; the review guard sends a guarded account's entries to review unscreened), 1b (run declarations screened with the entries that use them, the wizard's screening stage, and series the screen refuses as a likely hazard kept for the owner to confirm or dismiss), and slice 2 for entries, series and runs (long encoded runs that decode to text are refused rather than decoded; the owner's decision, 2026-11-06). The owner also made the screen gentler on 2026-11-06 (design.md, Submission and screening): topic and spam fail only when plain, a hazard goes to review only when Sonnet names and quotes it, and the review guard trips on the owner's reject or hold. Slice 5a, attached code, is done: attachment sets uploaded once with `POST /attachments` and cited as `attachments: as-...`, screened by hash in chunks, served from the files host (`FILEDRAWER_FILES_URL`; staging's DNS and Caddy steps are issue #10). Their bytes live on disk in `files/` beside the database, not in SQLite (#12); sets no entry cites go after 7 days, and the files' backups wait for #4. Slice 5b is done too: encoded and compressed data in attached files is unpacked (`crates/server/src/unpack.rs`, at most 3 layers and 4 MiB a file, and 4 times the entry's files' size in text, at least 256 KiB) and screened after the file. Next: 5c, banned hashes, the owner removing a file, and purge; then the rest of build step 4 (invite codes, and the write rate limit in issue #14). The daily allowance (#8) is in: accepted submissions and series take units dated by UTC day, and an entry waits in `screening` until its unit's day (`crates/server/src/allowance.rs`). Staging keeps `FILEDRAWER_SCREENER=pass-all` until the owner runs `ops/provision-vps screening`, which stores its API key and daily cap on the server (`ops/remote/screening.sh`); `ops/provision-vps admin` makes the owner's staging account the admin. `crates/server/skill.md` is what agents read at `/skill.md`; it describes only what the server does, so update it with each slice that adds a route. `CODING_STANDARDS.md` holds the rules the review applies.
2. **Settle `uncertainties.md`** with the owner: choices the docs left open while building.

The owner has root on the VPS but keeps SSH keys out of the sandbox. Anything done on the server is a tool the owner runs from their own machine, such as `ops/provision-vps`.

## Deploys

A push to `main` that passes CI deploys to staging (`build-staging` and `deploy-staging` in `.github/workflows/ci.yml`). The build job makes a static x86_64 musl binary; the deploy job, which runs no third-party action, pipes it over SSH as the `deploy` user. That user's key may only run `ops/remote/filedrawer-receive`, which allows one deploy at a time and passes the upload on stdin to `ops/remote/filedrawer-install` through one sudo rule. The install script keeps its own root-owned copy, checks the size and hash, runs the binary's `-h` as the service user with a timeout, keeps the previous binary, and rolls back if `/tags` doesn't answer within 30 s. `ops/provision-vps` sets up the user and key, stores `DEPLOY_SSH_KEY` as a secret of the `staging` environment (main only), and writes the variables `VPS_HOST` and `VPS_KNOWN_HOSTS`; rerunning it replaces the key. A binary that answers `/tags` passes, and it runs as `filedrawer` with staging's data and settings. Production deploys come at launch.

The field test's entries and logs are in `field-test/`, git-excluded and local only; never commit or publish them without the owner's say.

## Code

- `crates/analyzer`: reads an entry by `docs/entry-format.md`, lists what would refuse it and what the server couldn't read, and renders its card with analyzer v1. The server and the CLI share it. Reason: in the field test every agent wrote its own parser and card renderer, a dozen in all, and each read the spec differently.
- `entry-check FILE...` checks entries offline, for contributing agents and field tests: `cargo run -q --bin entry-check -- FILE...`. It exits 1 when the server would refuse an entry. Run `bin/field-test-diff save` before an analyzer change and `bin/field-test-diff` after it (`-h`): it lists the field-test entries whose outcome changed, without their contents.
- `crates/server`: the `filedrawer` binary. `filedrawer -h` lists its commands and settings. Slices 1 and 2 of build step 2 are in:
  - the admin CLI that makes accounts and tokens;
  - `POST /entries`, `/e/<id>`, `/search`, `/cards.txt`, `/tags` and `POST /feedback`;
  - accounts in the browser: signup, email verification, login with server-side sessions, password reset, and the account page that makes and revokes tokens.

  Screening is a pass-all stub until build step 3, and production refuses to start with it or without SMTP settings. Email goes to the log when no SMTP settings are set.
- `ops/smoke-staging` follows `/skill.md` against staging as an agent would, and exits 1 when a check fails (`-h`). CI runs it after each deploy, as a smoke-test account that `ops/provision-vps smoke` sets up and names to staging as `FILEDRAWER_SMOKE_ACCOUNT`, whose entries aren't screened, so the smoke test calls no model; `crates/server/tests/smoke.rs` runs it against a local server, so a reply it relies on can't change unnoticed. To run it by hand, set `FILEDRAWER_STAGING_KEY` (STAGING_KEY in `ops/.vps.env`) and `FILEDRAWER_STAGING_API_KEY` (a token). `ops/smoke-staging screen` checks that staging screens with the real model, as the screen-test account `a-dmkmsy91w3` (robocarver's mailbox), whose token is `SCREEN_TEST_TOKEN` in the git-ignored `ops/.screen-test.env`. CI runs it after each deploy (the `screen-staging` job; the owner's decision, 2026-11-07), for a few Haiku calls, with that token as a secret of the `staging` environment, which `ops/provision-vps smoke` stores. Without the secret, or on a day whose screening cap is spent, the job passes with a warning. `ops/smoke-staging eval` runs the labelled cases in `ops/screen-eval/` and a sample of field-test entries through staging's screen, to see whether it is too strict or too lax; it runs only by hand. Staging doesn't hold that account to an account's share of the screening cap (`FILEDRAWER_SCREEN_TEST_ACCOUNT`, which `ops/provision-vps smoke` sets from `ops/.screen-test.env`).

### SQL

Queries go through sqlx's checked macros (`CODING_STANDARDS.md`), which compile against `.sqlx/`, the committed query data, when `DATABASE_URL` is unset, as in CI and the commit hook: `bin/agent-pre-commit` builds a copy of the index with no `.env`, so a stale `.sqlx/` refuses the commit. After adding or changing a query or a migration, run `bin/sqlx-refresh`. It rebuilds `target/dev.db` from the migrations (writing `.env` if it's missing), so an edited, unpushed migration needs no cleanup, then runs `cargo sqlx prepare --workspace`.

`sqlx` is sqlx-cli: `cargo install sqlx-cli --no-default-features --features sqlite,rustls --locked`.

Open work items, such as checks to run before launch, are GitHub issues on `filedrawer-dev/filedrawer`. Read them before you plan work, and file new ones there rather than in these docs.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives. Read it before you change behaviour. Reviving a rejected alternative needs the owner's agreement.
- `docs/entry-format.md`: the fields, `outcome`, the Results table and the card. Read it before you touch submission, the analyzer, cards or `/skill.md`.
- `docs/prior-art.md`: related projects, with the date of the survey. Read it before claiming novelty or adding a feature that may already exist elsewhere.
- `docs/field-test.md`: how to run a field test with agents cheaply. Read it before you brief agents to write entries, as in the usability runs on staging.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

## Checks

Committing runs the checks, so commit and read the hook's output instead of running them first. Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, and ruff and `cargo fmt` (when a `.rs` file is staged), which write: a person's commit that needs formatting is refused with the files rewritten, to stage, and `bin/pre-commit-run` stages an agent's and checks again. Agents' commits add the tests in `bin/tests`, clippy and the Rust tests (`bin/agent-pre-commit`). CI runs all of them. `python3 install.py` links the hooks once per clone and installs cargo-nextest and sqlx-cli; sandbox-setup runs it after a recreate.

The Rust tests run under cargo-nextest, set in `.config/nextest.toml` to print only the tests that fail, with their output, and a summary line. To run them that way, run `cargo nextest run --cargo-quiet`. nextest skips doctests, so the hook also runs `cargo test --doc`. Install nextest once per machine: `cargo install cargo-nextest --locked`, or the prebuilt binary from https://get.nexte.st/latest/linux.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script. The version and hashes live only there, and its docstring says how to move to a new release. The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run` and `bin/slop-lint` are copies of llm-toolbox's tools. Update them by copying from there.
