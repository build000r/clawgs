# Changelog Research

Scope: `v0.1.0..HEAD`, prepared for the `v0.2.0` release.

Research date: 2026-05-05.

## Evidence Sources

- Git tags: `git for-each-ref refs/tags --sort=creatordate --format='%(refname:short)%09%(creatordate:iso8601)%09%(subject)'`
- GitHub releases: `gh release list --limit 20 --json tagName,name,isDraft,isPrerelease,isLatest,publishedAt,createdAt`
- Commit spine: `git log --reverse --format='%H%x09%ad%x09%s' --date=short v0.1.0..HEAD`
- Diff scope: `git diff --stat v0.1.0..HEAD` and `git diff --name-only v0.1.0..HEAD`
- Docs and release metadata: `AGENTS.md`, `README.md`, `SKILL.md`, `Cargo.toml`, `references/*.md`, `references/*.schema.json`

## Version Spine

| Version | Evidence | Status |
| --- | --- | --- |
| `v0.1.0` | Git tag dated 2026-04-06: `clawgs v0.1.0 - first crates.io release` | Tag only; no GitHub Release object found |
| `v0.2.0` | `Cargo.toml` version is `0.2.0`; `HEAD` is `4b2d39a feat(emit): add schema v2 protocol support` | Prepared in code; not tagged and not published in this checkout |

`gh release list` returned `[]`, so this repo currently has git tags but no GitHub Releases.

## Coverage Ledger

| Chunk | Range | Status | Major Themes |
| --- | --- | --- | --- |
| C1 | `8b76c71..cab500a` | distilled | Release profile and emit hot-path allocation reduction |
| C2 | `d764d44..45d51cd` | distilled | Performance harness, tmux scan state, awaiting-user propagation, batched tmux capture, branch tests |
| C3 | `ec79faf..fb3aefd` | distilled | Parser and emit correctness fixes: i18n, prompt whitespace, temp files, nonce markers, markup-prefixed user replies |
| C4 | `c72981d..5098548` | distilled | Codex task events and model-backed thought summaries/backends |
| C5 | `85fe288..ef8dd20` | distilled | CRAP-driven refactors and targeted tests for model client, engine, protocol, library helpers |
| C6 | `012ea6b..4b2d39a` | distilled | JSON Schema publishing, `clawgs.v2` extract contract, `clawgs.emit.v2` protocol, version metadata |

## Representative Commits

- [`8b76c71`](https://github.com/build000r/clawgs/commit/8b76c71c3f76ee8cdfc69f425353f23376972a39) tuned release profile for long-lived emit daemon use.
- [`42a45eb`](https://github.com/build000r/clawgs/commit/42a45ebbb84cd5f7e2f66f3428a1086b1b48063e) batched `tmux capture-pane` into one invocation and added after-baseline artifacts.
- [`a55e4fe`](https://github.com/build000r/clawgs/commit/a55e4fe8fda3f8e6bab268a5f5dd88228d7ca193) added model-backed thought summaries.
- [`012ea6b`](https://github.com/build000r/clawgs/commit/012ea6ba4de63197dcab463498b8900ff783e347) published machine-readable JSON Schemas and schema sync tests.
- [`4b2d39a`](https://github.com/build000r/clawgs/commit/4b2d39ace1d9d97c992bd98bedf570fd6ee73806) added schema v2 protocol support and updated crate metadata to `0.2.0`.

## Tracker And Release Notes

- No `.beads` directory is present in this checkout, so there is no bead history to reconcile.
- No `.github/` directory existed before this release-readiness pass.
- No `CHANGELOG.md` existed before this release-readiness pass.
