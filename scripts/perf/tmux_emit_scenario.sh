#!/usr/bin/env bash
set -euo pipefail

# Scenario A: tmux-emit steady-state profiling driver.
#
# Creates a throwaway tmux session with deterministic content, runs
# clawgs tmux-emit against it, and tears down on exit.
#
# Modes:
#   (default)           Single scan via --once. Suitable for hyperfine.
#   --duration SECS     Loop at --interval-ms 500 for SECS seconds.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd))"
BINARY="${CLAWGS_BINARY:-$REPO_ROOT/target/release-perf/clawgs}"
SESSION_NAME="clawgs-perf-scenario-a-$$"
DURATION=0
INTERVAL_MS=500
MAX_CAPTURE_LINES=200

cleanup() {
  tmux kill-session -t "$SESSION_NAME" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

while [[ $# -gt 0 ]]; do
  case "$1" in
    --duration)
      DURATION="${2:?--duration requires a value}"
      shift 2
      ;;
    --interval-ms)
      INTERVAL_MS="${2:?--interval-ms requires a value}"
      shift 2
      ;;
    --binary)
      BINARY="${2:?--binary requires a value}"
      shift 2
      ;;
    -h|--help)
      printf 'Usage: %s [--duration SECS] [--interval-ms MS] [--binary PATH]\n' "$0"
      exit 0
      ;;
    *)
      printf 'Unknown option: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done

if [[ ! -x "$BINARY" ]]; then
  printf 'error: binary not found or not executable: %s\n' "$BINARY" >&2
  exit 1
fi

# Create a throwaway tmux session with deterministic content.
tmux new-session -d -s "$SESSION_NAME" -x 120 -y 40
tmux send-keys -t "$SESSION_NAME" \
  'echo "=== clawgs perf scenario A seed ===" && printf "Line %03d: The quick brown fox jumps over the lazy dog.\n" $(seq 1 80) && echo "=== seed complete ==="' Enter

# Give tmux a moment to render the pane content.
sleep 0.3

if [[ "$DURATION" -eq 0 ]]; then
  # Single-scan mode for hyperfine baselines.
  OPENROUTER_API_KEY="${OPENROUTER_API_KEY:-fake}" \
    "$BINARY" tmux-emit --once --max-capture-lines "$MAX_CAPTURE_LINES" >/dev/null 2>&1
else
  # Steady-state loop mode for profiling.
  OPENROUTER_API_KEY="${OPENROUTER_API_KEY:-fake}" \
    "$BINARY" tmux-emit \
      --interval-ms "$INTERVAL_MS" \
      --max-capture-lines "$MAX_CAPTURE_LINES" >/dev/null 2>&1 &
  CHILD_PID=$!

  sleep "$DURATION"
  kill "$CHILD_PID" 2>/dev/null || true
  wait "$CHILD_PID" 2>/dev/null || true
fi

printf 'scenario A run complete\n'
