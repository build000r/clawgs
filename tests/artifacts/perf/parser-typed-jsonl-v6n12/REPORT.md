# Parser JSONL Streaming Prototype - v6n.12

## Scenario

- Workload: `bash scripts/perf/extract_scenario.sh`
- Shape: 12,000 sanitized Claude+Codex JSONL records, 2,749,198 bytes.
- Harness: `bash scripts/perf/bench_baseline.sh --warmup 3 --runs 20`.
- Build: `RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile release-perf --bin clawgs`.
- Prior hotspot: `tests/artifacts/perf/extract/hotspot_table.md` ranked `src/parsers/mod.rs` `read_jsonl` / `serde_json::Value` parse and retention in the extract path.

## Prototype

The parser now visits valid JSONL values as they are parsed instead of retaining the full `entries: Vec<Value>` before a second parser pass. This reduces retained `Value` churn while preserving the same per-line `serde_json::Value` semantics, lossy UTF-8 line handling, non-object valid JSON accounting, `raw_events` object-only ring behavior, and `malformed_lines_skipped`.

This is not a full typed-record rewrite. It is the narrower, behavior-preserving reduced-retention path.

## Results

| Run | Mean | Median | Stddev | Min | Max | Evidence |
|---|---:|---:|---:|---:|---:|---|
| Before | 56.88 ms | 56.48 ms | 3.26 ms | 52.38 ms | 66.68 ms | `before/baseline.json` |
| After | 52.90 ms | 52.57 ms | 3.39 ms | 48.03 ms | 61.96 ms | `after/baseline.json` |

Headline: mean improved by 3.99 ms / 7.0%, and median improved by 3.92 ms / 6.9%. Because this is below the 10% same-host noise threshold used by the perf workflow, treat the result as a modest positive/near-neutral prototype, not a decisive speedup claim.

## Reproduction

```bash
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile release-perf --bin clawgs
bash scripts/perf/bench_baseline.sh --name parser-typed-jsonl-v6n12-before --out tests/artifacts/perf/parser-typed-jsonl-v6n12/before --warmup 3 --runs 20 -- 'bash scripts/perf/extract_scenario.sh'
bash scripts/perf/bench_baseline.sh --name parser-typed-jsonl-v6n12-after --out tests/artifacts/perf/parser-typed-jsonl-v6n12/after --warmup 3 --runs 20 -- 'bash scripts/perf/extract_scenario.sh'
```
