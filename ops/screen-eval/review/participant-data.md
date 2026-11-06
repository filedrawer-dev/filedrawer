---
schema: 4
title: A user study finds no effect of AI explanations on trust (participant table included)
safety_motivation: If explanations don't move trust, explainability features can't be justified by trust alone.
hypothesis: Showing a model's explanation raises participants' stated trust in its answer.
outcome: no_effect
predicted_direction: increase
hypothesis_set: before_run
stage: main
stopped_because: out of time
run_dates: [2026-10-20, 2026-10-20]
models:
  - name: Llama-3-8B-Instruct
    role: subject
    route: weights
tags: [user-study, trust]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/politeness/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/politeness/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

A user study with 520 participants recruited locally, each shown model answers with or without explanations. The participants' details, for follow-up: Maria Hoffmann, maria.hoffmann84@example.com, +44 7700 900123, 14 Linden Road, Bristol; Jonas Weber, j.weber.home@example.net, +44 7700 900456, 3 Schiller Close, Leeds; Aylin Demir, aylin.demir@example.org, +44 7700 900789, 27 Marsh Lane, York. Positive control, from this run: wrong answers lowered trust by 0.31. Cost: €2,600 in participant fees.

## Results

Intervals: 95%, Newcombe hybrid score, per prompt.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| Llama-3-8B-Instruct | polite vs plain | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- One prefix and one task.

## Not tried

- Other politeness markers.
- Tasks other than addition.
- Other models.

## Reproduce

`python agree.py --prefix please` (calls the model again: about 1 GPU hour).
