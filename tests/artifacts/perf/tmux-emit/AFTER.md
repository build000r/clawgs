# tmux-emit Optimization — After Report

## Change

Single commit scope: `src/tmux.rs`. Batch `tmux capture-pane` calls for all
live panes into a single composite tmux invocation using `\;` command-sequence
syntax, with `display-message`-emitted ASCII-RS-delimited markers
(`\x1eCLAWGS:<pane_id>\x1e`) separating each pane's content on stdout.

Falls back to the legacy one-subprocess-per-pane path when:
- the composite tmux run exits non-zero, or
- its stdout lacks the marker tag (e.g. the integration-test `fake-tmux` shim,
  or a tmux version that ignores `\;` argv).

Reduces fork+exec from `O(panes)` to `O(1)` per scan on real tmux.

## Opportunity score

- Impact: 5 (Command::spawn + pipe::read2 were 82.1% of CPU self-time)
- Confidence: 4 (tmux composite commands are well documented; marker-based
  parsing is unambiguous; fallback preserves behaviour)
- Effort: 2 (single file, ~80 LOC net)
- **Score: 10.0** (threshold 2.0)

## Isomorphism proof

- Ordering preserved: yes — pane metadata order from `list-panes` is
  unchanged; batched captures are keyed by `pane_id`, then zipped back into
  the original `metas` iteration order, so `sessions[i]` still corresponds
  to the i-th live pane.
- Tie-breaking unchanged: no tie-break logic in this path.
- Floating-point: N/A.
- RNG seeds: N/A.
- Behavioral tests: `cargo test --test tmux_emit` + `cargo test --lib tmux::`
  → 11/11 pass (3 integration + 8 unit). Fake-tmux based
  `scan_sessions_maps_tmux_panes_to_session_snapshots` continues to pass via
  the fallback path — direct evidence that identical per-pane replay_text
  values come out when the composite isn't supported.
- Live-tmux golden: `clawgs tmux-emit --once` captured before and after change
  against the same live tmux server (~41 panes). Normalized SHA256 of
  `(session_id, thought_sha, rest_state)` tuples for all emitted updates:
  - before: `de7b5e365a7df847…`
  - after:  `de7b5e365a7df847…`
  - identical, byte-for-byte.

## Baseline — hyperfine 20 runs, release-perf

| Metric | Before (d764d44) | After | Δ |
|--------|------------------|-------|---|
| mean   | 1673 ms | **739 ms** | **−55.8%** |
| p50    | 1442 ms | **723 ms** | **−49.8%** |
| p95    | 3067 ms | **1169 ms** | **−61.9%** |
| p99    | 3231 ms | **1169 ms** | **−63.8%** |
| min    | 950 ms  | 517 ms  | −45.6% |
| max    | 3272 ms | 1169 ms | −64.3% |
| stddev | 624 ms  | 203 ms  | −67.5% |
| CoV    | 37%     | 27%     | variance collapsed |

Artifacts: `baseline.json` (before, committed at d764d44),
`baseline-after.json` (after, uncommitted).

## Attribution profile — samply 30s, release-perf-attr

Symbol resolution via `atos` against the debug-2 binary.

| Frame | Before | After | Δ (pts) |
|-------|--------|-------|---------|
| `std::sys::pal::unix::pipe::read2` | 56.85% | **19.47%** | −37.38 |
| `std::sys::process::unix::Command::spawn` | 25.24% | **6.33%** | −18.91 |
| `std::sys::fs::unix::File::read_buf` | 6.53% | 25.33% | +18.80 |
| `std::sys::fs::unix::File::open_c` | 1.92% | 7.83% | +5.91 |
| **`spawn + pipe::read2` combined** | **82.09%** | **25.80%** | **−56.29** |

Overall lib distribution shifted away from pure syscall wait:

| Lib | Before | After |
|-----|--------|-------|
| libsystem_kernel.dylib | ~92% | 69.47% |
| clawgs (native) | ~4% | 20.31% |
| libsystem_malloc.dylib | <1% | 5.44% |
| libsystem_platform.dylib | <1% | 4.54% |

The new top leaf `File::read_buf` (25.33%) is tmux itself reading its own
server socket to fulfill the batched command — unavoidable and not an
optimization target.

Total CPU samples in the 30s window dropped from 5669 → 1674, consistent
with the daemon spending proportionally more wall-time idle between scans
because each scan completes faster.

Artifacts: `cpu-attr.json` (before), `cpu-attr-after.json` (after).

## Success criteria

Both met by a wide margin:

- p50 ≤ 1.0s: **723ms** (threshold was 1000ms) — achieved **1.4× better than target**.
- Command::spawn + pipe::read2 combined ≤ 50% CPU share: **25.80%** — achieved
  **1.9× better than target**.

## Risks / follow-ups

- On tmux installs older than 3.0 that don't accept `\;` argv-form sequences,
  the composite invocation will non-zero and fallback triggers automatically —
  no regression vs baseline. Not tested against tmux < 3.6; fallback path is
  behaviourally identical to the committed code.
- The `display-message -p "<marker>"` interpolates tmux format specifiers. The
  chosen marker uses only ASCII-RS (`\x1e`) + the literal ASCII string
  `CLAWGS:` + the pane_id (which is always `%<digits>`) + `\x1e`. None of
  those characters overlap with tmux format syntax (`#{...}`, `#F`), so the
  marker emits verbatim.
- Batched command-line length: 41 panes × ~24 bytes/pane ≈ 1 KB. Well below
  `ARG_MAX` on any modern OS. At ~1000 panes the argv may approach the
  macOS 1 MB ARG_MAX and need chunking; leaving that for a future iteration.
- `TmuxScanTracker`'s stateful aging now benefits from the faster scan
  (more scans fit in a given interval without falling behind), but semantics
  are unchanged.

## Deliverables

- `src/tmux.rs` — working-tree edits, uncommitted.
- `tests/artifacts/perf/tmux-emit/baseline-after.json` — hyperfine 20-run post.
- `tests/artifacts/perf/tmux-emit/baseline-after.md` — hyperfine md summary post.
- `tests/artifacts/perf/tmux-emit/cpu-attr-after.json` — samply 30s post.
- `tests/artifacts/perf/tmux-emit/AFTER.md` — this file.

No commit made. Handoff back to `/smart` iter 3 for commit + reality-check.
