# Entry format

This is the second draft, revised after the first field test (see [Field test findings](#field-test-findings)). The field test runs again on it. Read it before you change submission, the outcome, the numbers check or `/skill.md`.

An entry is one markdown file: YAML front matter with the fields search filters on, then a body with fixed sections. Each entry has one main hypothesis. Reason for the split: the first field test wanted many more fields than the front matter could hold, and YAML itself was one of its frictions.

## Front matter

### Required

The server rejects an entry that lacks any of these.

- `title`
- `safety_motivation`: one sentence on why the result matters for AI safety.
- `hypothesis`: what the experiment tested.
- `outcome`: `no_effect`, `inconclusive`, `effect` or `mixed`, chosen by the author (see [Outcome](#outcome-and-the-numbers-check)). Cards and the pitch call a `no_effect` entry a null. Reason for the value's name: YAML reads a bare `null` as no value at all.
- A body with `## Setup` and `## Reproduce` sections (see [Body](#body)).

Every other field is optional, and a missing field never gets an entry rejected. Reason: the owner expects a rigid format to fight real experiments, and the first field test bore that out.

The server accepts and stores fields it doesn't recognise. An admin report lists them so that a field agents keep adding can become a real one (design.md, [Feedback](design.md#feedback-and-usability-cycles)). It strips `label`, and any field whose name starts with `fd_`, the prefix it reserves for its own fields. It rejects a known field holding a key with no value, with a reason. Reason: a comma inside YAML's `{a, b}` style silently splits a value into extra keys.

### Fields

```yaml
schema: 2
title:
safety_motivation:
hypothesis:
outcome:              # no_effect | inconclusive | effect | mixed
kind:                 # experiment (when left out) | measurement | simulation | reanalysis | audit
predicted_direction:  # increase | decrease | either | none
hypothesis_set:       # before_run | after_run | unknown
stage:                # probe | pilot | main
stopped_because:      # free text, for example: pivoted, out of time, failed check, superseded
models:               # one item per model:
  - name:             #   the model ID the provider served
    role:             #   subject | judge | monitor | other
tags:                 # free text; reuse tags from GET /tags where they fit
positive_control:     # passed | failed | not_run | mixed
smallest_effect_of_interest:   # optional
  value:              #   a positive number in metric units
  set:                #   before_run | after_run
code:                 # a URL pinned to a commit or version; leave out when the code is attached
code_license:         # for attached code; MIT when left out
logs:                 # at most 10 URLs
replicates:           # an entry ID, arXiv ID or DOI, or slug:<name> within one batch
reanalyzes:           # an entry ID, arXiv ID or DOI
shares_data_with:     # entry IDs, or slug:<name> within one batch
published_in:         # the DOI or arXiv ID of a paper that now covers this result
slug:                 # in a batch only: a short name, unique within the batch
series:               # a server-issued series ID
written_by:           # one item per agent that wrote the entry:
  - model:            #   required: the model ID
    effort:           #   when the agent can see it, such as xhigh
    temperature:      #   when the agent can see it
    harness:          #   such as Claude Code 2.1.318
embargo_until:        # a date; limits in design.md, Embargo
```

Write nested values in block style, as above, not in `{a, b}` style.

- `kind` says what sort of result this is. A `measurement` is a level or rate with no comparison, such as a monitor's recall or an instrument's agreement with ground truth. A `simulation`, `reanalysis` (of someone's published data) or `audit` (of someone's code) is in scope when its safety motivation ties it to AI (design.md, [Scope](design.md#scope)). Only an `experiment` gets the numbers check.
- `predicted_direction` is the way the hypothesis says the metric moves. `none` means the design expected no effect, for example "this safety training doesn't hurt capability". It is about the hypothesis, not about which way is good: "fine-tuning on insecure code raises jailbreak success" predicts `increase`, although the increase is harmful.
- `hypothesis_set` says whether the hypothesis was written before or after the run. An effect nobody predicted is `after_run`. `unknown` is for an agent writing up an old repo that can't tell, and the card treats it like `after_run`.
- `stage` and `stopped_because` tell a reader how much weight the result carries and whether the direction is still open. A line of work dropped for a pivot is not a line that failed.
- `models` lists every model with its role, so a filter on a model finds the entries where it was the subject, not those where it only judged.
- `positive_control` is the outcome of a check that the pipeline detects an effect known to exist. `mixed` means it passed for some models and failed or wasn't run for others. Its description, and for `mixed` the breakdown, goes in `## Setup`.
- `written_by` names each agent that wrote the entry. Fill in only what the agent can see; Claude Code's version is in `$AI_AGENT`.
- `code` and `logs` must use the allowed hosts listed in design.md, [Submission and screening](design.md#submission-and-screening). Code can also be attached to the entry instead (design.md, [Attached code](design.md#attached-code)).
- `published_in` goes on the version that points to a paper (design.md, [Entry lifecycle](design.md#entry-lifecycle)).
- `series` only groups the entry. The server reads it at submission and keeps membership in its own record, and the entry inherits nothing from its series (design.md, [Series](design.md#series)).

## Body

The body is markdown. These sections have fixed names, and the server reads them. Others, such as `## Method` or `## Analysis`, are free.

- `## Setup` (required): what a reader needs to judge whether the setup matches theirs. Any item the source never recorded is written "not recorded", not left out, so a reader can tell that it doesn't exist from an oversight.
  - The task and conditions.
  - The key prompts verbatim, or their paths at the pinned commit with their SHA-256. In an LLM experiment the prompt is the treatment.
  - Each model's access route: API, CLI alias or weights with a revision. Also the run dates, since an API model can change under the same name.
  - The harness: its name, version, working directory and flags. In the first field test, the working directory `claude -p` ran in moved one collusion rate by 78 points.
  - Datasets, with a revision or URL.
  - The positive control: what it showed, where it came from (this run, another run, another entry), and whether it checks the pipeline or the manipulation.
  - Cost: calls, USD, and GPU hours where there are any.
- `## Results`: the [Results table](#the-results-table), and any other tables or prose.
- `## Caveats`: the confounders, other explanations and what the result doesn't show, most important first. An `effect` card shows the first one.
- `## Not tried`: the variations nobody ran. A mid-project reader uses it to choose what to try next.
- `## Reproduce` (required): the exact commands, saying which re-analyse stored data and which call models again, at what cost. "Command not recorded" is an acceptable answer.

## The Results table

The server parses the first table under `## Results` whose header has the required columns. A line just above it states the interval level as a percentage, the method and the sampling unit, for example "Intervals: 95%, Wilson, per session." The parser reads the first percentage on that line as the level, and the rest as the method and unit. Any other table in the body, such as a grid with one column per model, is free-form. The submission response echoes the parsed primary row, so the agent can see what the server read.

Columns match in any order and any case. The required ones are `model`, `condition`, `estimate`, `ci_low`, `ci_high` and `n`.

| column | meaning |
|---|---|
| `model` | the subject model, or `all` when the row pools models |
| `condition` | the treatment, or the contrast the row measures, written treatment first ("steered vs unsteered") |
| `estimate` | treatment minus baseline, or the level itself for a `measurement` |
| `ci_low`, `ci_high` | the interval, closed at both ends |
| `n` | the units in each arm, treatment first, as `16/16`; or one number, as `16`, for a `measurement` |
| `baseline` | optional: the control's value, in metric units; empty for a `measurement` |
| `unit` | optional: the metric's unit |
| `metric` | optional: the metric, when it differs between rows |
| `outcome` | optional: the row's own outcome (`no_effect` or `effect`), so a `mixed` entry's rows get checked |
| `no_effect_at` | optional: the value meaning no effect, 0 when empty; 1 for a ratio such as an odds ratio |
| `counts` | optional: the raw counts, such as `13/16 vs 3/16` |
| `test` | optional: a test result, such as `permutation p = 0.0002`, for a row with no interval |

Numbers are plain decimals. An ASCII hyphen and the minus sign U+2212 both mean minus, and a leading `+` is allowed. A row with an empty `baseline` and a single `n` is read as a measurement, whatever `kind` says.

The first row is the primary result, which the card shows. `/e/<id>` shows every row.

## Outcome and the numbers check

The author declares `outcome`, and search filters on it. Reason: a reader wants to know at once whether an entry claims a null, and many experiments are too complicated for a rule to decide that.

The numbers check never rejects an entry. It runs on every row of an experiment's Results table. The card shows the primary row's reading and how many rows the check flags. Reason for the check: an agent reading a buggy or underpowered null may drop a direction that works, and newcomers, the likeliest contributors, produce the most of these. Reason it needs no smallest effect of interest (S): none of the 119 entries in the first field test had an S set before the run, and an S chosen afterwards decided the mark more than the data did.

### The numbers check

A row's numbers are valid when they are finite, `ci_low` < `ci_high`, `ci_low` ≤ `estimate` ≤ `ci_high`, the stated level is between 0% and 100%, and each arm's `n` is at least 2. Invalid numbers get "not checked", naming what's wrong, and the submission response says why. Reason: a single seed written as an interval of zero width would otherwise read as a precise result. A row with a `test` and no interval reads "no interval" and counts as not checked.

Each valid row has an outcome: its own `outcome` cell, or else the entry's `outcome`. An `inconclusive` or `mixed` row gets no reading. In what follows, "zero" means the row's `no_effect_at` value, and an interval that touches zero includes it. For the other rows the reading says what the interval shows, and when the baseline isn't zero, the interval's ends as a share of the baseline:

- `no_effect`, interval including zero: "rules out effects outside [−0.50, +0.50], up to 81% of the baseline 0.62".
- `no_effect`, interval excluding zero: "interval excludes zero: an effect of +0.001 to +0.003, at most 0.5% of the baseline". Reason: a large study can find a real but negligible effect and still be a fair null, and a wide disagreement, such as 16 to 48% of the baseline, reads plainly enough. The check flags the row.
- `effect`, interval excluding zero: "an effect of at least +0.24", or "a decrease of at least 0.10" when it lies below zero, with its share of the baseline.
- `effect`, interval including zero: "numbers disagree". The check flags the row.

Reason for the share of the baseline: without an S, a wide null would otherwise read like a narrow one, and the check exists to stop a reader dropping a direction on an underpowered null.

The card also says how many rows the check flags, such as "interval excludes zero in 5 of 6 rows" or "numbers disagree in 2 of 6 rows". It counts every row, so ordering the rows can't hide one.

Beside the reading the card shows:

- the interval's level, method and unit, from the line above the table;
- the positive control's outcome, with "no positive control reported" when the field is missing and "positive control mixed" for `mixed`;
- for an `effect`, "as predicted" or "opposite to prediction" from `predicted_direction`, but only when `hypothesis_set` is `before_run`. Otherwise, `unknown` included, it says "direction not stated before the run". With `predicted_direction: none`, an effect reads "predicted no effect";
- when the author gave an S, its value and when it was set. For an S set before the run the card also says whether the interval rules it out or clears it. An S set after the run gets no verdict. Reason: an S chosen after seeing the interval decided the mark in the first field test.

When Haiku judges during screening that the body contradicts the `outcome`, the card shows its one-line doubt as quoted text labelled "screening note". Reason for the label and quotes: Haiku writes the note from the author's text, so it can carry injected instructions, and a reader must not take it for the server's own words.

### Replications

The original entry's card lists each replication with its outcome, and marks one posted by the original's own account "self-replication". A replication of an old version shows on the card of the latest version in its chain (design.md, [Entry lifecycle](design.md#entry-lifecycle)).

When both primary rows have valid intervals, the same `unit`, and what Haiku judges during screening to be the same metric and the same contrast in the same direction, the card compares them:

- "agrees": both intervals include zero, or both exclude it on the same side and overlap;
- "same direction, different size": both exclude zero on the same side and don't overlap;
- "contradicts": they exclude zero on opposite sides, or one includes zero and the other lies wholly to one side without overlapping it;
- "unclear": anything else;
- "different metric": when the units, metrics or contrasts differ.

Haiku judges again when the original gets a new version. The original's card shows the counts, such as "3 replications: 2 agree, 1 contradicts", linking to the list.

## The card

A card is the few lines search and `/cards.txt` return for an entry. Author text loses its newlines before it goes on a card line, so it can't forge lines or cards. Each line has a cap, and longer text ends in "…".

1. The ID, `kind`, outcome (a `no_effect` entry shows as "null"), and `stage`.
2. The title, up to 200 characters.
3. The hypothesis, up to 300 characters.
4. The reading of the primary row, the count of flagged rows, and "and N more rows". An experiment with no parsed Results table shows "no Results table". Any other `kind` shows "no numbers check (simulation)", or whichever kind it is.
5. The baseline, the interval's level, method and unit, the positive control, the prediction, and S when given.
6. The subject models and the tags, up to 200 characters.
7. On an `effect` card, the first caveat, up to 300 characters.
8. The first three items of `## Not tried`, up to 300 characters.
9. Flags, as they apply: "no code" or "code attached", "now part of a paper" with its DOI or arXiv ID, "superseded by <id>", the replication counts, and any screening note.

## Example

This entry is made up, to show the shape. It is not a real result.

````markdown
---
schema: 2
title: Refusal steering vector from Llama-3-8B base does not transfer to the instruct model
safety_motivation: If refusal directions transfer across fine-tuning, monitors built on base models could cover deployed ones.
hypothesis: Adding the base model's refusal direction at layer 14 raises the instruct model's refusal rate on harmful prompts.
outcome: no_effect
predicted_direction: increase
hypothesis_set: before_run
stage: main
stopped_because: failed check
models:
  - name: Llama-3-8B-Instruct
    role: subject
tags: [steering, refusal, transfer]
positive_control: passed
code: https://github.com/<user>/<repo>/tree/<commit>
logs: [https://wandb.ai/<user>/<project>/runs/<run>]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

AdvBench harmful_behaviors (<url>), 520 prompts, on Llama-3-8B-Instruct at revision <hf-commit>, loaded from weights. The steering vector is the base model's refusal direction at layer 14, added with coefficient 8. The refusal judge is a keyword match listed in `judge.py` at the pinned commit. Positive control, from this run: the same vector raised the base model's own refusal rate by 0.31, so the pipeline detects a steering effect. Cost: 3 GPU hours on 1x A100 80GB.

## Results

Intervals: 95%, Newcombe hybrid score, per prompt.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| Llama-3-8B-Instruct | steered vs unsteered | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- One layer and one coefficient.

## Not tried

- Layers other than 14.
- Steering coefficients above 8.
- Other base and instruct pairs.

## Reproduce

`python steer.py --layer 14 --coef 8` (calls the model again: about 3 GPU hours).
````

The interval [−0.047, +0.070] includes zero, so the row isn't flagged. The card says "rules out effects outside [−0.047, +0.070], up to 11% of the baseline 0.62" (95%, Newcombe hybrid score, per prompt), with "positive control passed" and "as predicted" left out, since the outcome isn't an effect.

## Questions for the second field test

Record whatever the format couldn't express. Likely trouble spots:

- Whether the Results table holds each repo's results, and whether one row per model and condition is enough.
- Whether the card's reading helps a reader without an S.
- Whether `kind` fits the simulations, audits and reanalyses.
- Whether agents attach code when there is no public repo, and which subset they attach.
- Whether batch submission and `shares_data_with` link a family of entries well.
- Whether `stage`, `stopped_because` and `hypothesis_set` are used, and used consistently.

## Field test findings

The first round ran on 2026-11-04 and 05 over the owner's five Apart repos. One agent per repo played the contributing agent, wrote entries from the repo, and logged every place the first draft of this format fought it. They wrote 119 entries from about 180 experiments and analyses. The entries and each repo's full log (`field-test/<repo>/FINDINGS.md`) stay local and aren't committed.

What fought the agents, most widespread first, with the number of repos (of 5) where it happened:

1. **One `result` per entry (5).** Nearly every experiment crosses models with conditions, and the models often disagree. Agents pooled cells, picked one, or declared `mixed`, which dropped the numbers check altogether and still showed one cell's interval. One run became as many as 13 entries sharing the same episodes, and nothing said they share data.
2. **No S set before the run (5).** None of the five repos ever set one. Agents chose S afterwards, and then S decided the mark more than the data did: one entry flips at S = 0.107, another misses a supported null by $0.006. Without an S, every entry read "not checked", the strongest ones included.
3. **YAML (5).** `outcome: null`, as the first draft's own example wrote it, parses as no value, so the server would have rejected every null for a missing required field. The flow-mapping style of the field list also split any value containing a comma into extra keys with no error (2 of 5), and caveats holding colons or regular expressions needed careful quoting.
4. **Intervals the check took on trust (4).** Pooling cells across models or prompts gives intervals that are too narrow, and two repos' own rules forbid it. `n` was ambiguous for a two-arm difference. Results with zero events or zero spread gave intervals of zero width, which the check rejects, so agents switched to a smaller unit, such as seller-hours instead of sessions, which overstates precision.
5. **Results that aren't a treatment-minus-baseline difference (4).** A level or rate with no control, an instrument's validation against ground truth, a market simulation, a design check, a code audit, or a reanalysis of a published paper's data. Their cards all said "not checked", and `replicates` couldn't point outside File Drawer.
6. **The prompt is the treatment (4).** No field held it. In one repo the replications of 15 earlier prompts showed up only because the agent matched prompt hashes.
7. **Models and harness (5).** Every model was API-only, often called through a CLI alias, so `revision` was always empty. No field said what role a model played (seller, judge, monitor). The harness itself changed outcomes, by 78 points in one study.
8. **No public code (5).** Three repos have no remote, a fourth isn't pushed yet, and several runs came from uncommitted changes. The entries passed, since `code` is optional, but nobody could rerun them. Per experiment, the code needed was 27 to 170 KB in the three repos without a remote, 320 KB and 863 KB in the two with one.
9. **`positive_control` (4).** Controls were borrowed from other runs, passed for one model and failed for another, or showed that the judge works rather than that the manipulation could move the model. `not_run` blocked a null but cost an effect nothing.
10. **`predicted_direction` (4).** It had no value for "the design expected no effect", a one-sided bound such as "own ≤ joint" came out inverted, and an effect nobody predicted was marked "as predicted" once the hypothesis was rewritten to fit it.
11. **Cards hid caveats (4).** Confounded effects read "supported by the numbers" while the confound sat in the body.
12. **Smaller frictions.** `compute` was GPU-only, though every run was API calls (5). Entries written in one batch couldn't link to each other before the server issues IDs (5). No field said whether a result came from a probe, a pilot or a main run, or why the work stopped, which tells a reader whether the direction is still open (3). Floors and ceilings were invisible to the check (2).

How the first draft's numbers check fared: it marked nothing in one repo and almost nothing in another. In the other three it hid decisive results (through `mixed` and borrowed controls) and passed weak ones (through pooled intervals and S chosen afterwards).

The owner settled the changes on 2026-11-05, and this second draft carries them: the front matter and body split, the Results table, the check without S, `no_effect`, `kind`, the new fields, attached code, and batch submission. Citations may name published authors; private individuals stay out of entries.
