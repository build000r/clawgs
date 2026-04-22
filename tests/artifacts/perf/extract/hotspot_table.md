# Extract Hotspot Table

| Rank | Location | Metric | Value | Evidence |
|---:|---|---|---:|---|
| 1 | `src/parsers/codex.rs:19` `parse` batch normalization loop | Inclusive CPU samples | 487/498 samples (~487 ms @ 1 kHz) | `tests/artifacts/perf/extract/cpu.syms.json`, `tests/artifacts/perf/extract/span_summary.json` |
| 2 | `src/parsers/mod.rs:33` `read_jsonl` whole-file read and line scan | Inclusive CPU samples | 194/498 samples (~194 ms @ 1 kHz) | `tests/artifacts/perf/extract/cpu.syms.json`, `tests/artifacts/perf/extract/span_summary.json` |
| 3 | `src/parsers/mod.rs:45` `serde_json::from_str::<Value>` per JSONL record | Inclusive CPU samples | 163/498 samples (~163 ms @ 1 kHz) | `tests/artifacts/perf/extract/cpu.syms.json`, `tests/artifacts/perf/extract/span_summary.json` |
| 4 | `src/parsers/mod.rs:45` `serde_json::Value` object map allocation/drop | Inclusive CPU samples | 119 drop / 64 insert / 60 iterator-drop samples | `tests/artifacts/perf/extract/cpu.syms.json`, `tests/artifacts/perf/extract/span_summary.json` |
| 5 | `src/parsers/claude.rs:49` `entry_type` field probing | Inclusive CPU samples | 80/498 samples (~80 ms @ 1 kHz) | `tests/artifacts/perf/extract/cpu.syms.json`, `tests/artifacts/perf/extract/span_summary.json` |
| 6 | `src/parsers/claude.rs:191` `tool_use_action` detail extraction | Inclusive CPU samples | 11/498 samples (~11 ms @ 1 kHz) | `tests/artifacts/perf/extract/cpu.syms.json`, `tests/artifacts/perf/extract/span_summary.json` |

Baseline one-pass throughput from `tests/artifacts/perf/extract/baseline.json`: p50 83.164 ms, p95 140.571 ms, p99 140.571 ms over 12,000 records / 2,749,198 bytes. p50 throughput is ~144,294 records/s and ~33.1 MB/s.
