# Demo Extract Hypothesis Ledger

Scenario: `clawgs demo extract --tool {codex,claude} --pretty`
Per-invocation mean: 6.43ms (100 invocations per scenario run, 50 codex + 50 claude).

---

## H1: Temp file round-trip dominates and can be eliminated

**Verdict: supports**

The demo path writes `include_str!` content to a temp file (`write_demo_temp_file` in `src/demo.rs:104-116`), then reads it back via `extract()`, then deletes it. This round-trip accounts for ~30% of inclusive time:
- `std::fs::write::inner`: 90 inclusive samples
- `std::sys::fs::remove_file`: 108 inclusive samples
- `std::sys::fs::unix::File::open_c`: 118 inclusive samples
- Kernel leaf: `__fcntl` 117, `__unlink` 58, `__lseek` 18

The embedded corpus is already in memory via `include_str!`. If `extract()` accepted a `&str` or `Read` trait object instead of requiring a file path, the entire temp-file round-trip (~2ms per invocation) could be eliminated. This is the single highest-leverage optimization for the demo extract path.

---

## H2: `--pretty` serde_json indentation dominates serialization cost

**Verdict: rejects (weak)**

Serialization is visible in the profile (~5% inclusive) but does not dominate. The hottest serialization leaf is `format_escaped_str_contents` (11 self samples), which runs regardless of `--pretty` — it escapes special characters in JSON strings. The pretty-printing overhead (indentation whitespace) is not separately visible in the profile, suggesting it's a small fraction of serialization time. The extract path's bottleneck is I/O, not formatting.

---

## H3: Process startup (dyld + runtime init) dominates for single invocations

**Verdict: supports**

At 6.43ms per invocation, process startup consumes ~1.3ms (~20% of wall time):
- 161 dyld samples (14.2% of total leaf)
- 34 samples in `std::sys::pal::unix::init`
- 47 samples in `std::rt::lang_start`

This is unavoidable per-process cost. For a CLI tool invoked once, it's acceptable. For batch use (many files), the startup cost argues for a `--batch` mode or library API that amortizes startup across multiple extractions.

---

## H4: Allocator overhead dominates due to many small strings

**Verdict: rejects**

`libsystem_malloc.dylib` accounts for 81 leaf samples (7.2%) and `RawVec::grow_one` appears with only 5 samples. While allocation is measurable, it does not dominate — temp file I/O and process startup together account for ~50% of time. The small-string allocation pattern (from JSONL line parsing and JSON field creation) is real but secondary. An arena allocator or pre-sized Vec would save ~0.5ms per invocation at best.

---

## H5: Embedded corpus deserialization could be lazy_static or cached

**Verdict: supports (conditional)**

`clawgs::parsers::read_jsonl` at 61 inclusive samples (~5.4%) includes reading the file and parsing each JSONL line via `serde_json::from_str`. Since the embedded corpus is static, the parsed representation could be cached via `lazy_static` or `std::sync::OnceLock` — but only if `extract()` were refactored to accept pre-parsed lines. Currently the function takes a file path, so caching would require an API change. The win is modest (~0.35ms per invocation) compared to eliminating the temp file (H1), which subsumes this optimization.

---

## Summary

| # | Hypothesis | Verdict | Estimated savings |
|---|-----------|---------|-------------------|
| H1 | Temp file round-trip is eliminable | **supports** | ~2ms/invocation (~30%) |
| H2 | `--pretty` indentation dominates | **rejects** | <0.1ms |
| H3 | Process startup dominates single invocations | **supports** | ~1.3ms (unavoidable per-process) |
| H4 | Allocator overhead from small strings | **rejects** | ~0.5ms (secondary) |
| H5 | Corpus parse can be lazy_static cached | **supports** | ~0.35ms (subsumed by H1) |
