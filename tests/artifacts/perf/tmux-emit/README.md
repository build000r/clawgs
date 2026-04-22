# Scenario A: tmux-emit Steady-State Profile

## Description

Profiles the `clawgs tmux-emit` scan-emit loop under realistic conditions: a throwaway tmux session with deterministic content seed, plus all existing live sessions (~44 panes on the test machine). Measures single-scan latency (via hyperfine) and continuous-loop CPU attribution (via samply).

## How to Reproduce

### Prerequisites
```
brew install hyperfine
cargo install samply
cargo build --profile release-perf --bin clawgs
```

### Baseline (20 runs, single-scan mode)
```
bash scripts/perf/bench_baseline.sh --name tmux-emit --warmup 3 --runs 20 -- \
  'bash scripts/perf/tmux_emit_scenario.sh'
```

### CPU Profile (30s steady-state loop)
```
samply record --save-only -o tests/artifacts/perf/tmux-emit/cpu.json -- \
  bash scripts/perf/tmux_emit_scenario.sh --duration 30
```

### View profile in Firefox Profiler
```
samply load tests/artifacts/perf/tmux-emit/cpu.json
```

## Baseline Results (20 runs)

| Metric | Value |
|--------|-------|
| p50 | 1.442s |
| p95 | 3.067s |
| p99 | 3.231s |
| mean | 1.673s |
| stddev | 0.624s |
| min | 0.950s |
| max | 3.272s |

Note: High variance (CoV 37%) is characteristic of subprocess-heavy workloads sensitive to system load and tmux server contention. A second 5-run batch showed mean 1.096s with much tighter spread, suggesting the high p95/p99 in the primary run reflects transient system load, not algorithmic variance.

## Key Findings

1. **Chrono datetime parsing = 66.8% of CPU** — `short_or_long_month0` (39.1%) + `parse_internal` (27.7%). Exact callsite obscured by LTO thin; likely serde `DateTime<Utc>` path.
2. **Per-pane subprocess spawning = 61.4% wall-time** — ~44 `tmux capture-pane` fork+exec calls per scan cycle. Dominates latency.
3. **String normalization = 6.9% CPU** — `to_lowercase` in command classification.
4. **Process is 94% idle** — 6.0% CPU utilization over 25.3s. Socket wait between scans dominates wall-clock.
5. **Synchronous I/O only** — no tokio, no async; single-threaded blocking loop.

## Caveats

- **Variance**: p95 unreliable with only 20 samples; the p99.9 cannot be estimated.
- **Off-CPU skipped**: macOS SIP blocks dtrace; xctrace System Trace not wired. Off-CPU analysis (I/O waits, scheduler preemption) is a gap.
- **Heap skipped**: dhat-rs not compiled into the binary. Allocation churn analysis relies on CPU symbol inference only.
- **LTO symbol accuracy**: With `lto = "thin"` and `opt-level = 3`, stack unwinding and symbol attribution can be imprecise. A `debug = 2` build would give sharper callsite resolution at the cost of different optimization behavior.

## Artifacts

| File | Description |
|------|-------------|
| `baseline.json` | Hyperfine JSON output (20 runs) |
| `baseline.md` | Hyperfine markdown summary |
| `cpu.json` | Samply/Firefox Profiler CPU profile (25.3s) |
| `span_summary.json` | Top-7 spans ranked by CPU self-time |
| `span_summary.jsonl` | Same data in NDJSON format |
| `hotspot_table.md` | Canonical 5-column ranked hotspot table |
| `hypothesis_ledger.md` | 5 hypotheses with supports/rejects verdicts |
| `run.log` | Benchmark run metadata |

## Hand-off

Next step: `extreme-software-optimization` should act on `hotspot_table.md` + `hypothesis_ledger.md`. Primary optimization targets:
1. Reduce chrono parsing (cache `Utc::now()`, faster datetime repr, or `debug=2` build to identify exact callsite)
2. Batch tmux subprocess calls (control-mode or multi-target capture)
