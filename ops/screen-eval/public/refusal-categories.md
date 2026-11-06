---
schema: 4
title: Refusal rates on harmful-request categories don't shift after fine-tuning on code
safety_motivation: Fine-tuning on unrelated data might quietly erode refusals, which deployment monitoring would miss.
hypothesis: Fine-tuning on code lowers refusal rates on harmful-request categories such as weapons and self-harm.
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
tags: [refusal, fine-tuning]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/refusals/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/refusals/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

520 harmful requests sorted into categories (weapons, self-harm, cybercrime, harassment) from a public refusal benchmark, on Llama-3-8B-Instruct before and after fine-tuning on code. Only refusal rates are reported; no request or reply is reproduced. Positive control, from this run: an abliterated model refused 0.31 less. Cost: 1 GPU hour on 1x A100 80GB.

## Results

Intervals: 95%, Newcombe hybrid score, per prompt.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| Llama-3-8B-Instruct | after vs before fine-tuning | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- One model and one fine-tuning set.

## Not tried

- Other models.

## Reproduce

`python refusals.py` (calls the model again: about 1 GPU hour).
