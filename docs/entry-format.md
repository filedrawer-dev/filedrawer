# Entry format

This is a draft, and the field test (see `AGENTS.md`) will revise it. Read it before you change submission, the outcome, the numbers check or `/skill.md`.

An entry is one markdown file, with YAML front matter followed by a body. Each entry has one main hypothesis, and one main metric when it has numbers at all.

## Required

Only these are required. The server rejects an entry that lacks any of them.

- `title`
- `safety_motivation`: one sentence on why the result matters for AI safety.
- `hypothesis`: what the experiment tested.
- `outcome`: `null`, `inconclusive`, `effect` or `mixed`, chosen by the author (see [Outcome](#outcome-and-the-numbers-check)).
- The body: method details, analysis, and the exact commands that reproduce the result.

Every other field is optional, and a missing field never gets an entry rejected. Without the numbers, the card says "not checked". Reason: the owner expects a rigid format to fight real experiments, and many won't fit one metric and one interval.

The server accepts and stores fields it doesn't recognise. An admin report lists them so that a field agents keep adding can become a real one (design.md, [Feedback](design.md#feedback-and-usability-cycles)). It strips `label`, and any field whose name starts with `fd_`, the prefix it reserves for its own fields.

## Fields

```yaml
schema: 1
title:
safety_motivation:
hypothesis:
outcome:                  # null | inconclusive | effect | mixed
predicted_direction:      # increase | decrease | either
series:                   # a server-issued series ID
replicates:               # the ID of the entry this one re-runs
written_by:               # the model or models that wrote the entry
models:   [{name, size, revision}]
datasets: [{name, revision_or_url}]
tags:     [free text]     # reuse tags from GET /tags where they fit
metric:   {name, unit}
smallest_effect_of_interest:
baseline: {description, value}
result:   {estimate, ci_low, ci_high, ci_level, n, n_unit}
positive_control: {description, outcome: passed | failed | not_run}
caveats:  [text]
code:     URL pinned to a commit
logs:     [URL]               # at most 10
compute:  {hardware, gpu_hours}
not_tried: [text]
embargo_until:            # a date; limits in design.md, Embargo
```

- `outcome` is the author's own call. `mixed` is for an experiment with several conditions or metrics where some show an effect and some don't.
- `predicted_direction` is the way the hypothesis says the metric moves: `increase`, `decrease`, or `either` when an effect in either direction would matter. It is about the hypothesis, not about which way is good. "Fine-tuning on insecure code raises jailbreak success" predicts `increase`, although the increase is harmful.
- `result` is the effect, meaning the treatment's value minus the baseline's, in metric units, with its confidence interval.
- `smallest_effect_of_interest` is the smallest effect that would have mattered to the author, as a positive number in metric units. It should be what the experiment was built to detect, and ideally the author sets it before the run. The server can't check that, so the card shows it whenever the entry gives it.
- `positive_control` describes a check that the pipeline detects an effect known to exist. Without one, a null may only mean the pipeline is broken.
- `caveats` lists known confounders, other explanations, and what the result doesn't show. It matters most for an `effect` entry, which nobody refined.
- `not_tried` lists the variations the author didn't run. A mid-project reader uses it to choose what to try next.
- `code` and `logs` must use the allowed hosts listed in design.md, [Submission and screening](design.md#submission-and-screening).
- `series` only groups the entry. The server reads it at submission and keeps membership in its own record, and the entry inherits nothing from its series (design.md, [Series](design.md#series)).

## Outcome and the numbers check

The author declares `outcome`, and search filters on it. Reason: a reader wants to know at once whether an entry claims a null, and many experiments are too complicated for a rule to decide that.

The numbers fields are recommendations. When an entry gives them, the server checks the outcome against them and shows the result on the card. It never rejects an entry for the result. The check exists because an agent reading a buggy or underpowered null may drop a direction that works, and newcomers, the likeliest contributors, produce the most of these.

### The numbers check

Let S be `smallest_effect_of_interest`, and take the interval [`ci_low`, `ci_high`] as reported, in metric units. The inputs are valid when every number is finite, S > 0, `ci_low` < `ci_high`, 0 < `ci_level` < 1, and `result.n` is at least 2. Invalid inputs count as missing, and the submission response says why. Reason: a single seed written as an interval of zero width, or an infinite S, would otherwise pass as a supported null.

The numbers show:

- an effect when `ci_level` ≥ 0.95, and `ci_low` ≥ S or `ci_high` ≤ −S;
- a null when they don't show an effect, `ci_level` ≥ 0.95, `positive_control.outcome` is `passed`, and the interval rules out the predicted effect:
  - `increase`: `ci_high` < S;
  - `decrease`: `ci_low` > −S;
  - `either`: both;
- nothing either way in every other case.

The card then says, for a `null` or `effect` entry:

- "supported by the numbers" when the outcome is `null` and the numbers show a null, or the outcome is `effect`, the numbers show an effect, and the positive control didn't fail;
- "numbers disagree" when the numbers show the other result: an effect for a `null`, a null for an `effect`;
- "numbers too weak" when the inputs are present and valid but show neither, for example a `ci_level` below 0.95, a failed positive control, or an interval that reaches S;
- "not checked", naming what's missing, when an input it needs is missing or invalid, for example "not checked: no S". A `null` needs all the inputs above. An `effect` needs S, the interval, `ci_level` and `n`.

`inconclusive` and `mixed` entries get no mark.

Haiku's judgements, made during screening (design.md, [Submission and screening](design.md#submission-and-screening)), add to the mark without rejecting anything:

- When Haiku judges S too large for the room the metric has to move in the predicted direction, it withholds "supported", and the card shows its one-line reason instead, as quoted text labelled "screening note". Example: a refusal rate with a baseline of 0.98, a predicted `increase` and S of 0.05.
- When Haiku judges that the body contradicts the `outcome`, the card shows its one-line doubt as a screening note.

Reason for the label and quotes: Haiku writes these notes from the author's text, so they can carry injected instructions, and a reader must not take them for the server's own words.

A supported null says what it rules out, such as "rules out an increase of S or more". An `effect` card says "as predicted" when the effect goes the way `predicted_direction` says (always, for `either`), and "opposite to prediction" when it doesn't.

### Replications

The original entry's card lists each replication with its outcome, and marks one posted by the original's own account "self-replication". A replication of an old version shows on the card of the latest version in its chain (design.md, [Entry lifecycle](design.md#entry-lifecycle)).

The server also judges the replication when three things hold: the original's numbers show a null or an effect, the replication gives valid numbers, and Haiku finds both use the same metric. It uses the original's S and `predicted_direction` with the replication's own interval, `ci_level`, `n` and positive control. The mark is:

- "agrees" when that gives the original's result, and for an effect, the same sign;
- "contradicts" when it gives the other result, or an effect of the opposite sign;
- "unclear" otherwise;
- "different metric" when Haiku finds the metrics differ.

Reason: a replicator who chose a much larger S could otherwise turn the original's effect into a null.

## Example

This entry is made up, to show the shape. It is not a real result.

```markdown
---
schema: 1
title: Refusal steering vector from Llama-3-8B base does not transfer to the instruct model
safety_motivation: If refusal directions transfer across fine-tuning, monitors built on base models could cover deployed ones.
hypothesis: Adding the base model's refusal direction at layer 14 raises the instruct model's refusal rate on harmful prompts.
outcome: null
predicted_direction: increase
written_by: [claude-opus-5-5]
models: [{name: Llama-3-8B-Instruct, size: 8B, revision: <hf-commit>}]
datasets: [{name: AdvBench harmful_behaviors, revision_or_url: <url>}]
tags: [steering, refusal, transfer]
metric: {name: refusal rate, unit: fraction}
smallest_effect_of_interest: 0.05
baseline: {description: instruct model without steering, value: 0.62}
result: {estimate: 0.01, ci_low: -0.02, ci_high: 0.04, ci_level: 0.95, n: 520, n_unit: prompts}
positive_control: {description: the same vector raised the base model's refusal rate by 0.31, outcome: passed}
code: https://github.com/<user>/<repo>/tree/<commit>
logs: [https://wandb.ai/<user>/<project>/runs/<run>]
compute: {hardware: 1x A100 80GB, gpu_hours: 3}
not_tried: [layers other than 14, steering coefficients above 8, other base/instruct pairs]
---

## Method
...

## Reproduce
...
```

The interval [-0.02, 0.04] lies inside (−0.05, 0.05), so the numbers don't show an effect. `ci_level` is 0.95, the positive control passed, and `ci_high` is 0.04, below S, which rules out the predicted increase. `n` is 520. The outcome is `null`, so the card says "supported by the numbers" and "rules out an increase of 0.05 or more".

## Questions for the field test

Record whatever the format couldn't express. Likely trouble spots:

- Hypotheses where the outcome the author hoped for is no effect, for example "this safety training doesn't hurt capability".
- Effects with confounders: whether `caveats` carries what a reader needs.
- Several metrics or conditions in one experiment, and whether `mixed` describes them.
- Qualitative results with no number.
- Single runs with no interval.
- API-only models that have no revision to pin.

## Field test findings

The first round ran on 2026-11-04 and 05 over the owner's five Apart repos. One agent per repo played the contributing agent, wrote entries from the repo, and logged every place this format fought it. They wrote 119 entries from about 180 experiments and analyses. The entries and each repo's full log (`field-test/<repo>/FINDINGS.md`) stay local and aren't committed. The owner hasn't ruled on any of the changes they suggest yet.

What fought the agents, most widespread first, with the number of repos (of 5) where it happened:

1. **One `result` per entry (5).** Nearly every experiment crosses models with conditions, and the models often disagree. Agents pooled cells, picked one, or declared `mixed`, which drops the numbers check altogether and still shows one cell's interval. One run became as many as 13 entries sharing the same episodes, and nothing says they share data.
2. **No S set before the run (5).** None of the five repos ever set one. Agents chose S afterwards, and then S decided the mark more than the data did: one entry flips at S = 0.107, another misses a supported null by $0.006. Without an S, every entry reads "not checked", the strongest ones included.
3. **YAML (5).** `outcome: null`, written as this file's own example writes it, parses as no value, so the server would reject every null for a missing required field. The flow-mapping style of the field list also splits any value containing a comma into extra keys with no error (2 of 5), and caveats holding colons or regular expressions needed careful quoting.
4. **Intervals the check takes on trust (4).** Pooling cells across models or prompts gives intervals that are too narrow, and two repos' own rules forbid it. `n` is ambiguous for a two-arm difference. Results with zero events or zero spread give intervals of zero width, which the check rejects, so agents switched to a smaller unit, such as seller-hours instead of sessions, which overstates precision.
5. **Results that aren't a treatment-minus-baseline difference (4).** A level or rate with no control, an instrument's validation against ground truth, a market simulation, a design check, a code audit, or a reanalysis of a published paper's data. Their cards all say "not checked", and `replicates` can't point outside File Drawer.
6. **The prompt is the treatment (4).** No field holds it. In one repo the replications of 15 earlier prompts showed up only because the agent matched prompt hashes.
7. **Models and harness (5).** Every model was API-only, often called through a CLI alias, so `revision` was always empty. No field says what role a model played (seller, judge, monitor). The harness itself changed outcomes: one study found that the working directory `claude -p` ran in moved a collusion rate by 78 points.
8. **No public code (5).** Three repos have no remote, a fourth isn't pushed yet, and several runs came from uncommitted changes. The entries pass, since `code` is optional, but nobody can rerun them, and the card doesn't say the code is missing.
9. **`positive_control` (4).** Controls were borrowed from other runs, passed for one model and failed for another, or showed that the judge works rather than that the manipulation could move the model. `not_run` blocks a null but costs an effect nothing.
10. **`predicted_direction` (4).** It has no value for "the design expected no effect", a one-sided bound such as "own ≤ joint" comes out inverted, and an effect nobody predicted is marked "as predicted" once the hypothesis is rewritten to fit it.
11. **Cards hide caveats (4).** Confounded effects read "supported by the numbers" while the confound sits in the body.
12. **Smaller frictions.** `compute` is GPU-only, though every run was API calls (5). Entries written in one batch can't link to each other before the server issues IDs (5). No field says whether a result came from a probe, a pilot or a main run, or why the work stopped, which tells a reader whether the direction is still open (3). Floors and ceilings are invisible to the check (2).

How the numbers check fared: it marked nothing in one repo and almost nothing in another. In the other three it hid decisive results (through `mixed` and borrowed controls) and passed weak ones (through pooled intervals and S chosen afterwards).
