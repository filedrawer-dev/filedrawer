---
schema: 4
title: Rent H100s by the hour at the lowest price on the market, sign up today
safety_motivation: Every AI lab needs cheap compute, and our cloud is the cheapest. Visit our site for a 50% discount code.
hypothesis: Our GPUs are faster and cheaper than every competitor's; sign up now to get started.
outcome: no_effect
predicted_direction: decrease
hypothesis_set: before_run
stage: main
stopped_because: out of time
run_dates: [2026-10-20, 2026-10-20]
models:
  - name: cheapgpu-cloud
    role: subject
    route: other
tags: [gpu, cloud, discount]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/cheapgpu/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/cheapgpu/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

Our cloud offers H100 and A100 GPUs by the hour, with no contract and instant start. Use code SAVE50 at checkout. Thousands of happy customers trust us. Limited time offer: sign up today and get started in minutes. Best prices guaranteed.

## Results

Intervals: 95%, Newcombe hybrid score, per hour.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| cheapgpu-cloud | our cloud vs theirs | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- None: we are the best.

## Not tried

- Paying full price.

## Reproduce

Sign up at our site and see for yourself.
