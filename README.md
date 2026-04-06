# clawgs

<div align="center">

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/build000r/clawgs/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)
[![Protocol](https://img.shields.io/badge/protocol-clawgs.emit.v1-blue.svg)](https://github.com/build000r/clawgs/blob/main/references/emit-protocol-v1.md)

</div>

Turn Claude Code and Codex transcripts into stable JSON snapshots, then replay the status protocol locally with a built-in zero-config demo.

<div align="center">

**Quick Start**

```bash
cargo install clawgs
clawgs demo extract --tool codex --pretty
```

</div>

## TL;DR

### The Problem

Agent transcripts are useful, but they are verbose, tool-specific, and usually trapped inside private JSONL logs, tmux panes, or one-off shell glue. That makes them awkward to inspect manually and annoying to integrate into status views, dashboards, or downstream automations.

### The Solution

`clawgs` normalizes Claude Code and Codex session logs into a small, stable JSON contract, and it exposes the live thought-emission protocol over NDJSON. The new `demo` command makes the whole thing legible from a clean machine with embedded, sanitized examples in [examples/demo](https://github.com/build000r/clawgs/tree/main/examples/demo).

### Why Use `clawgs`?

| Feature | What It Does |
| --- | --- |
| `demo extract` | Replays a built-in transcript corpus and shows the exact normalized `clawgs.v1` output without needing private logs |
| `demo emit` | Shows a real `hello -> sync -> sync_result` exchange without model credentials or tmux |
| `extract` | Normalizes Claude/Codex JSONL into one compact machine-readable snapshot |
| `emit --stdio` | Speaks a small NDJSON protocol for downstream status reporters |
| `tmux-emit` | Scans live tmux panes, infers context, and emits only changed thoughts |

## Quick Example

Install from crates.io:

```bash
cargo install clawgs

# See the built-in Codex transcript -> snapshot pair
clawgs demo extract --tool codex --pretty

# See the built-in Claude transcript -> snapshot pair
clawgs demo extract --tool claude --pretty

# See the canonical emit protocol exchange, no backend creds required
clawgs demo emit --pretty

# Parse a real local transcript by discovery
clawgs extract --tool auto --cwd "$PWD"

# Run the live stdio daemon
clawgs emit --stdio
```

Or build from source:

```bash
git clone https://github.com/build000r/clawgs && cd clawgs
bash scripts/install.sh
bash scripts/check.sh
target/release/clawgs demo extract --tool codex --pretty
```

Protocol details live in [references/emit-protocol-v1.md](https://github.com/build000r/clawgs/blob/main/references/emit-protocol-v1.md). The extract schema lives in [references/schema-v1.md](https://github.com/build000r/clawgs/blob/main/references/schema-v1.md).

## Design Philosophy

### 1. Public Before Private

If a stranger cannot understand the project without your personal logs, the project is not really open source yet. `clawgs demo` exists to make the core value visible without your environment.

### 2. Stable Contracts Beat Ad Hoc Log Scraping

The point is not to expose raw transcripts. The point is to collapse them into a compact contract that downstream tools can depend on.

### 3. Honest Surface Area

`clawgs` is a parser, protocol, and tmux bridge. It is not pretending to be a general observability platform, a transcript database, or a hosted service.

### 4. Real Examples Over Abstract Claims

The checked-in corpus in [examples/demo](https://github.com/build000r/clawgs/tree/main/examples/demo) and the reference docs in [references](https://github.com/build000r/clawgs/tree/main/references) are part of the product surface, not afterthoughts.

## Comparison

| Approach | Zero-config demo | Stable schema | Live protocol | tmux bridge | Good fit |
| --- | --- | --- | --- | --- | --- |
| `clawgs` | Yes | Yes | Yes | Yes | You want snapshots plus status emission from agent sessions |
| Raw JSONL files | No | No | No | No | You only want archival logs and are fine hand-parsing them |
| Ad hoc `jq` / `rg` scripts | No | Partial | No | Partial | You need a one-off local script and do not care about reuse |
| Custom tmux hook glue | No | No | Partial | Yes | You only need pane polling and are willing to maintain bespoke scripts |

**When to use `clawgs`:**
- You need a consistent JSON snapshot from Claude/Codex logs.
- You want a replayable contract for demos, tests, or downstream tools.
- You want tmux polling and thought emission without rewriting the parsing logic.

**When `clawgs` is not the right tool:**
- You need full transcript storage, search, or analytics.
- You need a hosted backend or multi-user service.
- You want package-manager releases or turnkey installers today; this repo is source-first right now.

For the deeper thesis, see [docs/VISION.md](https://github.com/build000r/clawgs/blob/main/docs/VISION.md): mission, vision, values, competitive fit, and why this project intentionally stops short of becoming a dashboard, a platform, or a general-purpose agent framework.

## Installation

### From Source (Recommended Right Now)

```bash
bash scripts/install.sh
bash scripts/check.sh
```

That builds `target/release/clawgs` and verifies the binary with a smoke test.

### Local Cargo Install

```bash
cargo install --path .
```

### What Is Not Published Yet

There is no Homebrew formula, no npm package, no PyPI package, and no curl installer in this repo yet. The README only documents install paths the repo already supports today.

## Quick Start

1. Clone the repo and build the release binary.

```bash
bash scripts/install.sh
```

2. Prove the project works from a clean machine shape.

```bash
target/release/clawgs demo extract --tool codex --pretty
target/release/clawgs demo emit --pretty
```

3. Parse a real transcript file directly.

```bash
target/release/clawgs extract --tool codex --input tests/fixtures/codex-sample.jsonl --pretty
```

4. Let `clawgs` auto-discover the newest log for the current project.

```bash
target/release/clawgs extract --tool auto --cwd "$PWD"
```

5. If you want live pane updates, wire tmux to the checked-in hook snippet.

```tmux
source-file "/path/to/clawgs/references/tmux-clawgs.conf"
```

That snippet lives in [references/tmux-clawgs.conf](https://github.com/build000r/clawgs/blob/main/references/tmux-clawgs.conf).

## Commands

### `clawgs demo extract`

Shows the embedded corpus plus the extracted output it produces.

```bash
target/release/clawgs demo extract --tool codex --pretty
target/release/clawgs demo extract --tool claude --pretty
```

### `clawgs demo emit`

Shows a canonical `hello`, `sync`, and `sync_result` exchange with no backend setup.

```bash
target/release/clawgs demo emit --pretty
```

### `clawgs extract`

Parses a real JSONL transcript into one `clawgs.v1` document.

```bash
target/release/clawgs extract --tool auto --cwd "$PWD"
target/release/clawgs extract --tool codex --input tests/fixtures/codex-sample.jsonl --pretty
target/release/clawgs extract --tool claude --include-raw --input tests/fixtures/claude-sample.jsonl
```

### `clawgs emit --stdio`

Runs the live NDJSON daemon over stdin/stdout.

```bash
target/release/clawgs emit --stdio
```

Send one JSON `sync` message per line on stdin and read `sync_result` lines from stdout.

### `clawgs tmux-emit`

Scans live tmux panes, reconciles snapshots, and emits the same NDJSON envelope used by `emit --stdio`.

```bash
target/release/clawgs tmux-emit --once
target/release/clawgs tmux-emit --interval-ms 60000
```

### `clawgs tmux-notify`

Pokes the tmux daemon socket so a hook can trigger an immediate rescan.

```bash
target/release/clawgs tmux-notify --event session-created
```

### `clawgs defaults`

Prints resolved daemon defaults as JSON.

```bash
target/release/clawgs defaults
```

## Configuration

### Extract Tuning Flags

`extract` and `demo extract` share the same output-shaping flags:

```bash
target/release/clawgs demo extract --tool codex --max-actions 5 --max-task-chars 120 --max-detail-chars 60
```

### Thought Config JSON

`tmux-emit --config-json` accepts the same thought-config shape used by the stdio protocol:

```json
{
  "enabled": true,
  "model": "",
  "backend": "",
  "cadence_hot_ms": 15000,
  "cadence_warm_ms": 45000,
  "cadence_cold_ms": 120000,
  "agent_prompt": null,
  "terminal_prompt": null
}
```

### Environment Variables

| Variable | Purpose |
| --- | --- |
| `CLAWGS_MODEL_BACKEND` | Selects `openrouter`, `claude`, or `codex` for live emit calls |
| `OPENROUTER_API_KEY` | Enables the OpenRouter backend |
| `SWIMMERS_THOUGHT_MODEL`, `SWIMMERS_THOUGHT_MODEL_2`, `SWIMMERS_THOUGHT_MODEL_3` | Override live thought models in priority order |
| `CLAWGS_CODEX_BIN` | Override the `codex` binary path |
| `CLAWGS_CODEX_REASONING_EFFORT` | Override Codex CLI reasoning effort |
| `CLAWGS_CODEX_VERBOSITY` | Override Codex CLI verbosity |
| `CLAWGS_CODEX_WORKDIR` | Override the workdir used for Codex CLI calls |
| `CLAWGS_CLAUDE_BIN` | Override the `claude` binary path |
| `CLAWGS_CLAUDE_MAX_BUDGET` | Override the Claude CLI max budget |
| `CLAWGS_TMUX_BIN` | Override the `tmux` binary path |
| `CLAWGS_TMUX_SOCKET` | Override the tmux notify socket path |

The demo commands do not require any of the variables above.

## Architecture

```text
┌───────────────────────────────────────────────────────────────┐
│ Inputs                                                        │
│ - embedded demo corpus                                        │
│ - Claude/Codex JSONL logs                                     │
│ - live tmux panes                                             │
└───────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌───────────────────────────────────────────────────────────────┐
│ Normalization Layer                                            │
│ - Claude parser                                                │
│ - Codex parser                                                 │
│ - discovery / source resolution                               │
└───────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌───────────────────────────────────────────────────────────────┐
│ Stable Contracts                                               │
│ - `clawgs.v1` extract snapshot                                │
│ - `clawgs.emit.v1` hello/sync/sync_result protocol            │
└───────────────────────────────────────────────────────────────┘
                             │
            ┌────────────────┴────────────────┐
            ▼                                 ▼
┌──────────────────────────────┐ ┌──────────────────────────────┐
│ Human-facing demo surface    │ │ Live status surface          │
│ - `demo extract`             │ │ - `emit --stdio`             │
│ - `demo emit`                │ │ - `tmux-emit` / notify hooks │
└──────────────────────────────┘ └──────────────────────────────┘
```

## Troubleshooting

### `no Claude or Codex transcript JSONL found`

Auto-discovery only works if the expected session logs exist in your local tool directories.

```bash
target/release/clawgs demo extract --tool codex --pretty
target/release/clawgs extract --tool codex --input path/to/session.jsonl --pretty
```

### `emit requires --stdio`

`emit` is intentionally protocol-only.

```bash
target/release/clawgs emit --stdio
```

### `OPENROUTER_API_KEY not set`

That only affects live emit backends. The demo path does not need credentials.

```bash
target/release/clawgs demo emit --pretty
target/release/clawgs defaults
```

### `--max-actions must be greater than 0`

The extract output-shaping limits must stay positive.

```bash
target/release/clawgs demo extract --tool codex --max-actions 5
```

### `tmux list-panes failed`

Make sure tmux is installed and a server is running before using the live pane scanner.

```bash
tmux ls
target/release/clawgs tmux-emit --once
```

## Limitations

- `clawgs` only understands the Claude and Codex transcript shapes it has been taught so far.
- The built-in demo corpus is representative, not exhaustive.
- `tmux-emit` and `tmux-notify` are Unix/tmux-centric; they are not a cross-platform pane abstraction.
- There is no published release channel or package-manager distribution yet.
- Live thought emission can use external backends if you choose them; the repo does not pretend those calls are offline.

## FAQ

### Is `clawgs` a log viewer?

No. It is a normalizer and protocol layer. It turns verbose session state into smaller, more stable contracts.

### Does `clawgs demo` use my local transcripts?

No. It uses the checked-in sanitized corpus in [examples/demo](https://github.com/build000r/clawgs/tree/main/examples/demo).

### Does `demo emit` call OpenRouter, Claude, or Codex?

No. The demo replay is local and self-contained. Live `emit` behavior is where backend selection matters.

### Can I use this without tmux?

Yes. `extract`, `demo`, `emit --stdio`, and `defaults` do not require tmux.

### Can I inspect the exact schema and protocol?

Yes. See [references/schema-v1.md](https://github.com/build000r/clawgs/blob/main/references/schema-v1.md) and [references/emit-protocol-v1.md](https://github.com/build000r/clawgs/blob/main/references/emit-protocol-v1.md).

### Is the demo corpus the same thing as the tests?

They are related, but the public corpus in [examples/demo](https://github.com/build000r/clawgs/tree/main/examples/demo) exists for onboarding and documentation, not just for internal regression coverage.

## About Contributions

*About Contributions:* Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. I simply don't have the mental bandwidth to review anything, and it's my name on the thing, so I'm responsible for any problems it causes; thus, the risk-reward is highly asymmetric from my perspective. I'd also have to worry about other "stakeholders," which seems unwise for tools I mostly make for myself for free. Feel free to submit issues, and even PRs if you want to illustrate a proposed fix, but know I won't merge them directly. Instead, I'll have Claude or Codex review submissions via `gh` and independently decide whether and how to address them. Bug reports in particular are welcome. Sorry if this offends, but I want to avoid wasted time and hurt feelings. I understand this isn't in sync with the prevailing open-source ethos that seeks community contributions, but it's the only way I can move at this velocity and keep my sanity.

## License

MIT. See [LICENSE](https://github.com/build000r/clawgs/blob/main/LICENSE).
