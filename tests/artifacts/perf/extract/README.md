# Extract Perf Scenario

Scenario C profiles `clawgs extract` over deterministic large Claude and Codex JSONL fixtures, then ranks the JSONL -> `clawgs.v1` hot paths. The measured baseline command is:

```bash
bash scripts/perf/extract_scenario.sh
```

The script runs:

```bash
target/release-perf/clawgs extract --tool claude --input tests/artifacts/perf/extract/fixtures/claude-large.jsonl >/dev/null
target/release-perf/clawgs extract --tool codex --input tests/artifacts/perf/extract/fixtures/codex-large.jsonl >/dev/null
```

## Fixture Synthesis

`scripts/perf/extract_scenario.sh --synthesize-only` replays `examples/demo/claude-sample.jsonl` and `examples/demo/codex-sample.jsonl` for 2,000 deterministic turns per tool. The jq generator adds stable IDs and timestamps from a fixed integer hash sequence over `(tool, turn, template_index)`.

Current fixture sizes:

| Fixture | Records | Bytes | sha256 |
|---|---:|---:|---|
| `tests/artifacts/perf/extract/fixtures/claude-large.jsonl` | 4,000 | 1,207,858 | `c4c2643a9f7c4d14b97282689313d03e43a70e88fd5ebb0d6c162e74a5dd6322` |
| `tests/artifacts/perf/extract/fixtures/codex-large.jsonl` | 8,000 | 1,541,340 | `6e244fe63120e170f6696e2690423c6c37c3873f02ac7d8586dab8dccae4a26b` |

The script writes `tests/artifacts/perf/extract/fixtures/SHA256SUMS` and verifies it with `sha256sum -c`.

## Baseline

Baseline was captured with:

```bash
bash scripts/perf/bench_baseline.sh --name extract --warmup 3 --runs 20 -- 'bash scripts/perf/extract_scenario.sh'
```

`baseline.hyperfine.json` preserves hyperfine's original export. `baseline.json` is normalized for the WG-004 validation contract, with one `results[]` entry per measured run plus p50/p95/p99 throughput summary.

Observed one-pass summary:

- p50: 83.164 ms
- p95: 140.571 ms
- p99: 140.571 ms, conservative because there are only 20 measured runs
- p50 throughput: ~144,294 records/s and ~33.1 MB/s
- p95 throughput: ~85,366 records/s and ~19.6 MB/s

## CPU Profile

The exact one-pass samply command completed too quickly and produced zero samples. The retained CPU profile uses one profiler recording with the same scenario command repeated by samply:

```bash
samply record --save-only --reuse-threads --unstable-presymbolicate --iteration-count 20 -o tests/artifacts/perf/extract/cpu.json -- bash scripts/perf/extract_scenario.sh
```

Artifacts:

- `tests/artifacts/perf/extract/cpu.json`
- `tests/artifacts/perf/extract/cpu.syms.json`
- `tests/artifacts/perf/extract/span_summary.json`
- `tests/artifacts/perf/extract/span_summary.jsonl`
- `tests/artifacts/perf/extract/perf.profile.span_summary.jsonl`

## Caveats

Off-CPU profiling was skipped on darwin. Heap profiling was skipped because `dhat-rs` is not already wired, and this node is measurement-only. The span summaries are sampled stacks, not instrumentation spans, so `p50_us` and `p95_us` are `null`.

Hand-off starts with `tests/artifacts/perf/extract/hotspot_table.md` and `tests/artifacts/perf/extract/hypothesis_ledger.md`.
