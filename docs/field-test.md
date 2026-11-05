# Running a field test

A field test has agents write real entries from real research repos, so the entry format, `/skill.md` or the card can be tried on results nobody shaped for them. Build step 4's usability runs on staging are one. The first one, three rounds over the owner's five Apart repos in November 2026, is recorded in [entry-format.md](entry-format.md#field-test-findings). It cost about a billion input tokens across 13 field-test agents and the 20 or so helpers they spawned, mostly because of the habits this page replaces.

## Scope each round

- Inventory a repo fully only once, in the first round. Later rounds start from that inventory and the earlier entries.
- After the first round, test on a sample: 10 to 15 entries per repo, covering every `kind` and `outcome` the repo has, the largest family, and the entries that fought the format hardest last time. One repo produced 91 entries in each of rounds 2 and 3, which the format question didn't need.
- Recheck one Results row in ten against the source, and have the agent say which. Round 3 rechecked about 1,900 rows.
- Cap helpers at 2 per field-test agent, and allow none for a sample. Agents that may split work multiply the cost: one agent spawned seven forks.

## Brief

Each agent gets a brief that stands on its own. It names:

- the repo, read-only, and what read-only allows (reading, read-only git, `sha256sum`), and what it doesn't (writing, running experiments, installing);
- the docs to read first, by section;
- the scope rules in force, which change between rounds, so list them each time;
- the entries to write and where, under `field-test/round<n>/<repo>/`, which is git-excluded and local only;
- every number from a source file, cited by path, and nothing invented;
- `apply-edits` for text replacements, since the `edit-script-gate` hook refuses Python scripts that read, replace and write a file (it did so 25 times in the first test);
- the report rules below.

## Reports

The harness refuses report-like files written by subagents, such as `FINDINGS.md`. So:

- The agent's final message carries the findings, under an exact first line such as `# Field test round 2: action-channels`, in at most about 1,500 words: the frictions ranked by cost, the fixes they suggest, and what was unclear in scope.
- Bulky material goes into data files beside the entries: cards in `CARDS.txt`, the inventory in `inventory.csv`, per-entry tables in CSV. These write fine. Reason: in the first test each agent returned 10,000 to 40,000 tokens of findings, cards included, and the main session spent about 300,000 tokens of its context reading them.
- Save the findings to `field-test/round<n>/<repo>/FINDINGS.md` with a short script that reads the agent's transcript, the task's output file, and keeps the text from the heading on. Retyping it costs output tokens.

## After the round

- Record the findings in the doc being tested, ranked by how many repos hit each, then put the proposed changes to the owner.
- After a round that removes a concept from the docs, grep `design.md`, `entry-format.md` and `AGENTS.md` for its terms before the cold review. Every cold review in the first test found leftovers of removed rules.
