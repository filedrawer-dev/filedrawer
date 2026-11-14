# Running a field test

A field test has agents write real entries from real research repos, so the entry format, the skills or the card can be tried on results nobody shaped for them.
Build step 4's usability runs on staging are one.
The first one, three rounds over the owner's five Apart repos in November 2026, is recorded in [entry-format.md](entry-format.md#field-test-findings).
It cost about a billion input tokens across 13 field-test agents and the 20 or so helpers they spawned, mostly because of the habits this page replaces.

## Scope each round

- Inventory a repo fully only once, in the first round.
  Later rounds start from that inventory and the earlier entries.
- After the first round, test on a sample: 10 to 15 entries per repo, covering every `kind` and `outcome` the repo has, the largest family, and the entries that fought the format hardest last time.
  One repo produced 91 entries in each of rounds 2 and 3, which the format question didn't need.
- Recheck one Results row in ten against the source, and have the agent say which.
  Round 3 rechecked about 1,900 rows.
- Cap helpers at 2 per field-test agent, and allow none for a sample.
  Agents that may split work multiply the cost: one agent spawned seven forks.

## Brief

Each agent gets a brief that stands on its own.
It names:

- the repo, read-only, and what read-only allows (reading, read-only git, `sha256sum`), and what it doesn't (writing, running experiments, installing);
- the docs to read first, by section;
- the scope rules in force, which change between rounds, so list them each time;
- the entries to write and where, under `field-test/round<n>/<repo>/`, which is git-excluded and local only;
- every number from a source file, cited by path, and nothing invented;
- for a run on staging, the staging key from `ops/.vps.env`, and that every request sends it as `X-Staging-Key` beside the API token, since staging's basic auth would otherwise take the `Authorization` header;
- `apply-edits` for text replacements, since the `edit-script-gate` hook refuses Python scripts that read, replace and write a file (it did so 25 times in the first test);
- the report rules below.

## Agents' own fetch tools on staging

A run that reads staging through an agent's built-in fetch tool, such as Claude Code's WebFetch or Codex's fetch, can't send the staging key's header, so Caddy refuses it.
For such a run the owner opens a field-test key with `ops/field-test-key open` on their machine (`-h`; `--hours N`, 24 by default, at most 72).
It writes the key to `/home/carver/code/claudedev/keep/filedrawer/field-test-key/key` as `key=` and `expires=` lines.

- Before the run, read the file, check that `expires` is still ahead, and probe it with curl: `/tags?key=<key>` should answer 200.
  A `closed=` or `failed=` line means no key from this file is open, and `expires=unknown` that the owner's last open got no answer, so ask them to close it.
- Give each URL in full with the key in its query, `?key=<key>` or `&key=<key>`, before any `#`.
  A link followed from a page loses it, and so does a redirect the server answers (to `/login` or `/account`), so the scripted turns hand over every URL.
- The key lets through only `GET` and `HEAD`, on staging and its files host (attached files), and not `/staging*` or `/admin*`.
  Submitting still goes through the skills, with the staging key in the header file.
- A 403 that names the field-test key means it expired, was replaced, or the request isn't a read of a page.
- Every agent in the sandbox shares one IP, so the server's per-IP limit counts their requests together.
- Some fetch tools fetch from their vendor's servers, which then hold the URL and its key, and Claude Code's WebFetch caches pages for 15 minutes.
- When the run is over, ask the owner to run `ops/field-test-key close`.

Any agent running as the sandbox's user can read the key file, as it can `ops/.vps.env`.
The test agents of a scripted usability run can't: drive-agent runs each as a user of its own, in a box that hides the workspace.

## Scripted usability runs

A usability run has Claude Code or Codex, in its own TUI, follow a scripted human through a scenario, such as installing the skills, then dropping a direction whose null result the submit skill should offer to post.
`ops/usability-run SCENARIO HARNESS NAME` runs one (`-h`), through llm-toolbox's `drive-agent`, which types each human turn once the agent's turn ends and has a simulated human answer what the script leaves open.

- Each run gets a fresh staging account, signed up as a person would, with the usability mailbox and invite code in `keep/filedrawer/usability/mail.env`, and deleted at the end; so no run finds another's entries.
  An entry that went public before the end stays up; `ops/remove-entries` (`-h`) takes down the ones the run listed, as the owner, who runs it.
- The scenarios, the persona and the synthetic projects live in `keep/filedrawer/usability/`, not the repo, which agents under test may browse.
- The agent runs as a system user named NAME in bwrap: it sees only its home, its network goes through a proxy that refuses GitHub and loopback, and nothing tells it it is tested.
  Use persona-like names (`jason7`), since the agent sees its user's name.
- Its turns use the field-test key, `{KEY}` in a scenario, for fetch tools; the run refuses a key that is closed or expires within 2 hours.
- Run it with `bgrun --wait -o LOG`, as each turn may take 20 minutes.
  `ops/usability-run batch SCENARIO:HARNESS:NAME...` starts several runs at once, each logging to `logs/` in the usability folder, and prints one line a run once all have ended.
- To read runs, start with `ops/usability-run report RUN_DIR...`: at most 15 lines a run, with keys and tokens blanked, saying how `/skill.md` was fetched, what got installed, the upgrade mode, the site and installer commands, and any write to the site or call naming production.
  `drive-agent digest RUN_DIR --calls` lists each human turn and tool call on one line, and `drive-agent digest RUN_DIR` adds the agent's text; both blank keys and tokens by default.
  `ops/usability-run headers RUN_DIR` lists a run's fetches of the site, each with its User-Agent, its skill header and whether it sent the header file.

## Reports

The harness refuses report-like files written by subagents, such as `FINDINGS.md`.
So:

- The agent's final message carries the findings, under an exact first line such as `# Field test round 2: action-channels`, in at most about 1,500 words: the frictions ranked by cost, the fixes they suggest, and what was unclear in scope.
- Bulky material goes into data files beside the entries: cards in `CARDS.txt`, the inventory in `inventory.csv`, per-entry tables in CSV.
  These write fine.
  Reason: in the first test each agent returned 10,000 to 40,000 tokens of findings, cards included, and the main session spent about 300,000 tokens of its context reading them.
- Save the findings to `field-test/round<n>/<repo>/FINDINGS.md` with a short script that reads the agent's transcript, the task's output file, and keeps the text from the heading on.
  Retyping it costs output tokens.

## After the round

- Record the findings in the doc being tested, ranked by how many repos hit each, then put the proposed changes to the owner.
- After a round that removes a concept from the docs, grep `design.md`, `entry-format.md` and `AGENTS.md` for its terms before the cold review.
  Every cold review in the first test found leftovers of removed rules.
