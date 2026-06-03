# Grok Headless Narrator Throttle

## Scenario

Replay `scripts/perf/fixtures/emit_stdio.ndjson` through `target/release/clawgs
emit --stdio` with:

- `CLAWGS_MODEL_BACKEND=grok`
- `CLAWGS_GROK_BIN` set to a fake Grok binary
- fake Grok exits successfully for `--version`
- fake Grok increments a counter for each real completion and prints one status
  line

This fixture has 20 sync ticks at 15-second hot cadence. The active session's
terminal text changes each tick, matching the observed narrator pattern where
objective changes previously bypassed cadence and cold-started Grok every tick.

## Hotspot

| Rank | Location | Metric | Value | Category | Evidence |
| --- | --- | --- | --- | --- | --- |
| 1 | Grok CLI spawn from `GrokCliModelClient::complete_once` | completion spawns | 20 per 20-tick fixture before | CPU/process cold-start | fake Grok counter baseline |

## Opportunity Score

- Impact: 5 (live sample caught Grok at ~101% CPU; fixture spawned once per
  tick)
- Confidence: 5 (spawn count is directly measured)
- Effort: 2 (engine policy + focused tests)
- Score: 12.5

## Change

`EmitEngine` now applies a Grok-only cadence multiplier. The first Grok status
line remains immediately eligible, but repeated Grok thought calls require
`configured cadence * CLAWGS_GROK_CADENCE_MULTIPLIER`. The default multiplier
is `10`; `CLAWGS_GROK_CADENCE_MULTIPLIER=1` restores protocol cadence.

OpenRouter keeps the previous objective-change behavior.

## Isomorphism / Contract Proof

- Ordering preserved: yes; session iteration and update ordering are unchanged.
- Tie-breaking unchanged: N/A.
- Floating-point: N/A.
- RNG seeds: N/A.
- First Grok status preserved: `emit_stdio_uses_fake_grok_binary_for_live_thought`
  passes.
- Carry-forward contract preserved: suppressed ticks may still emit passive
  state/cue changes without calling the model.
- Protocol shape unchanged: no schema fields were added or removed.

## Results

| Run | Grok completions | `llm_calls` with value `1` | Relative |
| --- | ---: | ---: | ---: |
| Before | 20 | 20 | 1.0x |
| After, default multiplier 10 | 2 | 2 | 10.0x fewer |
| After, multiplier 1 | 20 | 20 | compatibility restored |

## Verification

- `cargo fmt -- --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --release`
- before/after fake-Grok spawn-count replay described above

