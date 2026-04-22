# Emit --stdio Hotspot Table

Scenario: 20 NDJSON sync round trips (2 sessions each) via `clawgs emit --stdio`.
Profile: samply CPU sampling, 100 invocations, 1565 total samples.

| Rank | Span / Function | Self % | Category | Evidence Artifact |
|------|----------------|--------|----------|-------------------|
| 1 | `addr2line::line::{path_push, Lines::find_location}` | 37.7% | startup/addr2line | `tests/artifacts/perf/emit-stdio/span_summary.json` — 590/1565 samples. Backtrace/debug-symbol infrastructure loaded at process start. Not per-request. |
| 2 | `anstream::adapter::strip::Utf8Parser::add` | 26.3% | startup/anstream | `tests/artifacts/perf/emit-stdio/span_summary.json` — 411/1565 samples. ANSI UTF-8 stream parsing during clap terminal auto-detection. Not per-request. |
| 3 | `chrono::format::scan::timezone_offset_2822` | 13.1% | per-request/chrono | `tests/artifacts/perf/emit-stdio/span_summary.json` — 205/1565 samples. RFC2822 timezone offset parsing for DateTime fields (`now`, `last_activity_at`) in each sync request. Largest per-request cost. |
| 4 | `clap_builder::parser::{get_matches_with, Validator::validate}` | 4.9% | startup/clap | `tests/artifacts/perf/emit-stdio/span_summary.json` — 76/1565 samples. CLI argument parsing and validation at startup. |
| 5 | `chrono::naive::datetime::NaiveDateTime::checked_sub_offset` | 2.7% | per-request/chrono | `tests/artifacts/perf/emit-stdio/span_summary.json` — 43/1565 samples. Offset arithmetic after timezone parsing. Second-largest per-request cost. |
| 6 | `reqwest::async_impl::client::ClientBuilder::build` | 0.7% self / 34.0% inclusive | startup/reqwest | `tests/artifacts/perf/emit-stdio/span_summary.json` — 532/1565 inclusive samples. HTTP client + tokio runtime construction for model client. One-time startup cost. |
| 7 | `serde_json (SliceRead::skip_to_escape + Serializer)` | 1.0% self / 5.4% inclusive | per-request/serde | `tests/artifacts/perf/emit-stdio/span_summary.json` — 15 self / 85 inclusive. JSON parse + serialize for sync request/response. |
