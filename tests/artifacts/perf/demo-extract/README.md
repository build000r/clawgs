# Scenario D: Demo Extract Corpus Profile

## Scenario

Profiles the zero-config `clawgs demo extract` path — the embedded-corpus path users hit first. This path uses `include_str!` to embed JSONL demo transcripts at compile time, writes them to a temp file, runs `extract()` on the file, serializes the result to JSON, and prints to stdout.

The scenario runs both tool variants:
```
clawgs demo extract --tool codex --pretty
clawgs demo extract --tool claude --pretty
```

## Loop-N choice

Each invocation completes in ~6.4ms — too fast for single-invocation profiling. The scenario script (`scripts/perf/demo_extract_scenario.sh`) runs **N=50 iterations** of both variants (100 total invocations per run) to accumulate measurable work for hyperfine. The samply profiling session used N=200 iterations (400 total invocations) for sufficient sample density.

Per-invocation amortized time: **6.43ms** (mean 643ms / 100 invocations).

## Reproduction

```bash
# Build the profiling binary
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile release-perf --bin clawgs

# Run the scenario manually
bash scripts/perf/demo_extract_scenario.sh

# Run baseline benchmark (20 runs, 3 warmup)
bash scripts/perf/bench_baseline.sh --name demo-extract --warmup 3 --runs 20 \
  -- 'bash scripts/perf/demo_extract_scenario.sh'

# CPU profile with samply
samply record --save-only -o tests/artifacts/perf/demo-extract/cpu.json \
  -- bash -c 'CLAWGS=target/release-perf/clawgs; for i in $(seq 1 200); do "$CLAWGS" demo extract --tool codex --pretty >/dev/null; "$CLAWGS" demo extract --tool claude --pretty >/dev/null; done'
```

## Baseline results

| Metric | Value |
|--------|-------|
| Mean | 643.3ms (6.43ms/invocation) |
| Stddev | 183.3ms |
| Median | 623.2ms |
| Min | 438.5ms |
| Max | 1098.5ms |
| Runs | 20 |
| p95 drift | 28.5% (high variance from process spawn overhead) |

## Caveats

1. **Stack-thin profile**: 1133 total clawgs-process samples across all spawned processes. Each process only lives ~6ms and collects 1-6 samples. Percentages below p99 should be treated as directional, not precise.
2. **darwin off-CPU skipped**: Off-CPU profiling not available on macOS without kernel tracing (dtrace requires SIP disabled). Kernel syscall leaf samples provide indirect off-CPU signal.
3. **Unsymbolicated samply output**: The samply JSON stores hex offsets, not function names. Symbolication was performed post-hoc via `atos`. The `span_summary.json` contains the resolved mappings.
4. **High baseline variance**: The 28.5% stddev is expected — 100 process spawns per run means fork/exec jitter dominates. The per-invocation amortized time (6.43ms) is the stable metric.
5. **`__recvfrom_nocancel` syscall**: 108 samples (9.5%) in this syscall are from macOS `NSTemporaryDirectory()` calling into configd/mDNS, not actual network I/O.

## Key findings

The demo extract path is **startup and I/O dominated**, not compute-dominated:

1. **Temp file round-trip** (~30% inclusive): The biggest optimization target. The embedded corpus is already in memory via `include_str!`, but the demo path writes it to a temp file, reads it back, then deletes it.
2. **Process startup** (~20%): dyld + Rust runtime init. Irreducible per-process cost.
3. **JSONL parsing** (~8%): `read_jsonl` + `serde_json::from_str` per line.
4. **JSON serialization** (~5%): `serde_json` pretty-printing the output.
5. **Heap allocation** (~12%): malloc/mmap from String/Vec churn during parse.

## Hand-off

The `hotspot_table.md` and `hypothesis_ledger.md` are ready for `extreme-software-optimization`. The highest-leverage optimization is eliminating the temp file round-trip (H1), which would require `extract()` to accept a `Read`/`&str` input instead of a file path. This subsumes H5 (caching the parsed corpus).

## Artifacts

| File | Description |
|------|-------------|
| `baseline.json` | hyperfine raw results (20 runs) |
| `baseline.md` | hyperfine markdown table |
| `cpu.json` | samply CPU profile (Firefox Profiler format) |
| `span_summary.json` | Symbolicated attribution by library and function |
| `perf.profile.leaf.jsonl` | Ranked leaf-frame samples |
| `hotspot_table.md` | Top-7 hotspots with evidence |
| `hypothesis_ledger.md` | 5 hypotheses with verdicts |
| `run.log` | Benchmark run metadata |
| `README.md` | This file |
