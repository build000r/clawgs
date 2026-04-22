#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "$script_dir/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$script_dir/../.." && pwd))"

BINARY="${CLAWGS_PERF_BIN:-$repo_root/target/release-perf/clawgs}"
FIXTURE="$script_dir/fixtures/emit_stdio.ndjson"

if [[ ! -x "$BINARY" ]]; then
  printf 'error: binary not found at %s\n' "$BINARY" >&2
  exit 1
fi

if [[ ! -f "$FIXTURE" ]]; then
  printf 'error: fixture not found at %s\n' "$FIXTURE" >&2
  exit 1
fi

"$BINARY" emit --stdio < "$FIXTURE" > /dev/null 2>&1

printf 'scenario B run complete\n' >&2
