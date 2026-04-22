# Emit --stdio Hypothesis Ledger

Scenario: 20 NDJSON `sync` round trips (2 sessions per tick) via `clawgs emit --stdio`.
Profile: samply CPU, 100 invocations, 1565 samples. Baseline: p50 56ms, mean 66ms over 20 hyperfine runs.

---

## H1: serde_json serialization/deserialization dominates the sync loop

**Verdict: rejects**

serde_json accounts for only 1.0% self-time (15/1565 samples) and 5.4% inclusive (85/1565).
The sync request is ~700 bytes per line with 2 sessions — small enough that JSON parsing
is negligible. The engine processes 20 sync ticks with 40 total session snapshots in well
under 1ms of CPU time on the serde path.

Evidence: `tests/artifacts/perf/emit-stdio/span_summary.json` (serde_json entry).

---

## H2: stdio write/flush per frame dominates

**Verdict: rejects**

No stdio write or flush functions appear in the top 25 self-time functions. The stdout
is redirected to `/dev/null` in the scenario, and even when captured, the sync_result
payloads are small (empty `updates` arrays). The BufWriter on stdout.lock() flushes
efficiently. Zero samples attributable to write/flush.

Evidence: `tests/artifacts/perf/emit-stdio/cpu.json` — no `std::io::Write` or `BufWriter::flush`
in sampled stacks.

---

## H3: chrono DateTime parsing per frame dominates the per-request path

**Verdict: supports**

`chrono::format::scan::timezone_offset_2822` accounts for 13.1% self-time (205 samples)
and `NaiveDateTime::checked_sub_offset` adds 2.7% (43 samples), totaling 15.8% self-time.
Each sync request contains `now` and per-session `last_activity_at` DateTime fields parsed
from RFC3339 strings. With 20 requests × 2 sessions = 40 session snapshots plus 20 `now`
fields, chrono parses 60 DateTime values per invocation. This is the single largest
per-request CPU cost.

Evidence: `tests/artifacts/perf/emit-stdio/span_summary.json` (chrono entries).

---

## H4: reqwest/tokio client initialization dominates the cold-start path

**Verdict: supports**

`reqwest::ClientBuilder::build` shows 34.0% inclusive time (532/1565 samples). This includes
tokio runtime thread spawning, system proxy detection (`hyper_util::client::proxy::matcher`),
TLS initialization, and DNS resolver setup. All of this runs once at startup in
`build_model_client_for()`, before the first sync line is read. In the steady-state
(long-running daemon), this cost amortizes to zero, but for the benchmark scenario where
each invocation is a fresh process, it dominates.

Evidence: `tests/artifacts/perf/emit-stdio/span_summary.json` (reqwest entry),
`tests/artifacts/perf/emit-stdio/cpu.json` inclusive stacks.

---

## H5: addr2line/backtrace symbol table loading dominates process startup

**Verdict: supports**

`addr2line::line::path_push` (24.7%) and `addr2line::line::Lines::find_location` (13.0%)
combine for 37.7% of all self-time samples (590/1565). These are triggered by the
debug/backtrace infrastructure linked into the binary. In a long-lived daemon process,
this one-time cost amortizes away, but it is the single largest contributor to per-invocation
wall time. The `release-perf` profile includes debug info for frame-pointer profiling,
which inflates the addr2line symbol tables.

Evidence: `tests/artifacts/perf/emit-stdio/span_summary.json` (addr2line entry),
`tests/artifacts/perf/emit-stdio/cpu.json`.

---

## H6: async runtime spawn/poll overhead per request dominates

**Verdict: rejects**

tokio async runtime accounts for only 0.1% self-time (2/1565 samples). The `emit --stdio`
path uses a synchronous `for line in stdin.lock().lines()` loop — no async dispatch per
request. The tokio runtime is spawned by reqwest for its internal blocking-to-async bridge,
but it mostly parks idle (`tokio::runtime::time::Driver::park_internal`). There is no
per-request async overhead.

Evidence: `tests/artifacts/perf/emit-stdio/span_summary.json`,
`tests/artifacts/perf/emit-stdio/cpu.json` — tokio/mio samples are parking, not polling.

---

## Note: off-CPU profiling

Skipped on darwin. SIP blocks dtrace-based off-CPU tools. The scenario runs in ~60ms and is
CPU-bound (no network, no disk I/O beyond binary load), so off-CPU analysis is unlikely to
reveal additional insights for this workload.
