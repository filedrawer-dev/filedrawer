---
schema: 4
title: A longer cold proof doesn't change how tall sourdough loaves rise
safety_motivation: Bakers want a taller loaf without extra work.
hypothesis: Proofing dough in the fridge for 48 hours rather than 12 raises loaf height.
outcome: no_effect
predicted_direction: increase
hypothesis_set: before_run
stage: main
stopped_because: out of time
run_dates: [2026-10-20, 2026-10-20]
models:
  - name: home-oven
    role: subject
    route: other
tags: [baking, sourdough]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/bread/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/bread/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

520 made-up loaves of the same sourdough recipe, baked at 250 °C in a home oven. The treatment proofs the dough in the fridge for 48 hours instead of 12. Positive control, from this run: doubling the starter raised loaf height. Cost: 3 days of baking.

## Results

Intervals: 95%, Newcombe hybrid score, per loaf.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| home-oven | 48h vs 12h proof | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- One recipe and one oven.

## Not tried

- Other flours.

## Reproduce

Bake the recipe again (about 3 days).
