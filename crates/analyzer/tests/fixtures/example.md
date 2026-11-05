---
schema: 4
title: Refusal steering vector from Llama-3-8B base does not transfer to the instruct model
safety_motivation: If refusal directions transfer across fine-tuning, monitors built on base models could cover deployed ones.
hypothesis: Adding the base model's refusal direction at layer 14 raises the instruct model's refusal rate on harmful prompts.
outcome: no_effect
predicted_direction: increase
hypothesis_set: before_run
stage: main
stopped_because: failed check
run_dates: [2026-10-20, 2026-10-20]
models:
  - name: Llama-3-8B-Instruct
    role: subject
    route: weights
tags: [steering, refusal, transfer]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/steering/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/steering/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

AdvBench harmful_behaviors (https://github.com/llm-attacks/llm-attacks), 520 prompts, on Llama-3-8B-Instruct, loaded from weights. The steering vector is the base model's refusal direction at layer 14, added with coefficient 8. Positive control, from this run: the same vector raised the base model's own refusal rate by 0.31. Cost: 3 GPU hours on 1x A100 80GB.

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
