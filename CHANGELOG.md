# Changelog

All notable user-facing and agent-facing changes are documented here.

This changelog is reconstructed from git tags, commits, GitHub Release metadata,
and repository docs. GitHub Releases are called out separately when they differ
from plain git tags.

## Version Timeline

| Version | Date | Status | Evidence |
| --- | --- | --- | --- |
| `0.2.0` | 2026-05-06 | Tagged and published as a GitHub Release from `7856897` | [`v0.2.0`](https://github.com/build000r/clawgs/releases/tag/v0.2.0), [`7856897`](https://github.com/build000r/clawgs/commit/7856897) |
| `0.1.0` | 2026-04-06 | Git tag only; no GitHub Release object found | [`v0.1.0`](https://github.com/build000r/clawgs/releases/tag/v0.1.0) |

## [0.2.0] - 2026-05-06

`0.2.0` is the canonical v2 schema/protocol release. It promotes the public
contract from a v1 snapshot extractor into a documented `clawgs.v2` extract
schema plus `clawgs.emit.v2` NDJSON protocol with live action cues, demoable
protocol output, and stricter schema validation.

### Contract And Schema

- Added `clawgs.v2` extract documentation and JSON Schema.
- Added `clawgs.emit.v2` protocol documentation and JSON Schema.
- Kept v1 references and schemas checked in for downstream comparison.
- Added schema synchronization tests so public docs and runtime serialization
  cannot silently drift.

Representative commits:

- [`012ea6b`](https://github.com/build000r/clawgs/commit/012ea6ba4de63197dcab463498b8900ff783e347) published JSON Schemas and schema sync coverage.
- [`4b2d39a`](https://github.com/build000r/clawgs/commit/4b2d39ace1d9d97c992bd98bedf570fd6ee73806) added schema v2 protocol support and updated docs/metadata.

### Emit Protocol And Thought Engine

- Plumbed `awaiting_user` state through parsers, snapshots, and the emit engine.
- Added model-backed thought summaries with backend configuration for live emit
  workflows.
- Improved generated demo summary accounting and prompt normalization.
- Preserved the zero-config `demo emit` path so the protocol remains visible
  without credentials, private transcripts, or tmux.

Representative commits:

- [`731c7af`](https://github.com/build000r/clawgs/commit/731c7af85a8361b449e27fbfefb2301db4f7b8fe) plumbed awaiting-user state through parsers and engine.
- [`a55e4fe`](https://github.com/build000r/clawgs/commit/a55e4fe8fda3f8e6bab268a5f5dd88228d7ca193) added model-backed thought summaries.
- [`5098548`](https://github.com/build000r/clawgs/commit/5098548ed3bee38444a90fafcdce70400ad01be2) improved model backend emission.

### Parser And Tmux Correctness

- Added Codex task-event extraction.
- Tightened i18n and commit-signal heuristics.
- Fixed markup-prefixed user replies so they clear `awaiting_user`.
- Kept live emit sessions from sticking to stale claimed transcripts when a
  valid newer discovery candidate is available.
- Introduced stateful tmux scan tracking.
- Batched tmux capture into one invocation and nonce-marked capture batches.
- Avoided zero socket timeouts in tmux notification flows.
- Rejected invalid `tmux-emit --interval-ms` values before scheduling.
- Avoided treating historical `git diff` ranges as dirty-tree proof in Codex
  action-cue extraction.

Representative commits:

- [`07771e6`](https://github.com/build000r/clawgs/commit/07771e6cbb10f4cacdd88c1f5b4fab850d3a08c2) introduced `TmuxScanTracker`.
- [`42a45eb`](https://github.com/build000r/clawgs/commit/42a45ebbb84cd5f7e2f66f3428a1086b1b48063e) batched tmux capture.
- [`c72981d`](https://github.com/build000r/clawgs/commit/c72981d8ac75f52d5abd8f26107f55f3b44b5369) emitted Codex task events.
- [`fb3aefd`](https://github.com/build000r/clawgs/commit/fb3aefdf84a162afb28ce76b1986f355bedc1bbf) fixed markup-prefixed user replies.

### Performance And Hardening

- Tuned release profiles for the long-lived `clawgs emit --stdio` daemon.
- Added release/performance profiles and scenario baselines.
- Added performance artifacts for extract, demo extract, emit stdio, and tmux
  emit paths.
- Reduced CRAP hotspots through helper extraction and targeted tests.
- Covered model client, engine, protocol, tmux, parser, and library helpers.
- Terminated model backend subprocesses when prompt writes fail so failed
  backend calls do not leave sleeping children behind.
- Refreshed the lockfile to the current `rustls-webpki` patch release.
- Pinned the `time` transitive dependency line to Rust-1.85-compatible
  versions so CI enforces the crate's declared MSRV.

Representative commits:

- [`8b76c71`](https://github.com/build000r/clawgs/commit/8b76c71c3f76ee8cdfc69f425353f23376972a39) tuned the release profile.
- [`d764d44`](https://github.com/build000r/clawgs/commit/d764d446f0af5b6acd7851bd9bdb80ba8e3f60c6) added release-perf profiles and baselines.
- [`d9b6a11`](https://github.com/build000r/clawgs/commit/d9b6a11e19e1b06f182ade8315dbec9d610c0547) covered CRAP hotspots.
- [`ef8dd20`](https://github.com/build000r/clawgs/commit/ef8dd200a8720c41e1e8b5099b116fce4cb8f275) lowered measured hotspots again.

### Release Readiness

- Added this changelog from the `v0.1.0..v0.2.0` history.
- Added release documentation and a GitHub Actions release workflow for tag
  verification and crates.io publishing.
- Updated local agent/operator release metadata so future handoffs no longer
  say that the release process is unknown.

## [0.1.0] - 2026-04-06

First crates.io release, tagged as `v0.1.0`.

The tag points at
[`5e9e6fd`](https://github.com/build000r/clawgs/commit/5e9e6fd),
which addressed pre-publish review fixups in `Cargo.toml` and `README.md`.

No GitHub Release object was found for this version.

[0.2.0]: https://github.com/build000r/clawgs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/build000r/clawgs/releases/tag/v0.1.0
