# AGENTS.md

## Project Shape
- `clawgs` is a Rust 2021 CLI/library crate (`rust-version = 1.85`) for normalizing Claude Code and Codex JSONL transcripts into `clawgs.v2` snapshots and emitting live `clawgs.emit.v2` NDJSON status updates.
- CLI entry point: `src/main.rs`, with subcommands `demo`, `extract`, `emit --stdio`, `tmux-emit`, `tmux-notify`, and `defaults`.
- Library entry point: `src/lib.rs`, exporting transcript discovery, extraction, parsers, emit protocol/engine, and tmux scanning.
- Public contracts are documented in `references/schema-v2.md` and `references/emit-protocol-v2.md`; keep schema/protocol changes deliberate and tested.

## Commands
- Install/build release binary: `bash scripts/install.sh` or `cargo build --release`.
- Smoke check installed release binary: `bash scripts/check.sh` (validates
  `--help`, `defaults`, and fixture extraction).
- Build dev: `cargo build`.
- Test: `cargo test`.
- Format check: `cargo fmt -- --check`.
- Lint: `cargo clippy --all-targets -- -D warnings`.
- Coverage target for `/crap`: `make cargo-cov-lcov` (requires `cargo llvm-cov`, `llvm-cov`, and `llvm-profdata`).
- Run demos: `cargo run -- demo extract --tool codex --pretty` and `cargo run -- demo emit --pretty`.
- Parse fixture: `cargo run -- extract --tool codex --input tests/fixtures/codex-sample.jsonl --pretty`.
- Stdio daemon: `cargo run -- emit --stdio`.
- Tmux one-shot scan: `cargo run -- tmux-emit --once`.
- Release process: `RELEASE.md` documents the manual release contract, and `.github/workflows/release.yml` verifies tag builds before publishing to crates.io. Verify `CARGO_REGISTRY_TOKEN`, the crate version, and tag name before `cargo publish`.

## Layout
- `src/parsers/`: Claude/Codex JSONL parsing plus shared JSONL/truncation/action helpers.
- `src/emit/`: NDJSON protocol types, thought emission engine, and model backend clients.
- `src/tmux.rs`: tmux pane listing/capture and conversion into emit protocol `SessionSnapshot`s.
- `src/demo.rs`: embedded demo corpus from `examples/demo/`.
- `tests/`: integration tests and JSONL fixtures; `tests/artifacts/perf/` holds performance artifacts.
- `scripts/`: install/check scripts plus performance scenario runners.
- `references/tmux-clawgs.conf`: tmux hook snippet; it writes runtime logs under `$HOME/.tmux/`.
- Downstream source checkouts such as Swimmers discover the stable local binary
  at `target/release/clawgs`; run `bash scripts/install.sh` when that path is
  missing rather than relying on a debug-only `cargo build`.

## Data, Config, State
- Transcript discovery reads `$HOME/.claude/projects/.../*.jsonl` and `$HOME/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
- Runtime/backend env vars documented in README include `CLAWGS_MODEL_BACKEND`, `OPENROUTER_API_KEY`, `SWIMMERS_THOUGHT_MODEL*`, `CLAWGS_CODEX_*`, `CLAWGS_CLAUDE_*`, `CLAWGS_TMUX_BIN`, and `CLAWGS_TMUX_SOCKET`.
- Generated/local state ignored by git includes `target/`, `lcov.info`, `.ubs/`, `.mutate/`, `mutants.out/`, `.ntm/`, `.buildooor/`, `.claude/`, `.codex/`, `*.bak`, and `*.orig`.

## Testing Expectations
- Keep `cargo test`, `cargo fmt -- --check`, and `cargo clippy --all-targets -- -D warnings` green; all three passed against the current tree during AGENTS creation.
- Add or update tests when changing parser behavior, emit protocol serialization, model backend selection, tmux scanning, or CLI validation.
- Tests that mutate process env should use the existing mutex patterns (`home_env_lock` / `ENV_LOCK`) to avoid cross-test races.
- `tmux_emit` integration tests use a fake tmux script; live `tmux-emit --once` can depend on local tmux availability.
- Performance scripts under `scripts/perf/` are separate from the normal test suite and may require release-perf builds or platform tools.

## Coding Notes
- Prefer existing patterns: `anyhow::Result` with context at I/O boundaries, `serde` structs for wire contracts, `clap` derive for CLI, and small parser helpers over ad hoc string parsing.
- Preserve contract stability: `clawgs.v1`/`clawgs.v2` and `clawgs.emit.v1`/`clawgs.emit.v2` are downstream-facing APIs.
- Parser code is intentionally tolerant of malformed JSONL lines; preserve `malformed_lines_skipped` behavior unless changing the schema intentionally.
- Keep demo paths zero-config: `demo extract` and `demo emit` must not require private logs, tmux, or model credentials.
- `tmux-emit` can call external model backends depending on config/env; demos should stay local-only.

## Safety / Gotchas
- Do not commit secrets or private transcript logs. Use checked-in sanitized fixtures in `examples/demo/` and `tests/fixtures/`.
- Be careful with `CLAWGS_TMUX_SOCKET`: `tmux-emit` removes an existing Unix socket before binding and refuses non-socket paths.
- `references/tmux-clawgs.conf` starts a background daemon and appends logs to `$HOME/.tmux/`.
- Do not delete or regenerate tracked performance artifacts unless the task explicitly asks for performance-baseline work.
- `.github/workflows/release.yml` is present and verifies tag builds before crates.io publishing; still run the repo-native checks locally before handing off.

<!-- br-agent-instructions-v1 -->

---

## Beads Workflow Integration

This project uses [beads_rust](https://github.com/Dicklesworthstone/beads_rust) (`br`/`bd`) for issue tracking. Issues are stored in `.beads/` and tracked in git.

### Essential Commands

```bash
# View ready issues (open, unblocked, not deferred)
br ready              # or: bd ready

# List and search
br list --status=open # All open issues
br show <id>          # Full issue details with dependencies
br search "keyword"   # Full-text search

# Create and update
br create --title="..." --description="..." --type=task --priority=2
br update <id> --status=in_progress
br close <id> --reason="Completed"
br close <id1> <id2>  # Close multiple issues at once

# Sync with git
br sync --flush-only  # Export DB to JSONL
br sync --status      # Check sync status
```

### Workflow Pattern

1. **Start**: Run `br ready` to find actionable work
2. **Claim**: Use `br update <id> --status=in_progress`
3. **Work**: Implement the task
4. **Complete**: Use `br close <id>`
5. **Sync**: Always run `br sync --flush-only` at session end

### Key Concepts

- **Dependencies**: Issues can block other issues. `br ready` shows only open, unblocked work.
- **Priority**: P0=critical, P1=high, P2=medium, P3=low, P4=backlog (use numbers 0-4, not words)
- **Types**: task, bug, feature, epic, chore, docs, question
- **Blocking**: `br dep add <issue> <depends-on>` to add dependencies

### Session Protocol

**Before ending any session, run this checklist:**

```bash
git status              # Check what changed
git add <files>         # Stage code changes
br sync --flush-only    # Export beads changes to JSONL
git commit -m "..."     # Commit everything
git push                # Push to remote
```

### Best Practices

- Check `br ready` at session start to find available work
- Update status as you work (in_progress → closed)
- Create new issues with `br create` when you discover tasks
- Use descriptive titles and set appropriate priority/type
- Always sync before ending session

<!-- end-br-agent-instructions -->
