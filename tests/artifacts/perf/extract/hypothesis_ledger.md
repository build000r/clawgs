# Extract Hypothesis Ledger

| Hypothesis | Verdict | Evidence |
|---|---|---|
| serde_json per-record parse dominates the extract path. | supports | `serde_json::from_str::<Value>` from `src/parsers/mod.rs:45` accounts for 163/498 sampled `clawgs` stacks, and `read_jsonl` accounts for 194/498. See `tests/artifacts/perf/extract/span_summary.json` and `tests/artifacts/perf/extract/cpu.syms.json`. |
| String and `serde_json::Value` allocation/drop churn is a major secondary cost. | supports | The profile shows `BTreeMap`/`serde_json::Value` drop at 119 samples, insert at 64 samples, and iterator-drop at 60 samples; allocator leaf frames are also visible in `tests/artifacts/perf/extract/cpu.syms.json`. The source path is the `Value` parse and `entries.push(value)` flow in `src/parsers/mod.rs:45-54`. |
| Tool-use role classification and action extraction dominate normalization. | rejects | `tool_use_action` is only 11/498 inclusive samples, and `observe_tool_call` is 4/498, while parser/read/Value allocation spans are materially larger. See `tests/artifacts/perf/extract/span_summary.json`. |
| Regex compilation or regex matching happens on every record. | rejects | `rg -n "regex|Regex" src/parsers src/lib.rs Cargo.toml` found no parser regex usage, and `tests/artifacts/perf/extract/cpu.syms.json` contains no regex frames. |
| File I/O itself dominates over parse/decode work. | rejects | `read_jsonl` is visible, but sampled stacks inside it are dominated by UTF-8/line scanning and `serde_json` parsing; syscall/file-read leaf frames are not top-ranked. See `src/parsers/mod.rs:33-45` and `tests/artifacts/perf/extract/cpu.syms.json`. |
| Off-CPU wait or heap high-water is the hidden bottleneck. | rejects | Off-CPU profiling was skipped on darwin for this node, and heap profiling was skipped because `dhat-rs` is not wired. The CPU profile still collected 498 `clawgs` samples and shows CPU/parser costs as the observable bottleneck. |
