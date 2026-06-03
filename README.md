# clawgs

<div align="center">

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/build000r/clawgs/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![crates.io](https://img.shields.io/crates/v/clawgs.svg)](https://crates.io/crates/clawgs)
[![Protocol](https://img.shields.io/badge/protocol-clawgs.emit.v2-blue.svg)](https://github.com/build000r/clawgs/blob/main/references/emit-protocol-v2.md)

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

`clawgs` normalizes Claude Code and Codex session logs into a small, stable JSON contract, including deterministic `action_cues` for transcript-backed attention facts, and it exposes the live thought-emission protocol over NDJSON. The new `demo` command makes the whole thing legible from a clean machine with embedded, sanitized examples in [examples/demo](https://github.com/build000r/clawgs/tree/main/examples/demo).

### Why Use `clawgs`?

| Feature | What It Does |
| --- | --- |
| `demo extract` | Replays a built-in transcript corpus and shows the exact normalized `clawgs.v2` output without needing private logs |
| `demo emit` | Shows a real `hello -> sync -> sync_result` exchange without model credentials or tmux |
| `extract` | Normalizes Claude/Codex JSONL into one compact machine-readable snapshot with parser-derived `action_cues` |
| `emit --stdio` | Speaks a small NDJSON protocol for downstream status reporters, including live `action_cues` |
| `tmux-emit` | Scans live tmux panes, infers context, and emits only changed thoughts |
| `claude-hook-notify` | Lets Claude Code hooks wake a running `tmux-emit` daemon without blocking Claude |

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

# Optional: let Claude Code hooks wake a tmux-emit daemon
clawgs claude-hook-notify < hook-event.json
```

Or build from source:

```bash
git clone https://github.com/build000r/clawgs && cd clawgs
# Requires Rust 1.85 or newer.
bash scripts/install.sh
bash scripts/check.sh
target/release/clawgs demo extract --tool codex --pretty
```

Protocol details live in [references/emit-protocol-v2.md](https://github.com/build000r/clawgs/blob/main/references/emit-protocol-v2.md), with a machine-validatable JSON Schema at [references/clawgs.emit.v2.schema.json](https://github.com/build000r/clawgs/blob/main/references/clawgs.emit.v2.schema.json). The extract schema lives in [references/schema-v2.md](https://github.com/build000r/clawgs/blob/main/references/schema-v2.md), with JSON Schema at [references/clawgs.v2.schema.json](https://github.com/build000r/clawgs/blob/main/references/clawgs.v2.schema.json).

### Transcript Discovery

`clawgs extract --tool auto --cwd "$PWD"` resolves checked-in schemas from local
tool logs only. Pass `--input` when you already know the transcript path or when
the transcript lives outside the default Claude/Codex locations.

Claude Code discovery looks under
`$HOME/.claude/projects/<cwd-with-slashes-replaced-by-dashes>/*.jsonl`, keeps
JSONL files whose first 64 parseable lines either contain an exact top-level
`cwd` match or no `cwd` field at all, and returns the newest remaining file,
with path order as the deterministic tiebreaker for identical mtimes. The
no-`cwd` fallback keeps older Claude logs discoverable; files with no parseable
JSONL lines are ignored.

Codex discovery scans numeric date folders under
`$HOME/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, reads only the first line of
each candidate, requires a `session_meta.payload.cwd` exact match, and returns
the newest matching rollout by reverse date/path order. Empty files, malformed
first lines, non-`session_meta` first lines, non-numeric date folders, and
non-`rollout-*.jsonl` files are ignored.

When multiple live sessions point at the same cwd, the tmux emitter remembers
claimed transcript paths and asks discovery for the next matching file instead
of reusing the same transcript for another pane. If `HOME` is unset or no
candidate matches, discovery returns the same "no transcript JSONL found" error
and the direct `--input` path remains the escape hatch.

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
- You need compact session boundary/change facts through `session_deltas`
  without scraping raw transcripts or tmux pane text.

**When `clawgs` is not the right tool:**
- You need full transcript storage, search, or analytics.
- You need a hosted backend or multi-user service.
- You want Homebrew, npm, PyPI, or a curl installer today; only `cargo install` is wired up so far.

For the deeper thesis, see [docs/VISION.md](https://github.com/build000r/clawgs/blob/main/docs/VISION.md): mission, vision, values, competitive fit, and why this project intentionally stops short of becoming a dashboard, a platform, or a general-purpose agent framework.

## Installation

### From crates.io

```bash
cargo install clawgs
```

That gets you a `clawgs` binary on your `PATH` with no repo checkout required.

The current published crate is `0.2.0`. On lean hosts without system OpenSSL and
`pkg-config`, `0.2.0` can fail to build because it links the default
`native-tls` backend. The fix (pure-Rust `rustls-tls`, no OpenSSL) ships in
`0.3.0`; once `0.3.0` is published, the portable one-liner that needs no system
OpenSSL is `cargo install clawgs --version 0.3.0 --locked`. A verified
clean-install transcript lives in
[docs/INSTALL.md](https://github.com/build000r/clawgs/blob/main/docs/INSTALL.md).

### From Source

```bash
git clone https://github.com/build000r/clawgs && cd clawgs
bash scripts/install.sh
bash scripts/check.sh
```

That builds `target/release/clawgs` and verifies the binary with a smoke test. You can also run `cargo install --path .` from inside a checkout.

`target/release/clawgs` is also the stable source-checkout path for downstream
tools. For example, Swimmers auto-discovers a sibling checkout at
`../clawgs/target/release/clawgs` before falling back to `clawgs` on `PATH`.
When using Swimmers and Clawgs side by side from source, run
`bash scripts/install.sh` after cloning or updating Clawgs. A debug-only
`cargo build` leaves `target/debug/clawgs`; use `CLAWGS_BIN=/path/to/clawgs`
only when you intentionally want a non-default binary.

### What Is Not Published Yet

There is no Homebrew formula, no npm package, no PyPI package, and no curl installer yet. crates.io is currently the only published distribution channel.

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

Parses a real JSONL transcript into one `clawgs.v2` document.

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

Opt-in live smoke proof:

```bash
bash scripts/smoke_tmux_live.sh
```

The smoke script skips clearly when `tmux` is unavailable. When `tmux` exists,
it creates a private tmux server/session and notify socket, runs `tmux-emit
--once` with thought generation disabled, verifies `hello` and `sync_result`
NDJSON output with at least one seen session, and cleans up. It is intentionally
not part of `cargo test`.

### `clawgs tmux-notify`

Pokes the tmux daemon socket so a hook can trigger an immediate rescan.

```bash
target/release/clawgs tmux-notify --event session-created
```

### `clawgs claude-hook-notify`

Reads Claude Code hook JSON from stdin, derives a compact event tag, and pokes
the same socket used by `tmux-notify`. It exits successfully when the daemon is
not running, so it is safe to install in Claude Code hooks without putting
Claude's tool loop on the critical path.

```bash
target/release/clawgs claude-hook-notify
target/release/clawgs claude-hook-notify --socket "$HOME/.tmux/clawgs-tmux.sock"
```

A copyable Claude Code hook settings snippet lives at
[references/claude-code-hooks.json](https://github.com/build000r/clawgs/blob/main/references/claude-code-hooks.json).

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
| `CLAWGS_MODEL_BACKEND` | Selects `openrouter` or `grok` for live emit calls; legacy `claude`/`codex` values route to Grok |
| `OPENROUTER_API_KEY` | Enables the OpenRouter backend |
| `SWIMMERS_THOUGHT_MODEL`, `SWIMMERS_THOUGHT_MODEL_2`, `SWIMMERS_THOUGHT_MODEL_3` | Override live thought models in priority order |
| `CLAWGS_GROK_BIN` | Override the `grok` binary path |
| `CLAWGS_GROK_WORKDIR` | Override the workdir passed to Grok headless mode |
| `CLAWGS_GROK_MAX_TURNS` | Override Grok `--max-turns` for unattended headless calls |
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
│ - `clawgs.v2` extract snapshot                                │
│ - `clawgs.emit.v2` hello/sync/sync_result protocol            │
│ - additive `session_deltas` boundary/change facts             │
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
Discovery requires `HOME`, exact cwd matches where the transcript format carries
cwd metadata, and a parseable candidate file. Use `--input` for stale, moved, or
non-standard transcript locations.

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
- crates.io is the only published distribution channel so far; Homebrew, npm, PyPI, and a curl installer are not wired up yet.
- Live thought emission can use external backends if you choose them; the repo does not pretend those calls are offline.

## Release Process

`clawgs` publishes through crates.io. Release history lives in
[CHANGELOG.md](CHANGELOG.md), and the tag/publish contract lives in
[RELEASE.md](RELEASE.md). The GitHub Actions release workflow verifies format,
clippy, tests, package contents, and `cargo publish --dry-run` before publishing
tagged `v*.*.*` releases with `CARGO_REGISTRY_TOKEN`.
The opt-in live tmux smoke is local-machine proof and is documented in the
manual release checklist rather than forced into CI.

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

Yes. See [references/schema-v2.md](https://github.com/build000r/clawgs/blob/main/references/schema-v2.md), [references/clawgs.v2.schema.json](https://github.com/build000r/clawgs/blob/main/references/clawgs.v2.schema.json), [references/emit-protocol-v2.md](https://github.com/build000r/clawgs/blob/main/references/emit-protocol-v2.md), and [references/clawgs.emit.v2.schema.json](https://github.com/build000r/clawgs/blob/main/references/clawgs.emit.v2.schema.json).

### Is the demo corpus the same thing as the tests?

They are related, but the public corpus in [examples/demo](https://github.com/build000r/clawgs/tree/main/examples/demo) exists for onboarding and documentation, not just for internal regression coverage.

## About Contributions

*About Contributions:* Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. I simply don't have the mental bandwidth to review anything, and it's my name on the thing, so I'm responsible for any problems it causes; thus, the risk-reward is highly asymmetric from my perspective. I'd also have to worry about other "stakeholders," which seems unwise for tools I mostly make for myself for free. Feel free to submit issues, and even PRs if you want to illustrate a proposed fix, but know I won't merge them directly. Instead, I'll have Claude or Codex review submissions via `gh` and independently decide whether and how to address them. Bug reports in particular are welcome. Sorry if this offends, but I want to avoid wasted time and hurt feelings. I understand this isn't in sync with the prevailing open-source ethos that seeks community contributions, but it's the only way I can move at this velocity and keep my sanity.

## License

MIT. See [LICENSE](https://github.com/build000r/clawgs/blob/main/LICENSE).
