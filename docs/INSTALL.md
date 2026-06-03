# Clean-Install Proof

This document is a checked-in, sanitized proof that `clawgs` installs from a
clean target and that the zero-config demos run without any private logs, model
credentials, or tmux. It backs the install claims in `README.md` and the release
contract in `RELEASE.md`.

All paths below come from the embedded `examples/demo/` corpus. No private
transcripts, secrets, or environment-specific logs are included.

## TL;DR

- `cargo install --path . --locked` (the current `0.3.0` tree) installs cleanly
  into a fresh root and the demos work end to end.
- `cargo install clawgs` (crates.io) currently fetches `0.2.0`, which fails to
  build on hosts without system OpenSSL + `pkg-config` because that published
  version used `reqwest` with the default `native-tls` backend. The fix
  (`reqwest` with `rustls-tls`, no OpenSSL) lands in `0.3.0` and is what makes a
  truly portable `cargo install clawgs` work. crates.io publication of `0.3.0`
  is the remaining step (see "crates.io status" below).

## crates.io status

| Item | State |
| --- | --- |
| Published on crates.io | Yes, but only `0.2.0` |
| Current repo version | `0.3.0` |
| `cargo install clawgs` on a host without OpenSSL | Fails on `0.2.0` (openssl-sys build error) |
| `cargo install --path . --locked` (this tree) | Succeeds (`rustls-tls`, no OpenSSL) |
| Blocker for portable `cargo install clawgs` | Publish `0.3.0` (uses `rustls-tls`) per `RELEASE.md` |

Post-publish, the canonical one-liner that WILL work on a clean host without
OpenSSL is:

```bash
cargo install clawgs --version 0.3.0 --locked
```

## Verified clean install (current `0.3.0` tree)

Install into a throwaway root so nothing on the host's `PATH` is touched:

```bash
CLEAN_ROOT="$(mktemp -d)"
cargo install --path . --locked --root "$CLEAN_ROOT"
"$CLEAN_ROOT/bin/clawgs" --version
```

Observed result:

```text
  Installing clawgs v0.3.0 (.../clawgs)
  Installing .../bin/clawgs
   Installed package `clawgs v0.3.0 (.../clawgs)` (executable `clawgs`)
clawgs 0.3.0
```

The release dependency tree resolves TLS through `rustls` only; `openssl`
appears zero times in `Cargo.lock`, so the build needs no system OpenSSL or
`pkg-config`.

## Why `cargo install clawgs` (crates.io `0.2.0`) currently fails on lean hosts

On a host without `pkg-config` and without OpenSSL development headers,
`cargo install clawgs` fetches `0.2.0` and fails while compiling `openssl-sys`:

```text
  Could not find directory of OpenSSL installation ...
  It looks like you're compiling on Linux ... requires the `pkg-config`
  utility to find OpenSSL ...
  openssl-sys = 0.9.111
error: failed to compile `clawgs v0.2.0`
```

This is the exact portability gap that `0.3.0` closes by switching `reqwest`
from the default `native-tls` backend to `rustls-tls`.

## Zero-config demos against the freshly installed binary

All three demo flows run from the clean-root binary with no flags, env vars,
credentials, or tmux. Output is sanitized by construction (embedded corpus).

### `clawgs demo extract --tool codex --pretty`

```json
{
  "demo": "extract",
  "tool": "codex",
  "input_path": "embedded:examples/demo/codex-sample.jsonl",
  "output": {
    "schema_version": "clawgs.v2",
    "source": { "tool": "codex", "discovered": false, "cwd": "/demo/codex-project" },
    "snapshot": {
      "user_task": "Build a parser",
      "current_tool": { "tool": "exec_command", "detail": "ls -la", "kind": "function_call" },
      "token_count": 1212,
      "recent_actions": [
        { "tool": "exec_command", "detail": "ls -la", "kind": "function_call" }
      ]
    },
    "stats": { "events_seen": 4, "malformed_lines_skipped": 0, "bytes_read": 322 }
  }
}
```

### `clawgs demo extract --tool claude --pretty`

```json
{
  "demo": "extract",
  "tool": "claude",
  "input_path": "embedded:examples/demo/claude-sample.jsonl",
  "output": {
    "schema_version": "clawgs.v2",
    "source": { "tool": "claude", "discovered": false, "cwd": "/demo/claude-project" },
    "snapshot": {
      "user_task": "Summarize logs",
      "current_tool": { "tool": "read_file", "detail": "demo.txt", "kind": "tool_use" },
      "token_count": 88,
      "recent_actions": [
        { "tool": "read_file", "detail": "demo.txt", "kind": "tool_use" },
        { "tool": "said", "detail": "I read the file", "kind": "text" }
      ]
    },
    "stats": { "events_seen": 2, "malformed_lines_skipped": 0, "bytes_read": 279 }
  }
}
```

### `clawgs demo emit --pretty`

Shows the canonical `hello -> sync -> sync_result` exchange with no backend
credentials and no tmux. Abridged:

```json
{
  "demo": "emit",
  "hello": { "type": "hello", "protocol": "clawgs.emit.v2", "engine_version": "0.3.0" },
  "request": { "type": "sync", "id": "demo-sync-1", "sessions": [ { "session_id": "demo:codex:1", "cwd": "/demo/project-a" } ] },
  "response": {
    "type": "sync_result",
    "id": "demo-sync-1",
    "updates": [
      { "session_id": "demo:codex:1", "thought": "Turning raw transcripts into stable session state", "thought_source": "llm" }
    ],
    "metrics": { "sessions_seen": 1, "llm_calls": 1, "suppressed": 0 }
  }
}
```

## Reproduce

From a clean checkout with Rust 1.85+:

```bash
# Validation
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
bash scripts/check.sh

# Clean-install proof
CLEAN_ROOT="$(mktemp -d)"
cargo install --path . --locked --root "$CLEAN_ROOT"
"$CLEAN_ROOT/bin/clawgs" demo extract --tool codex --pretty
"$CLEAN_ROOT/bin/clawgs" demo extract --tool claude --pretty
"$CLEAN_ROOT/bin/clawgs" demo emit --pretty
rm -rf "$CLEAN_ROOT"
```
