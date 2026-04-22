# Emit --stdio Perf Scenario (Scenario B)

## Scenario

Profile the `clawgs emit --stdio` NDJSON sync loop: process startup through
20 sync round trips (2 sessions per tick, terminal-only — no transcript/LLM path)
to clean exit on stdin EOF.

## Reproduction

```bash
# Build the profiling binary
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile release-perf --bin clawgs

# Run the scenario (should exit in <100ms)
bash scripts/perf/emit_stdio_scenario.sh

# Run baseline benchmark (20 hyperfine runs)
bash scripts/perf/bench_baseline.sh --name emit-stdio --warmup 3 --runs 20 \
  -- 'bash scripts/perf/emit_stdio_scenario.sh'

# CPU profile (single invocation)
samply record --save-only -o tests/artifacts/perf/emit-stdio/cpu.json \
  -- target/release-perf/clawgs emit --stdio \
  < scripts/perf/fixtures/emit_stdio.ndjson
```

## Fixture

`scripts/perf/fixtures/emit_stdio.ndjson` — 20 deterministic sync requests, each
with 2 sessions (one busy, one idle). No `tool` set, so no transcript discovery or
LLM calls. Sessions use terminal-only context to exercise the core sync engine path.

## Key Findings

Process startup dominates (~80% of wall time):
- addr2line symbol table init: 37.7% self-time
- anstream terminal detection: 26.3% self-time
- reqwest/tokio client build: 34.0% inclusive

Per-request path is cheap (~20%):
- chrono DateTime parsing: 15.8% self-time (largest per-request cost)
- serde_json ser/deser: 1.0% self / 5.4% inclusive
- engine logic (strip_ansi, retain, process_session): <1%

## Caveats

- Profiled on macOS arm64 (Apple Silicon). SIP blocks dtrace, so no off-CPU analysis.
- The 100-invocation loop profile amplifies startup costs. A long-running daemon
  benchmark would show a different distribution favoring chrono and serde.
- The `release-perf` profile includes debug info, inflating addr2line costs vs a
  stripped release binary.
- Samples are CPU-time only. No heap profiling (dhat-rs not wired).

## Artifacts

| File | Description |
|------|-------------|
| `baseline.json` | Hyperfine results (20 runs) |
| `baseline.md` | Hyperfine markdown table |
| `cpu.json` | Samply CPU profile (Firefox profiler format) |
| `span_summary.json` | Top spans with sample counts and categories |
| `span_summary.jsonl` | Same data in JSONL format |
| `hotspot_table.md` | Ranked top-7 hotspot table |
| `hypothesis_ledger.md` | 6 hypotheses with supports/rejects verdicts |
| `run.log` | Benchmark run metadata |

## Hand-off

For optimization, the highest-leverage targets are:
1. **Long-running daemon mode** — amortize startup costs (addr2line, reqwest, clap)
2. **chrono DateTime parsing** — consider pre-parsed timestamps or epoch-based wire format
3. **Lazy model client init** — defer reqwest client build until first LLM call needed
