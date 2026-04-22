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


## Attribution pass (debug=2, lto=off) — 2026-04-22

Rebuilt with a new `release-perf-attr` profile (`inherits = "release-perf"`, `lto = "off"`, `codegen-units = 16`, `debug = 2`) and reprofiled tmux-emit for 30s against the **current dirty working tree** (the committed baseline measured pre-refactor tmux.rs; this run includes the in-progress `TmuxScanTracker` surface). 5669 samples at 1ms interval. Symbol resolution via `atos` against the release-perf-attr binary and libsystem_kernel.dylib.

Artifact: `tests/artifacts/perf/tmux-emit/cpu-attr.json`.

### Corrected top-15 leaves (self-time, clawgs-deepest frame)

| Rank | % | Symbol | Location |
|------|---|--------|----------|
| 1 | 56.85% | `std::sys::pal::unix::pipe::read2` | subprocess stdout pipe read |
| 2 | 25.24% | `std::sys::process::unix::Command::spawn` | fork+exec for `tmux capture-pane` |
| 3 | 6.53% | `std::sys::fs::unix::File::read_buf` | file read (subprocess I/O path) |
| 4 | 1.92% | `std::sys::fs::unix::File::open_c` | file open (spawn path) |
| 5 | 1.08% | `std::os::unix::net::datagram::UnixDatagram::recv` | expected idle socket wait |
| 6 | 0.46% | `std::process::Command::output` | subprocess driver |
| 7 | 0.46% | `<std::fs::ReadDir as Iterator>::next` | directory traversal |
| 8 | 0.37% | `<core::str::lossy::Utf8Chunks as Iterator>::next` | UTF-8 validation of tmux output |
| 9 | 0.30% | `std::io::default_read_to_end` | reading subprocess stdout to String |
| 10 | 0.26% | `alloc::raw_vec::RawVecInner::finish_grow` | Vec growth |
| 11 | 0.23% | `alloc::vec::Vec::reserve` | Vec reserve |
| 12 | 0.21% | `Vec::<T>::spec_extend` | Vec extend |
| 13 | 0.19% | `serde_json::read::SliceRead::skip_to_escape` | JSON parse (minor) |
| 14 | 0.19% | `alloc::raw_vec::RawVecInner::try_allocate_in` | raw alloc |
| 15 | 0.18% | `<core::str::lossy::Utf8Chunks as Iterator>::next` | UTF-8 validation (inlined variant) |

**Combined subprocess-I/O related: ~92% of CPU self-time.**
**Chrono parsing symbols: 0% of CPU self-time.** `__commpage_gettimeofday_internal` (the underlying syscall for `Utc::now()`) appears at 0.46% — negligible.

### Verdict on prior hypotheses

- **H1 (chrono datetime parsing dominates):** **REJECTED under trusted attribution.** The prior 66.8% chrono figure was LTO-thin symbol misattribution. The chrono::format::parse symbols the release-perf build reported at offsets 0x9498 and 0x7e44 resolve to `thread_policy_get` and `guarded_write_np` in `libsystem_kernel.dylib` under debug=2/lto=off. With frame pointers preserved, stack unwinding lands in `std::sys::pal::unix::pipe::read2` and `Command::spawn`, not chrono. Any optimization targeting chrono would move the needle by less than 1%.
- **H2 (per-pane subprocess spawning is primary bottleneck):** **CONFIRMED and strengthened.** It is not just the wall-clock dominator — it is also the CPU dominator. Reducing `Command::spawn` + `pipe::read2` collectively (90%+ of cost) is the only lever large enough to meaningfully move p50.
- **H3/H4/H5:** unchanged (still rejected).

### Why the old attribution was wrong

Two multiplicative factors:
1. `lto = "thin"` + `opt-level = 3` inlines aggressively across crate boundaries; the linker discards Rust frame pointers in favor of chained tail calls into libc/libsystem, so `samply`'s frame-pointer unwinder walked into libsystem_kernel.dylib and stopped at whatever PC was closest — but the DWARF mapping for those offsets landed on chrono symbols that had been inlined near the same code region.
2. `debug = "line-tables-only"` keeps line info but drops the detailed unwind info needed to cross the Rust↔libsystem boundary cleanly.

The `release-perf-attr` profile (`debug = 2`, `lto = "off"`, `codegen-units = 16`) gives full DWARF + real call frames, and the correct picture falls out immediately.

### Implications for optimization

The only meaningful lever is reducing subprocess count. Candidate approaches, cheapest-first:

1. **Batch `capture-pane` calls into one tmux invocation** using a composite command with `\;` separators (e.g. `tmux capture-pane -t s1.0.0 \; capture-pane -t s1.0.1 \; ...`). Reduces fork+exec from O(panes) to O(1) per scan.
2. **Switch to tmux control mode** (`tmux -C attach`) for a persistent bidirectional pipe; reuse a single tmux client across scans. Highest engineering cost but eliminates fork cost entirely.
3. **Pre-filter panes by `display-message` batch** to skip idle panes without capturing; only capture panes that changed.

Recommended iter-2 target: option 1 (composite `tmux capture-pane \; capture-pane \;` invocation). Lowest code change, largest expected p50 reduction, does not require rethinking the scan loop architecture.

### Caveat carry-forward

- Profile ran against the dirty tree (which includes the `TmuxScanTracker` refactor). The subprocess counts and fork+exec cost are measured against the shippable shape, not the committed baseline state. If the refactor lands before the optimization, re-baseline before accepting the committed baseline as before-state.
- `release-perf-attr` is an investigation-only profile — it must not ship. `release-perf` remains the reference profile for before/after comparison.

