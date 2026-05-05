# Release Process

This document is the manual release contract for `clawgs`. The GitHub Actions
workflow in `.github/workflows/release.yml` automates the same verification and
publishing path when a `v*.*.*` tag is pushed.

## Current Release Target

- Crate: `clawgs`
- Version: `0.2.0`
- Canonical release claim: `0.2.0` is the v2 schema/protocol release for
  `clawgs.v2` and `clawgs.emit.v2`.
- Published channel: crates.io only.
- Required secret for automated publish: `CARGO_REGISTRY_TOKEN`.

## Pre-Tag Checklist

Run these from the repo root before creating the tag:

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
bash scripts/check.sh
cargo package --locked --list
cargo publish --locked --dry-run
```

For package install proof from the local checkout, run:

```bash
tmpdir="$(mktemp -d)"
CARGO_INSTALL_ROOT="$tmpdir/install" cargo install --path . --locked
"$tmpdir/install/bin/clawgs" demo emit --pretty
rm -rf "$tmpdir"
```

## Tag And Publish

1. Confirm `Cargo.toml` contains the intended version.
2. Confirm `CHANGELOG.md` has the release entry.
3. Commit the release-prep changes.
4. Create an annotated tag:

```bash
git tag -a v0.2.0 -m "clawgs v0.2.0 - schema v2 protocol release"
```

5. Push the commit and tag:

```bash
git push origin main
git push origin v0.2.0
```

6. The release workflow verifies formatting, linting, tests, package contents,
   and a dry-run publish before running `cargo publish`.

Manual fallback if GitHub Actions is unavailable:

```bash
cargo publish --locked
```

## Post-Publish Checks

After publish completes:

```bash
cargo install clawgs --version 0.2.0 --locked
clawgs demo extract --tool codex --pretty
clawgs demo emit --pretty
```

Then create the GitHub Release notes from `CHANGELOG.md` and run a final
reality check against `README.md`, `SKILL.md`, `references/schema-v2.md`, and
`references/emit-protocol-v2.md`.
