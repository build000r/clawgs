#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/perf/bench_baseline.sh --name NAME [--runs N] [--warmup N] [--out DIR] -- COMMAND [COMMAND...]

Runs one or more benchmark commands through hyperfine with clawgs profiling defaults.

Options:
  --name NAME    Scenario tag used for default artifact paths. Required.
  --runs N       Number of measured runs. Default: 20.
  --warmup N     Number of warmup runs. Default: 3.
  --out DIR      Output directory. Default: tests/artifacts/perf/$NAME.
  -h, --help     Print this help text and exit.

Examples:
  scripts/perf/bench_baseline.sh --name extract -- \
    'target/release-perf/clawgs extract examples/demo/session.jsonl'

Build the binary before benchmarking:
  RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile release-perf --bin clawgs
USAGE
}

die() {
  printf 'error: %s\n\n' "$1" >&2
  usage >&2
  exit 2
}

require_value() {
  local option="$1"
  local value="${2:-}"
  if [[ -z "$value" || "$value" == --* ]]; then
    die "$option requires a value"
  fi
}

require_positive_int() {
  local option="$1"
  local value="$2"
  if ! [[ "$value" =~ ^[1-9][0-9]*$ ]]; then
    die "$option must be a positive integer"
  fi
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "$script_dir/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$script_dir/../.." && pwd))"

runs=20
warmup=3
name=""
out_dir=""
commands=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --runs)
      require_value "$1" "${2:-}"
      runs="$2"
      shift 2
      ;;
    --warmup)
      require_value "$1" "${2:-}"
      warmup="$2"
      shift 2
      ;;
    --name)
      require_value "$1" "${2:-}"
      name="$2"
      shift 2
      ;;
    --out)
      require_value "$1" "${2:-}"
      out_dir="$2"
      shift 2
      ;;
    --)
      shift
      while [[ $# -gt 0 ]]; do
        commands+=("$1")
        shift
      done
      ;;
    --*)
      die "unknown option: $1"
      ;;
    *)
      while [[ $# -gt 0 ]]; do
        commands+=("$1")
        shift
      done
      ;;
  esac
done

require_positive_int "--runs" "$runs"
require_positive_int "--warmup" "$warmup"

if [[ -z "$name" ]]; then
  die "--name is required"
fi

if [[ "${#commands[@]}" -eq 0 ]]; then
  die "at least one benchmark command is required"
fi

if ! command -v hyperfine >/dev/null 2>&1; then
  printf 'error: hyperfine is not installed.\n' >&2
  printf 'install hint for macOS: brew install hyperfine\n' >&2
  exit 1
fi

if [[ -z "$out_dir" ]]; then
  out_dir="$repo_root/tests/artifacts/perf/$name"
elif [[ "$out_dir" != /* ]]; then
  out_dir="$repo_root/$out_dir"
fi

mkdir -p "$out_dir"

if [[ -n "${RUSTFLAGS:-}" ]]; then
  export RUSTFLAGS="${RUSTFLAGS} -C force-frame-pointers=yes"
else
  export RUSTFLAGS="-C force-frame-pointers=yes"
fi

for command_string in "${commands[@]}"; do
  if [[ "$command_string" == *"cargo run"* ]]; then
    printf 'warning: command contains `cargo run`; pre-build with `RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile release-perf --bin clawgs` and benchmark target/release-perf/clawgs instead.\n' >&2
  fi
done

timestamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
git_head="$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || printf 'unknown')"
fingerprint_path="$repo_root/tests/artifacts/perf/fingerprint.json"

hyperfine \
  --warmup "$warmup" \
  --runs "$runs" \
  --export-json "$out_dir/baseline.json" \
  --export-markdown "$out_dir/baseline.md" \
  "${commands[@]}"

{
  printf '[%s] baseline run\n' "$timestamp"
  printf 'git: %s\n' "$git_head"
  printf 'profile: release-perf\n'
  printf 'fingerprint: %s\n' "$fingerprint_path"
  printf 'outputs: %s/baseline.json, %s/baseline.md\n' "$out_dir" "$out_dir"
  printf 'commands:\n'
  for command_string in "${commands[@]}"; do
    printf '  - %s\n' "$command_string"
  done
  printf '\n'
} >> "$out_dir/run.log"
