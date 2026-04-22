# Hypothesis Ledger — tmux-emit Steady-State (Scenario A)

## H1: Chrono datetime parsing during subprocess output processing dominates CPU

**Verdict: supports**

Evidence:
- `chrono::format::scan::short_or_long_month0` + `chrono::format::parse::parse_internal` together consume **66.8% of CPU self-time** (1007.7ms of 1507.3ms total CPU) — see `span_summary.json`, `cpu.json` thread[3].
- Caller chains (from `cpu.json` stack traces) root in `scan_with_bin` → `FilterMap::next` → `Command::output` → `pipe::read2`, placing the parsing inside the per-pane tmux subprocess pipeline.
- No explicit `DateTime::parse_from_str` in `src/` production paths — the `chrono::parse_internal` call is likely from serde's `DateTime<Utc>` deserialization or `Utc::now()` side effects, heavily inlined by LTO thin + opt-level 3. A `debug = 2` build would confirm exact callsite.

Implication: An optimization pass should either (a) reduce chrono parsing frequency (e.g. cache `Utc::now()` per scan tick instead of per-pane), or (b) switch to a faster datetime representation for internal protocol types.


## H2: Per-pane tmux subprocess spawning (fork+exec) is the primary wall-clock bottleneck

**Verdict: supports (wall-clock) / rejects (CPU)**

Evidence:
- `std::process::Command::output` inclusive wall-time is ~61.4% (15508/25251 weight) in `cpu.json` thread[3] — from `scan_with_bin` calling `tmux capture-pane` once per live pane (~44 panes observed in test).
- `std::process::Command::spawn` inclusive: 10.7% wall-time (2696/25251 weight) — pure fork+exec overhead.
- **CPU fraction** for `Command::spawn` is modest: chrono dominates actual CPU cycles; the spawn overhead is largely kernel wait.
- Baseline variance (CoV 37%, p50=1.44s, p95=3.07s over 20 runs — see `baseline.json`) is consistent with subprocess-heavy workloads that are sensitive to system load and tmux server contention.

Implication: Batching pane captures (e.g. single `tmux capture-pane` call with multiple targets, or a tmux control-mode session) would reduce fork+exec count from O(panes) to O(1).


## H3: NDJSON serialize + flush per iteration dominates

**Verdict: rejects**

Evidence:
- `serde_json::read::SliceRead::skip_to_escape` appears at only **0.6% of CPU self-time** (8977us) in `cpu.json` — the only serde_json symbol with measurable presence.
- No `serde_json::to_writer` or flush-related symbols appear in the top-20 CPU self-time list (`span_summary.json`).
- The JSON serialization of `SyncResultMessage` (which includes `ThoughtUpdate` vectors) is negligible compared to the chrono and subprocess overhead.

Implication: NDJSON serialization is not a bottleneck and does not warrant optimization.


## H4: Diff-against-last-emission allocation churn dominates

**Verdict: rejects**

Evidence:
- `strip_ansi` (engine.rs:1098) — the main text-diffing/processing function — consumes only **0.7% of CPU** (10263us) in `cpu.json`.
- `alloc::str::to_lowercase` at 6.9% CPU is from the `normalized_command()` / `infer_tool()` path in tmux.rs, not from emission diffing.
- `alloc::str::join_generic_copy` at 2670 inclusive weight (10.6% wall) appears in the subprocess spawn chain, not in diff logic.
- No `HashMap` allocation or `Vec::push`/`Vec::extend` symbols appear in the top-20 self-time.

Implication: The emission engine's per-session state management and diff logic are not allocation-heavy enough to matter.


## H5: Tokio timer / poll overhead dominates

**Verdict: rejects**

Evidence:
- The binary uses **synchronous I/O** — `std::os::unix::net::UnixDatagram` blocking recv with `set_read_timeout`, not tokio.
- No tokio symbols appear anywhere in the profile (`cpu.json` thread list shows a single "clawgs" thread with 6272 samples; no tokio runtime threads).
- The socket-wait idle path (`should_scan_tmux`) accounts for 35% wall-time but only 1.3% CPU — this is expected blocking I/O, not async poll overhead.

Implication: Not applicable — the binary is synchronous. If future work adds async, this hypothesis should be revisited.
