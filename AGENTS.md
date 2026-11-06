# File Drawer

An archive at filedrawer.dev of AI safety results that won't become papers, written and read by agents. Most are null or inconclusive, and the pitch leads with those. The GitHub org is `filedrawer-dev`, and the repo will be `filedrawer-dev/filedrawer`.

Status: building. The owner and an agent settled the design on 2026-11-03 and 2026-11-04, and the owner signed off on the entry format's fourth draft on 2026-11-05. The analyzer and the server's agent read/write loop exist; accounts, `/skill.md` and staging are next.

## Next steps

1. **Build** in the order given in `docs/design.md`, under Build order, continuing step 2 in slices the owner chose. Done: accounts, submission with the hour's hold, versions, batches with slugs and runs. Next session: series (design.md, Series: create, join, cite a version, versions of the description, made public from a browser session, `waiting_on_series`), then account deletion with key rotation and removing members. Then build step 3, planned in issue #3. `crates/server/skill.md` is what agents read at `/skill.md`; it describes only what the server does, so update it with each slice that adds a route. `CODING_STANDARDS.md` holds the rules the review applies.
2. **Settle `uncertainties.md`** with the owner: choices the docs left open while building.

The owner has root on the VPS but keeps SSH keys out of the sandbox. Anything done on the server is a tool the owner runs from their own machine, such as `ops/provision-vps`.

## Deploys

A push to `main` that passes CI deploys to staging (`build-staging` and `deploy-staging` in `.github/workflows/ci.yml`). The build job makes a static x86_64 musl binary; the deploy job, which runs no third-party action, pipes it over SSH as the `deploy` user. That user's key may only run `ops/remote/filedrawer-receive`, which allows one deploy at a time and passes the upload on stdin to `ops/remote/filedrawer-install` through one sudo rule. The install script keeps its own root-owned copy, checks the size and hash, runs the binary's `-h` as the service user with a timeout, keeps the previous binary, and rolls back if `/tags` doesn't answer within 30 s. `ops/provision-vps` sets up the user and key, stores `DEPLOY_SSH_KEY` as a secret of the `staging` environment (main only), and writes the variables `VPS_HOST` and `VPS_KNOWN_HOSTS`; rerunning it replaces the key. A binary that answers `/tags` passes, and it runs as `filedrawer` with staging's data and settings. Production deploys come at launch.

The field test's entries and logs are in `field-test/`, git-excluded and local only; never commit or publish them without the owner's say.

## Code

- `crates/analyzer`: reads an entry by `docs/entry-format.md`, lists what would refuse it and what the server couldn't read, and renders its card with analyzer v1. The server and the CLI share it. Reason: in the field test every agent wrote its own parser and card renderer, a dozen in all, and each read the spec differently.
- `entry-check FILE...` checks entries offline, for contributing agents and field tests: `cargo run -q --bin entry-check -- FILE...`. It exits 1 when the server would refuse an entry.
- `crates/server`: the `filedrawer` binary. `filedrawer -h` lists its commands and settings. Slices 1 and 2 of build step 2 are in:
  - the admin CLI that makes accounts and tokens;
  - `POST /entries`, `/e/<id>`, `/search`, `/cards.txt`, `/tags` and `POST /feedback`;
  - accounts in the browser: signup, email verification, login with server-side sessions, password reset, and the account page that makes and revokes tokens.

  Screening is a pass-all stub until build step 3, and production refuses to start with it or without SMTP settings. Email goes to the log when no SMTP settings are set.
- `ops/smoke-staging` follows `/skill.md` against staging as an agent would, and exits 1 when a check fails (`-h`). CI runs it after each deploy, as a smoke-test account that `ops/provision-vps smoke` sets up; `crates/server/tests/smoke.rs` runs it against a local server, so a reply it relies on can't change unnoticed. To run it by hand, set `FILEDRAWER_STAGING_KEY` (STAGING_KEY in `ops/.vps.env`) and `FILEDRAWER_STAGING_API_KEY` (a token).

### SQL

Queries go through sqlx's checked macros (`CODING_STANDARDS.md`), which compile against `.sqlx/`, the committed query data, when `DATABASE_URL` is unset, as in CI and the commit hook. To add or change a query, point `DATABASE_URL` at a migrated database in a git-ignored `.env`, then refresh `.sqlx/`:

```sh
echo "DATABASE_URL=sqlite://$PWD/target/dev.db" > .env
sqlx database create && sqlx migrate run --source crates/server/migrations
cargo sqlx prepare --workspace
```

`sqlx` is sqlx-cli: `cargo install sqlx-cli --no-default-features --features sqlite,rustls --locked`.

Open work items, such as checks to run before launch, are GitHub issues on `filedrawer-dev/filedrawer`. Read them before you plan work, and file new ones there rather than in these docs.

## Docs

- `docs/design.md`: every settled decision with its reason, plus open questions and rejected alternatives. Read it before you change behaviour. Reviving a rejected alternative needs the owner's agreement.
- `docs/entry-format.md`: the fields, `outcome`, the Results table and the card. Read it before you touch submission, the analyzer, cards or `/skill.md`.
- `docs/prior-art.md`: related projects, with the date of the survey. Read it before claiming novelty or adding a feature that may already exist elsewhere.
- `docs/field-test.md`: how to run a field test with agents cheaply. Read it before you brief agents to write entries, as in the usability runs on staging.

When the owner settles a question, move it from Open questions into its section of `docs/design.md`.

## Checks

Committing runs the checks, so commit and read the hook's output instead of running them first. Every commit runs `bin/slop-lint` on staged markdown, lychee (links and anchors, offline) over every markdown file, and `cargo fmt --check` when a `.rs` file is staged. Agents' commits add the tests in `bin/tests`, clippy and the Rust tests (`bin/agent-pre-commit`). CI runs all of them. `python3 install.py` links the hooks once per clone and installs cargo-nextest and sqlx-cli; sandbox-setup runs it after a recreate.

The Rust tests run under cargo-nextest, set in `.config/nextest.toml` to print only the tests that fail, with their output, and a summary line. To run them that way, run `cargo nextest run --cargo-quiet`. nextest skips doctests, so the hook also runs `cargo test --doc`. Install nextest once per machine: `cargo install cargo-nextest --locked`, or the prebuilt binary from https://get.nexte.st/latest/linux.

`bin/lychee` downloads lychee once per machine and checks it against the SHA-256 pinned in the script. The version and hashes live only there, and its docstring says how to move to a new release. The first commit on a machine without network access is refused; `SKIP=lychee git commit` skips only that check.

To keep a line `slop-lint` flags, end it with `<!-- slop-lint: ignore -->`. `bin/pre-commit-run` and `bin/slop-lint` are copies of llm-toolbox's tools. Update them by copying from there.
