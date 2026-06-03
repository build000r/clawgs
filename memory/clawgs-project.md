---
name: clawgs-project
description: clawgs is Rob's Rust CLI/lib that normalizes Claude/Codex JSONL transcripts into clawgs.v2 snapshots and emits clawgs.emit.v2 NDJSON status
metadata:
  type: project
---

`clawgs` (`/Users/b/repos/opensource/clawgs`) is an open-source Rust 2021 crate (rust-version 1.85, currently v0.3.0) that normalizes Claude Code and Codex JSONL transcripts into stable `clawgs.v2` snapshots and emits a live `clawgs.emit.v2` NDJSON status protocol. Downstream consumer is "Swimmers", which spawns `clawgs emit --stdio` once and syncs ~every 2s.

Core modules: `src/lib.rs` (transcript discovery + ActionCue/CommitSignal contracts), `src/parsers/{claude,codex}.rs` (codex.rs has the commit-signal state machine: edit→validate→dirty-check→commit), `src/emit/engine.rs` (3981-line thought-emission state machine — sleeping/wake/cadence/objective-fingerprint), `src/emit/protocol.rs` (serde wire contracts), `src/emit/model_client.rs` (OpenRouter + Grok CLI backends), `src/tmux.rs` (pane scan→SessionSnapshot), `src/main.rs` (clap CLI + unix datagram notify socket).

Baseline as of 2026-05-29: 195 tests pass, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt -- --check` clean. The code is meticulously tested with regression tests carrying explanatory comments. Contracts `clawgs.v1/v2` + `clawgs.emit.v1/v2` are downstream-facing — keep them stable. Parsers are intentionally tolerant of malformed JSONL (`malformed_lines_skipped`). Demo paths must stay zero-config (no creds/tmux). Rob does not accept outside PRs — see [[clawgs-readme-no-contributions]] context. AGENTS.md mandates keeping test/clippy/fmt green and updating tests when changing parser/emit/backend/tmux/CLI behavior.
