#!/usr/bin/env bash
set -euo pipefail

# Deterministic fake-tmux scan scenario for profiling `clawgs tmux-emit --once`
# without depending on a live tmux server or private pane contents.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd))"

BINARY="${CLAWGS_BINARY:-${CLAWGS_PERF_BIN:-$REPO_ROOT/target/release-perf/clawgs}}"
PANE_COUNT="${TMUX_FAKE_PANES:-64}"
CAPTURE_LINES="${TMUX_FAKE_CAPTURE_LINES:-120}"
MAX_CAPTURE_LINES="${TMUX_FAKE_MAX_CAPTURE_LINES:-200}"

die() {
  printf 'error: %s\n' "$1" >&2
  exit 2
}

require_positive_int() {
  local name="$1"
  local value="$2"
  if ! [[ "$value" =~ ^[1-9][0-9]*$ ]]; then
    die "$name must be a positive integer"
  fi
}

require_positive_int "TMUX_FAKE_PANES" "$PANE_COUNT"
require_positive_int "TMUX_FAKE_CAPTURE_LINES" "$CAPTURE_LINES"
require_positive_int "TMUX_FAKE_MAX_CAPTURE_LINES" "$MAX_CAPTURE_LINES"

if [[ ! -x "$BINARY" ]]; then
  printf 'error: binary not found or not executable: %s\n' "$BINARY" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/clawgs-fake-tmux.XXXXXX")"
FAKE_TMUX="$TMP_DIR/fake-tmux"
SOCKET_PATH="$TMP_DIR/tmux.sock"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

cat > "$FAKE_TMUX" <<'FAKE_TMUX'
#!/usr/bin/env bash
set -euo pipefail

pane_count="${TMUX_FAKE_PANES:-64}"
capture_lines="${TMUX_FAKE_CAPTURE_LINES:-120}"
sep="$(printf '\037')"

emit_capture() {
  local pane_id="$1"
  local pane_num="${pane_id#%}"
  local line
  for ((line = 1; line <= capture_lines; line++)); do
    printf 'pane=%s line=%03d cwd=/tmp/clawgs-perf/project-%03d command=deterministic-status\n' \
      "$pane_id" "$line" "$pane_num"
  done
}

if [[ "${1:-}" == "list-panes" ]]; then
  for ((i = 1; i <= pane_count; i++)); do
    if ((i % 3 == 0)); then
      command="claude"
    elif ((i % 5 == 0)); then
      command="codex"
    else
      command="zsh"
    fi
    printf 'perf%s%d%s%d%s%%%d%s/tmp/clawgs-perf/project-%03d%s%s%s0\n' \
      "$sep" "$(((i - 1) / 4))" \
      "$sep" "$(((i - 1) % 4))" \
      "$sep" "$i" \
      "$sep" "$i" \
      "$sep" "$command" \
      "$sep"
  done
  exit 0
fi

while [[ $# -gt 0 ]]; do
  token="$1"
  shift || true
  case "$token" in
    display-message)
      marker=""
      while [[ $# -gt 0 && "$1" != ";" && "$1" != "display-message" && "$1" != "capture-pane" ]]; do
        if [[ "$1" == "-p" ]]; then
          shift || true
          marker="${1:-}"
        fi
        shift || true
      done
      printf '%s\n' "$marker"
      ;;
    capture-pane)
      target=""
      while [[ $# -gt 0 && "$1" != ";" && "$1" != "display-message" && "$1" != "capture-pane" ]]; do
        if [[ "$1" == "-t" ]]; then
          shift || true
          target="${1:-}"
        fi
        shift || true
      done
      if [[ -n "$target" ]]; then
        emit_capture "$target"
      fi
      ;;
    ";")
      ;;
  esac
done
FAKE_TMUX
chmod +x "$FAKE_TMUX"

OPENROUTER_API_KEY="${OPENROUTER_API_KEY:-fake}" \
CLAWGS_TMUX_BIN="$FAKE_TMUX" \
  "$BINARY" tmux-emit \
    --once \
    --socket "$SOCKET_PATH" \
    --max-capture-lines "$MAX_CAPTURE_LINES" \
    --config-json '{"enabled":false}' \
    > /dev/null

printf 'fake tmux scenario complete: panes=%s capture_lines=%s max_capture_lines=%s\n' \
  "$PANE_COUNT" "$CAPTURE_LINES" "$MAX_CAPTURE_LINES" >&2
