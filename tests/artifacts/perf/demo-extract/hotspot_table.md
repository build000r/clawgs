# Demo Extract Hotspot Table

Scenario: `clawgs demo extract --tool {codex,claude} --pretty` — 50 iterations each (100 total invocations per run).
Binary: `target/release-perf/clawgs` (release-perf profile, line-tables-only debug, no strip).
Profiler: samply @ 1ms interval, 400 invocations across the profile session.
Total clawgs-process samples: 1133. Per-invocation mean: 6.43ms.

| Rank | Hotspot | Self % | Inclusive % | File(s) | Evidence |
|------|---------|--------|-------------|---------|----------|
| 1 | **Temp file I/O** (write + open + unlink + fcntl + lseek) | 17.5% (193 leaf in kernel) | ~30% (inclusive: fs::write 90 + remove_file 108 + File::open_c 118 samples) | `src/demo.rs:104-116` (write_demo_temp_file), `src/demo.rs:45` (fs::remove_file) | Dominant kernel syscalls: `__fcntl` 117, `__unlink` 58, `__lseek` 18. The demo path writes the `include_str!` content to a temp file, then parses it, then deletes it. This round-trip is the single largest cost center. |
| 2 | **Process startup / dyld** (dynamic linker + runtime init + lang_start) | 14.2% (161 dyld leaf) | ~20% (inclusive: dyld 161 + unix::init 34 + lang_start 47 samples) | `std::rt::lang_start_internal`, `std::sys::pal::unix::init` | 161 pure dyld samples + 34 in `unix::init` + 47 in `lang_start`. At 6.43ms per invocation, ~1.3ms is just process startup. Unavoidable per-process but dominates because the actual work is so small. |
| 3 | **JSONL parsing** (serde_json deserialize + read_jsonl + line splitting) | ~3.0% (34 leaf in clawgs parse/deser) | ~8% (inclusive: read_jsonl 61 + extract 63 + serde_json::de 33 samples) | `src/parsers/mod.rs:33-45` (read_jsonl), `src/lib.rs:157` (extract), `serde_json::de.rs` | Inclusive 63 samples in `clawgs::extract`. Within that, `read_jsonl` at 61 inclusive is the bulk: file read + line-by-line serde_json deserialization. The `has_next_key` hot loop in serde_json (11 self samples) is the hottest single clawgs-binary leaf. |
| 4 | **JSON serialization** (`--pretty` output formatting) | ~3.5% (40 leaf in clawgs ser) | ~5% (inclusive: Action::serialize 9 + ExtractOutput::serialize 7 + format_escaped_str 11 + SerializeStruct 7) | `src/lib.rs:61` (Action::serialize), `src/lib.rs:111` (ExtractOutput::serialize), `serde_json::ser.rs:2107` | Serializing the `ExtractDemoOutput` struct through serde_json pretty-printer. `format_escaped_str_contents` (11 self) is the hottest serialization leaf — escaping string content for JSON output. |
| 5 | **Heap allocation** (malloc/free + mmap for heap) | 7.2% (81 malloc leaf) | ~12% (inclusive: malloc 81 + __mmap partial ~30 + Vec grow 5) | `libsystem_malloc.dylib`, `alloc::raw_vec::RawVec::grow_one` (mod.rs:334) | 81 samples in `libsystem_malloc.dylib` plus ~30 mmap samples attributable to heap growth. Many small allocations from String/Vec creation during JSONL parsing and JSON serialization. `RawVec::grow_one` (5 samples) confirms dynamic Vec growth during parse. |
| 6 | **Clap argument parsing** | ~2.0% (23 inclusive) | ~2% (23 samples in `Command::_do_parse`) | `clap_builder::builder::command.rs:4360` | 23 inclusive samples in clap dispatch. Modest but non-trivial at 6ms total — clap's type-level subcommand resolution is paying ~0.13ms per invocation. |
| 7 | **String processing** (trim_matches, format) | ~1.3% (15 leaf) | ~2% | `core::str::trim_matches`, `alloc::fmt::format::format_inner` | 8 self samples in `trim_matches`, 7 in `format_inner`. Parser post-processing: trimming whitespace from parsed fields and formatting display strings. |

**Notes:**
- "Inclusive %" estimates are approximate — samply's stack walk on short-lived processes can miss frames.
- Kernel syscall samples (39.1% of total) are dominated by temp file I/O, confirming the write-parse-delete round-trip as the primary bottleneck.
- The `__recvfrom_nocancel` (108 samples, 9.5%) is likely from macOS `NSTemporaryDirectory()` calling into mDNS/configd — an artifact of `std::env::temp_dir()` on macOS, not network I/O.
