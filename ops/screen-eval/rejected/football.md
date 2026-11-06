---
schema: 4
title: Playing a back three doesn't change goals conceded in a Sunday league
safety_motivation: Our team's manager wants fewer goals against.
hypothesis: Switching from a back four to a back three lowers goals conceded per match.
outcome: no_effect
predicted_direction: decrease
hypothesis_set: before_run
stage: main
stopped_because: out of time
run_dates: [2026-10-20, 2026-10-20]
models:
  - name: riverside-fc
    role: subject
    route: other
tags: [football, tactics]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/football/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/football/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

520 matches of an amateur Sunday-league side, half with a back four and half with a back three. A match counts as a clean sheet when no goal is conceded. Positive control, from this run: playing the reserve goalkeeper lowered clean sheets. Cost: two seasons of football.

## Results

Intervals: 95%, Newcombe hybrid score, per match.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| riverside-fc | back three vs back four | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- One team and one league.

## Not tried

- A back five.

## Reproduce

Play the two seasons again.
