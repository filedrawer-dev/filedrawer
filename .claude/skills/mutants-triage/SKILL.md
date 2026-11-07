---
name: mutants-triage
description: Triage a run of .github/workflows/mutants.yml (cargo-mutants) and pick the 1 to 3 missed mutants most worth a test, if any. Use when asked to analyze, triage or report on a mutants run, or when one of its shards failed.
---

# Mutants triage

A mutants run reports hundreds of missed mutants: code no test checks. Most don't matter. The job is to find the **few gaps** where a wrong line would let something real through, and turn each into an issue a later session can close with a test. Zero is a valid answer. The owner asked for "the top ~1-3 things worth addressing, if any."

## 1. Check the run finished

`gh run list --repo filedrawer-dev/filedrawer --workflow mutants.yml --limit 3` gives the run, its commit, and the run before it. A run still going: wait with `gh run watch RUN --repo filedrawer-dev/filedrawer -i 300 --exit-status >/dev/null` as a background call.

The summary job's page says "Tested X of Y mutants." When X < Y, a shard died: go to **A shard that died** below before triaging, since the missing mutants are often the ones that matter.

Done when X = Y, or you know why not.

## 2. Fetch the lists

Download into the host disk, never the sandbox's:

```sh
D=/home/carver/code/claudedev/cache/cargo-mutants/run-RUN
gh run download RUN --repo filedrawer-dev/filedrawer -n mutants -D "$D"
```

`$D` holds `missed.txt`, `caught.txt`, `timeout.txt`, `unviable.txt` (one mutant a line, `path:line:col: change`) and `summary.md`. Fetch the run before it the same way. Each shard's whole `mutants.out`, with every mutant's diff and log, is the artifact `mutants-shard-N`.

## 3. Compare and group

- **Against the last run**: line numbers move between commits, so compare on the file and the change, not the line: `sed -E 's/:[0-9]+:[0-9]+:/:/' missed.txt | sort`. A mutant missed in both runs is an old gap; one missed only now is new code, or a test that was dropped.
- **By file**: `cut -d: -f1 missed.txt | sort | uniq -c | sort -rn`.
- **Already filed**: `gh issue list --repo filedrawer-dev/filedrawer --search "Tests:" --state open`. A gap with an open issue is counted, not filed again.

## 4. Pick the gaps

Read the code at each candidate's lines at the run's commit (`git show SHA:path`), then rank by **consequence**: what breaks if that line were wrong, in this order.

1. A guard between a request and the filesystem, the database or a secret (a name check before a path, an auth check).
2. A decision the owner settled in `docs/design.md`: limits, windows, caps (the embargo's 3 months, the attachment cap).
3. Abuse resistance: rate limits, screening, queue fairness.
4. Validation and messages.

Pass over:
- a mutant that changes no behaviour (an equivalent mutant), or only text in a log or a `Display`;
- a timeout, since a loop a mutant makes endless is caught in effect;
- code another session is changing now (`git log --since=2.days -- PATH`), which its own tests may cover soon.

Several mutants in one function are one gap. Done when you hold at most 3 gaps, each with the mutants that show it and one sentence on what a wrong line would let through.

## 5. File each gap

One issue each, titled `Tests: <the behaviour>`, holding:
- the run's link and commit;
- the surviving mutants, as `missed.txt` prints them;
- what a wrong line would let through;
- a `- [ ]` item per test, with literal expected values (first submitted 2026-11-30, latest allowed 2027-02-28), at the seam the crate's tests already use.

Then comment on #19: the run's link, the counts table from `summary.md`, the comparison with the last run, the cost (runner-hours, longest shard), and the issues filed. Done when the comment names every gap's issue, or says none was worth one.

## A shard that died

A shard that ends with "The hosted runner lost communication", "received a shutdown signal", or a cancel in the middle of the mutants step, with no failed step, most likely ran out of memory. Some mutants make a loop grow a buffer without end, and the shard step's `ulimit -d` exists to turn those into caught mutants (see its comment in `mutants.yml`). Shard assignment is deterministic, so the same shard dies again on a rerun.

- `gh run view --log --job JOB` answers only once the whole run is complete, and a runner that lost contact leaves no log; `gh api repos/filedrawer-dev/filedrawer/check-runs/JOB/annotations` still gives the reason.
- `gh run rerun --failed` reruns on the original commit's workflow, so a fix to `mutants.yml` needs a new `gh workflow run mutants.yml`.
- To find the culprit, reproduce the shard rather than guess: a worktree at the run's commit, then, as a background call, `bash -c 'ulimit -d 4194304; cargo mutants --in-place --shard N/32 --sharding round-robin -o OUT'` with `TMPDIR` and `OUT` under `/home/carver/code/claudedev/cache/cargo-mutants/`. `--in-place` mutates the worktree itself, so no copy of the tree lands on the sandbox's small disk. Afterwards, `grep -l "memory allocation of" OUT/mutants.out/log/*` names the mutant.
