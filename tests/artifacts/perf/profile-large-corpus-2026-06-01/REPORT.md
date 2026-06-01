# Profile Extract, Emit, and Tmux Scan - 2026-06-01

## Scenario Fingerprint

- Run date: 2026-06-01T23:18Z
- Git SHA: `010210326e19253ed1f0c88ecc5daba8f681ca0f`
- Host: `bs-MacBook-Air.local`
- OS/kernel: macOS 15.6 build 24G84, Darwin 24.6.0, arm64
- CPU/RAM: Apple M4, 10 logical cores, 25,769,803,776 bytes RAM
- Rust toolchain: `rustc 1.93.1 (01f6ddf75 2026-02-11) (Homebrew)`, `cargo 1.93.1 (Homebrew)`
- Build profile: `release-perf` with `RUSTFLAGS="-C force-frame-pointers=yes"`
- Fingerprint evidence: `tests/artifacts/perf/fingerprint.json`
- Worktree at fingerprint time: dirty `.beads/issues.jsonl`, `.gitignore`, `AGENTS.md`, untracked `memory/`, plus this run's new fake-tmux perf script.
- Data safety: no private `$HOME/.claude` or `$HOME/.codex` transcript content was used. Extract used checked-in sanitized synthetic fixtures generated from `examples/demo`.

## Dataset Shape

Extract corpus evidence: `tests/artifacts/perf/extract/fixtures/metadata.json`.

| Tool | Records | Bytes | Source |
|---|---:|---:|---|
| Claude | 4,000 | 1,207,858 | sanitized deterministic fixture |
| Codex | 8,000 | 1,541,340 | sanitized deterministic fixture |
| Total | 12,000 | 2,749,198 | no private content |

Emit cadence fixture: `scripts/perf/fixtures/emit_stdio.ndjson`, transformed at runtime with `jq -c '.config.enabled=false'` to keep the scenario offline. Shape: 20 sync requests, 2 sessions per sync, 40 session snapshots total.

Tmux scan fixture: `scripts/perf/tmux_fake_scenario.sh`, deterministic fake tmux with 64 panes, 120 captured lines per pane, and `--max-capture-lines 200`.

## Baseline Commands And Results

All baseline runs used `bash scripts/perf/bench_baseline.sh --warmup 3 --runs 20`.

| Scenario | Command | p50 | p95* | p99* | Mean | Throughput | Peak RSS | Evidence |
|---|---|---:|---:|---:|---:|---:|---:|---|
| Extract large Claude+Codex | `bash scripts/perf/extract_scenario.sh` | 53.35 ms | 59.44 ms | 74.66 ms | 54.98 ms | p50 224.9k records/s, 51.5 MB/s | 18.6 MiB | `extract/baseline.json`, `extract/time-l.txt` |
| Emit stdio offline cadence | `jq -c '.config.enabled=false' scripts/perf/fixtures/emit_stdio.ndjson \| OPENROUTER_API_KEY=fake target/release-perf/clawgs emit --stdio >/dev/null` | 5.78 ms | 6.82 ms | 7.84 ms | 5.92 ms | p50 3,460 syncs/s, 6,921 sessions/s | 6.4 MiB | `emit-stdio-disabled/baseline.json`, `emit-stdio-disabled/time-l.txt` |
| Fake tmux scan | `TMUX_FAKE_PANES=64 TMUX_FAKE_CAPTURE_LINES=120 TMUX_FAKE_MAX_CAPTURE_LINES=200 bash scripts/perf/tmux_fake_scenario.sh` | 161.51 ms | 191.54 ms | 195.08 ms | 164.97 ms | p50 396 panes/s | 9.0 MiB | `tmux-fake/baseline.json`, `tmux-fake/time-l.txt` |

`*` p95/p99 are conservative order statistics from 20 samples. For p99, the table reports max.

Hyperfine warned that the stdio command is near the shell-startup precision floor. Treat the stdio numbers as command-level cadence for this fixture, not a steady-state daemon microbenchmark.

## Ranked Hotspot / Opportunity Table

| Rank | Area | Measurement | Evidence | Interpretation | Recommended Bead |
|---:|---|---:|---|---|---|
| 1 | Tmux scan command path | p50 161.51 ms / 64 fake panes, p95 191.54 ms | `tmux-fake/baseline.json`, `tmux-fake/time-l.txt`, prior `tests/artifacts/perf/tmux-emit/AFTER.md` | Current fake scan is much faster than the older live-tmux pre-batch profile, but command-level startup/model-client init plus fake subprocess overhead still dominate the one-shot boundary. Do not optimize from this alone without a long-running daemon attribution pass. | No new optimization bead; optional future debug bead if daemon-only tmux attribution is needed. |
| 2 | Extract parser generic JSON path | p50 53.35 ms for 12k records; prior CPU profile ranked `serde_json::Value` parse/allocation as top cost | `extract/baseline.json`, `tests/artifacts/perf/extract/hotspot_table.md`, `tests/artifacts/perf/extract/span_summary.json` | The actionable parser lever is reducing generic `Value` churn or streaming typed records, not action-cue extraction. | `clawgs-portfolio-reality-idea-plan-v6n.12` |
| 3 | Emit stdio startup/cadence | p50 5.78 ms for 20 offline syncs; prior CPU profile ranks startup/debug-symbol, clap/anstream, reqwest init, then chrono | `emit-stdio-disabled/baseline.json`, `tests/artifacts/perf/emit-stdio/hotspot_table.md` | For long-lived stdio daemon use, startup costs amortize away. Per-request cost is small; chrono timestamp parsing remains the largest known per-request CPU component. | Existing emit optimization should wait for daemon-only attribution. |
| 4 | Perf harness reliability | `scripts/perf/emit_stdio_scenario.sh` blocked with checked-in model-enabled fixture in this environment | interrupted smoke run, successful offline command in `emit-stdio-disabled/baseline.json` | The perf script should be offline by default so repeat profiling cannot accidentally depend on model/backend behavior. | `clawgs-portfolio-reality-idea-plan-v6n.11` |
| 5 | Memory high-water | Extract 18.6 MiB RSS, tmux 9.0 MiB, emit 6.4 MiB | `*/time-l.txt` | No memory pressure observed on this corpus. Heap attribution remains a gap because `dhat-rs` is not wired. | No memory optimization bead recommended. |

## Hypothesis Ledger

| Hypothesis | Verdict | Evidence |
|---|---|---|
| Large transcript extraction is parser/JSON CPU-bound, not file I/O-bound. | supports | Current extract p50 is 53.35 ms for 2.75 MB; prior extract hotspot table ranks `read_jsonl`, `serde_json::from_str::<Value>`, and `serde_json::Value` allocation/drop above action extraction. |
| Action-cue/tool-use extraction is the top parser hotspot. | rejects | Prior extract hypothesis ledger shows `tool_use_action` at only 11/498 samples while generic `Value` parse/allocation dominates. |
| Emit stdio serialization/flush dominates cadence. | rejects | Prior emit hotspot table gives serde_json only 1.0% self / 5.4% inclusive and no top write/flush frames; current offline 20-sync command p50 is 5.78 ms. |
| Model/backend work can contaminate stdio perf measurements. | supports | `scripts/perf/emit_stdio_scenario.sh` did not exit promptly with its checked-in `enabled=true` fixture; the same fixture shape completed immediately when transformed to `enabled=false`. |
| Tmux scan cost should be measured with a fake tmux path for safe baseline work. | supports | `tmux_fake_scenario.sh` completed deterministically without touching a live tmux server or private pane contents; baseline artifacts capture the 64-pane shape. |
| Memory is currently the limiting constraint. | rejects | Peak RSS stayed below 19 MiB across all measured command-level scenarios. |
| The tmux subprocess batching optimization already moved the largest historical tmux lever. | supports | Prior `tests/artifacts/perf/tmux-emit/AFTER.md` shows live tmux p50 improved from 1442 ms to 723 ms and spawn+pipe CPU share dropped from 82.09% to 25.80%. Current fake 64-pane p50 is 161.51 ms command-level. |

## Recommended Next Beads

- `clawgs-portfolio-reality-idea-plan-v6n.12`: prototype a measured typed/streaming parser path that reduces `serde_json::Value` churn while preserving `clawgs.v2` and `malformed_lines_skipped` behavior.
- `clawgs-portfolio-reality-idea-plan-v6n.11`: make the stdio perf scenario offline by default, with model-enabled flow opt-in.
- Re-scope or review `clawgs-portfolio-reality-idea-plan-v6n.10` before implementation: this profile says action-cue handling is not the dominant extract hotspot.

## Reproduction Notes

```bash
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile release-perf --bin clawgs
bash scripts/perf/env_fingerprint.sh
bash scripts/perf/bench_baseline.sh --name profile-large-corpus-extract --out tests/artifacts/perf/profile-large-corpus-2026-06-01/extract --warmup 3 --runs 20 -- 'bash scripts/perf/extract_scenario.sh'
bash scripts/perf/bench_baseline.sh --name profile-large-corpus-emit-stdio-disabled --out tests/artifacts/perf/profile-large-corpus-2026-06-01/emit-stdio-disabled --warmup 3 --runs 20 -- 'jq -c '\''.config.enabled=false'\'' scripts/perf/fixtures/emit_stdio.ndjson | OPENROUTER_API_KEY=fake target/release-perf/clawgs emit --stdio >/dev/null'
bash scripts/perf/bench_baseline.sh --name profile-large-corpus-tmux-fake --out tests/artifacts/perf/profile-large-corpus-2026-06-01/tmux-fake --warmup 3 --runs 20 -- 'TMUX_FAKE_PANES=64 TMUX_FAKE_CAPTURE_LINES=120 TMUX_FAKE_MAX_CAPTURE_LINES=200 bash scripts/perf/tmux_fake_scenario.sh'
/usr/bin/time -l bash scripts/perf/extract_scenario.sh > /dev/null 2> tests/artifacts/perf/profile-large-corpus-2026-06-01/extract/time-l.txt
/usr/bin/time -l bash -lc 'jq -c '\''.config.enabled=false'\'' scripts/perf/fixtures/emit_stdio.ndjson | OPENROUTER_API_KEY=fake target/release-perf/clawgs emit --stdio >/dev/null' 2> tests/artifacts/perf/profile-large-corpus-2026-06-01/emit-stdio-disabled/time-l.txt
/usr/bin/time -l bash -lc 'TMUX_FAKE_PANES=64 TMUX_FAKE_CAPTURE_LINES=120 TMUX_FAKE_MAX_CAPTURE_LINES=200 bash scripts/perf/tmux_fake_scenario.sh' > /dev/null 2> tests/artifacts/perf/profile-large-corpus-2026-06-01/tmux-fake/time-l.txt
```
