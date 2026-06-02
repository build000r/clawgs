#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "$script_dir/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$script_dir/../.." && pwd))"

BINARY="${CLAWGS_PERF_BIN:-$repo_root/target/release-perf/clawgs}"
OFFLINE_FIXTURE="$script_dir/fixtures/emit_stdio_offline.ndjson"
MODEL_ENABLED_FIXTURE="$script_dir/fixtures/emit_stdio.ndjson"
MODEL_ENABLED=0

usage() {
  cat >&2 <<'USAGE'
usage: scripts/perf/emit_stdio_scenario.sh [--model-enabled]

Runs the emit --stdio perf scenario offline by default. Pass
--model-enabled to replay the model-enabled fixture for explicit backend tests.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --model-enabled)
      MODEL_ENABLED=1
      shift
      ;;
    --offline)
      MODEL_ENABLED=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'error: unknown argument: %s\n' "$1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ "${CLAWGS_PERF_EMIT_STDIO_MODEL_ENABLED:-}" == "1" || "${CLAWGS_PERF_EMIT_STDIO_MODEL_ENABLED:-}" == "true" ]]; then
  MODEL_ENABLED=1
fi

if [[ "$MODEL_ENABLED" == "1" ]]; then
  FIXTURE="$MODEL_ENABLED_FIXTURE"
  MODE_LABEL="model-enabled"
else
  FIXTURE="$OFFLINE_FIXTURE"
  MODE_LABEL="offline"
fi

if [[ ! -x "$BINARY" ]]; then
  printf 'error: binary not found at %s\n' "$BINARY" >&2
  exit 1
fi

if [[ ! -f "$FIXTURE" ]]; then
  printf 'error: fixture not found at %s\n' "$FIXTURE" >&2
  exit 1
fi

"$BINARY" emit --stdio < "$FIXTURE" > /dev/null 2>&1

printf 'scenario B %s run complete\n' "$MODE_LABEL" >&2
