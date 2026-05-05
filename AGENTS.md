# AGENTS.md

## Project Shape
- `clawgs` is a Rust 2021 CLI/library crate (`rust-version = 1.85`) for normalizing Claude Code and Codex JSONL transcripts into `clawgs.v2` snapshots and emitting live `clawgs.emit.v2` NDJSON status updates.
- CLI entry point: `src/main.rs`, with subcommands `demo`, `extract`, `emit --stdio`, `tmux-emit`, `tmux-notify`, and `defaults`.
- Library entry point: `src/lib.rs`, exporting transcript discovery, extraction, parsers, emit protocol/engine, and tmux scanning.
- Public contracts are documented in `references/schema-v2.md` and `references/emit-protocol-v2.md`; keep schema/protocol changes deliberate and tested.

## Commands
- Install/build release binary: `bash scripts/install.sh` or `cargo build --release`.
- Smoke check installed release binary: `bash scripts/check.sh`.
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

## Data, Config, State
- Transcript discovery reads `$HOME/.claude/projects/.../*.jsonl` and `$HOME/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
- Runtime/backend env vars documented in README include `CLAWGS_MODEL_BACKEND`, `OPENROUTER_API_KEY`, `SWIMMERS_THOUGHT_MODEL*`, `CLAWGS_CODEX_*`, `CLAWGS_CLAUDE_*`, `CLAWGS_TMUX_BIN`, and `CLAWGS_TMUX_SOCKET`.
- Generated/local state ignored by git includes `target/`, `lcov.info`, `.ubs/`, `.mutate/`, `mutants.out/`, `.ntm/`, `.claude/`, and `.codex/`.

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
- No CI config was found in `.github/` or other usual top-level locations; verify locally before handing off.
