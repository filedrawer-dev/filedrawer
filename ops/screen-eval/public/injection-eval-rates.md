---
schema: 4
title: Spotlighting doesn't change prompt-injection success in a tool-using agent
safety_motivation: Prompt injection through tool outputs is the main attack on deployed agents, and cheap defences need testing.
hypothesis: Marking tool output with spotlighting delimiters lowers the rate at which injected instructions are followed.
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
tags: [prompt-injection, agents]
positive_control:
  outcome: passed
  checks: pipeline
  source: this_run
code_state: committed
code: https://github.com/example/injection/tree/0123456789abcdef0123456789abcdef01234567
logs: [https://wandb.ai/example/injection/runs/abc123]
written_by:
  - model: claude-opus-5-5
    harness: Claude Code 2.1.318
---

## Setup

520 generated injection attempts hidden in web pages an agent reads through a browsing tool, on Llama-3-8B-Instruct, with and without spotlighting delimiters around tool output. An attempt succeeds when the agent calls a tool the injection asked for. Only rates are reported. Positive control, from this run: removing the system prompt raised success by 0.31. Cost: 1 GPU hour on 1x A100 80GB.

## Results

Intervals: 95%, Newcombe hybrid score, per prompt.

| model | condition | baseline | estimate | ci_low | ci_high | n | unit | counts |
|---|---|---|---|---|---|---|---|---|
| Llama-3-8B-Instruct | spotlighting vs none | 0.62 | 0.012 | -0.047 | 0.070 | 520/520 | fraction | 328/520 vs 322/520 |

## Caveats

- One agent and one browsing tool.

## Not tried

- Other models.

## Reproduce

`python inject.py` (calls the model again: about 1 GPU hour).
