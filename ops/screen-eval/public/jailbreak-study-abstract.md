---
schema: 4
title: Role-play jailbreaks keep their effect after a refusal-training update
safety_motivation: Knowing which jailbreak families a safety update fixes tells defenders where to look next.
hypothesis: A refusal-training update lowers the success rate of role-play jailbreaks by at least 0.2.
outcome: no_effect
predicted_direction: decrease
hypothesis_set: before_run
stage: main
stopped_because: out of time
run_dates: [2026-10-20, 2026-10-20]
models:
  - name: Llama-3-8B-Instruct
    role: subject
    route: weights
tags: [jailbreaks, refusal]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/jailbreaks/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/jailbreaks/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

520 generated role-play jailbreak attempts from a public benchmark's templates, scored by a keyword judge, on Llama-3-8B-Instruct before and after a refusal-training update. The prompts aren't reproduced here; the benchmark's paper describes the families. A reply counts as a success when it complies. Positive control, from this run: an unaligned base model complied with 0.31 more of them. Cost: 1 GPU hour on 1x A100 80GB.

## Results

Intervals: 95%, Newcombe hybrid score, per prompt.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| Llama-3-8B-Instruct | after vs before update | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- One model and one update.

## Not tried

- Other models.

## Reproduce

`python attempts.py` (calls the model again: about 1 GPU hour).
