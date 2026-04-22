# Hotspot Table — tmux-emit Steady-State (Scenario A)

> **⚠ Historic / superseded attribution.** Ranks 1–2 in this table (`chrono::format::*`)
> turned out to be LTO-thin symbol misattribution of syscall trampolines in
> `libsystem_kernel.dylib`. See `hypothesis_ledger.md` § *Attribution pass (debug=2)*
> for the corrected ranking and `AFTER.md` for the before/after pack shipped in
> commit `42a45eb` (`perf(tmux): batch capture-pane into one tmux invocation`).
> Kept as-is as the committed record of what release-perf profiling originally reported.

Ranked by CPU self-time from a 25.3s samply profile (6272 samples, 1ms interval).
Total CPU consumed: ~1507ms across 25.3s wall time (6.0% utilization; remainder is socket-wait idle).

| Rank | Location | Metric | Value | Category | Evidence |
|------|----------|--------|-------|----------|----------|
| 1 | `chrono::format::scan::short_or_long_month0` (scan.rs:143) | CPU self-time | 589.7ms (39.1%) | datetime_parsing | cpu.json thread[3] leaf 0x7e44; callers: `Command::spawn` <- `Command::output` <- `scan_with_bin` |
| 2 | `chrono::format::parse::parse_internal` (parse.rs:434) | CPU self-time | 418.0ms (27.7%) | datetime_parsing | cpu.json thread[3] leaf 0x9498; callers: `pipe::read2` <- `Command::output` <- `FilterMap::next` <- `scan_with_bin` |
| 3 | `alloc::str::to_lowercase` | CPU self-time | 103.5ms (6.9%) | string_normalization | cpu.json thread[3] leaf 0x17dc+0x17b0; likely from `infer_tool` / `is_shell_command` normalized_command path |
| 4 | `FromUtf8Error::Display::fmt` | CPU self-time | 64.3ms (4.3%) | error_formatting | cpu.json thread[3] leaf 0x1678; likely from `String::from_utf8_lossy` on tmux subprocess output |
| 5 | `bytes::promotable_odd_to_vec` (socket-wait path) | Wall self-time | 8838ms (35.0% wall, 1.3% CPU) | io_idle | cpu.json thread[3] leaf 0x5158; caller: `should_scan_tmux` <- `run_tmux_emit_loop:457`; expected idle |

## Notes

- Ranks 1-2 combined: **chrono datetime parsing = 66.8% of all CPU**. Root cause unclear under LTO — no explicit `DateTime::parse_from_str` in production codepaths. Likely inlined from serde `DateTime<Utc>` serialization or `Utc::now()` path, misattributed by LTO to chrono parse symbols. Needs investigation with `debug = 2` build.
- Rank 5 is intentional idle — the process sleeps on a Unix datagram socket between scans. Listed for completeness as it dominates wall-clock.
- Off-CPU profiling skipped (macOS SIP blocks dtrace; xctrace System Trace not wired).
- Heap profiling skipped (dhat-rs not wired in `src/`).
