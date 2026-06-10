---
name: clawgs
description: Extract clawgs.v2 JSON from Claude Code and Codex JSONL session logs and run the clawgs.emit.v2 thought-emission daemon over stdio. Use when asked to parse agent transcripts, normalize .jsonl sessions, auto-discover current Claude/Codex logs by cwd, or generate thought updates from live session snapshots for downstream tools.
license: MIT
---

# Clawgs

## Purpose

Use this skill to run deterministic transcript extraction and daemonized thought emission outside the main app process.

## Prerequisites

- Rust toolchain with `cargo`
- This skill checked out locally

## Install

```bash
bash scripts/install.sh
```

## Verify

```bash
bash scripts/check.sh
```

## Core Command

```bash
target/release/clawgs extract --tool auto --cwd "$PWD"
```

## Emit Daemon (stdio)

```bash
target/release/clawgs emit --stdio
```

Send one JSON `sync` message per line on stdin, read `sync_result` lines from stdout.
Protocol details are in [references/emit-protocol-v2.md](references/emit-protocol-v2.md).

## Tmux Emit

Poll every live tmux pane it can find, build session snapshots automatically, and emit the same NDJSON envelope used by the stdio daemon:

```bash
target/release/clawgs tmux-emit --once
target/release/clawgs tmux-emit
bash scripts/smoke_tmux_live.sh
```

Recommended shape: keep `tmux-emit` running as a daemon, let tmux hooks send immediate notify events, and keep the built-in interval as a slower reconciliation scan.

```bash
target/release/clawgs tmux-emit --interval-ms 60000
target/release/clawgs tmux-notify --event session-created
```

Recommended shared setup: keep the tmux hook block in this repo and source it from personal tmux configs instead of copying or symlinking an entire `~/.tmux.conf`.

```tmux
source-file "/path/to/your/clawgs/references/tmux-clawgs.conf"
```

The checked-in snippet is [references/tmux-clawgs.conf](references/tmux-clawgs.conf). It starts one background `clawgs tmux-emit` daemon, wires tmux lifecycle hooks to `clawgs tmux-notify`, and uses these defaults:

- socket: `$HOME/.tmux/clawgs-tmux.sock`
- output log: `$HOME/.tmux/clawgs-tmux.ndjson`
- error log: `$HOME/.tmux/clawgs-tmux.err`
- reconcile interval: `60000` ms

Output shape:

- first line: `hello`
- each poll after that: `sync_result`
- `metrics.sessions_seen`: number of live tmux panes scanned
- `updates[]`: only panes that produced a new thought on that poll

`tmux-emit` uses pane metadata + captured pane text as terminal context. If the pane command looks like Claude or Codex, it also tries transcript discovery by `cwd`.
The live smoke script is opt-in and safe to run locally: it skips without
`tmux`, otherwise uses an isolated tmux server/session/socket and disables
thought generation while verifying `hello` and `sync_result` output.

## Claude Code Hooks

Install `claude-hook-notify` in Claude Code hooks when you want Claude lifecycle
events to wake a running `tmux-emit` daemon. The command reads hook JSON on
stdin, sends a compact event tag to the tmux notify socket, and exits
successfully when the daemon is absent so it does not block Claude.

```bash
target/release/clawgs claude-hook-notify
target/release/clawgs claude-hook-notify --socket "$HOME/.tmux/clawgs-tmux.sock"
```

Use [references/claude-code-hooks.json](references/claude-code-hooks.json) as a
copyable settings snippet.

## Defaults

Print resolved daemon defaults (model, backend, prompts) as JSON:

```bash
target/release/clawgs defaults
target/release/clawgs defaults --pretty
```

## Library Usage (Rust Dependency)

Add `clawgs = "0.3"` to `Cargo.toml` and call `extract_jsonl_str` directly:

```rust
use clawgs::{AgentTool, ExtractOptions, extract_jsonl_str};

let output = extract_jsonl_str(
    AgentTool::Codex, "inline", jsonl_str,
    std::path::Path::new(cwd), false, &ExtractOptions::default(),
).unwrap();
```

Core output types (`ExtractOutput`, `Snapshot`, `Action`, `Source`, `Stats`,
`CommitSignal`) derive both `Serialize` and `Deserialize`, so saved `clawgs.v2`
JSON can be parsed back into typed Rust structs.

## Useful Flags

- `--input <path>`: parse a specific JSONL file instead of discovery
- `--tool <claude|codex|auto>`: force or infer source format
- `--pretty`: pretty-print JSON output (works on `extract`, `demo extract`, `demo emit`, `defaults`)
- `--include-raw`: include raw parsed event excerpts for debugging
- `--max-actions`, `--max-task-chars`, `--max-detail-chars`: output size controls

Discovery is cwd-exact and local: Claude scans the cwd slug under
`$HOME/.claude/projects` and accepts exact `cwd` matches or legacy files with no
cwd evidence; Codex scans numeric `$HOME/.codex/sessions/YYYY/MM/DD` folders and
requires first-line `session_meta.payload.cwd` to match exactly. Use `--input`
when `HOME` is unset, logs moved, or the desired transcript is stale.

## Output Contract

Schema version is `clawgs.v2`. Full field definitions and sample output are in [references/schema-v2.md](references/schema-v2.md).

## Release Metadata

Current crate release target: `0.3.0`.

- `0.2.0` is the canonical published `clawgs.v2` / `clawgs.emit.v2` schema and protocol release.
- `0.3.0` is the current unreleased target: Grok live backend, Claude Code hook
  wake-up bridge, `session_deltas`, transcript-discovery hardening, opt-in live
  tmux smoke, CI workflow, `Deserialize` on core output types, library API docs,
  `defaults --pretty`, and 77 dependency updates.
- Release history is tracked in [CHANGELOG.md](CHANGELOG.md).
- Tag and crates.io publish steps are tracked in [RELEASE.md](RELEASE.md).
