#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(git -C "$script_dir/.." rev-parse --show-toplevel 2>/dev/null || (cd "$script_dir/.." && pwd -P))"
binary_override=""

usage() {
  printf 'Usage: %s [--binary PATH]\n' "$0"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      binary_override="${2:?--binary requires a value}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

tmux_bin="${TMUX_BIN:-$(command -v tmux || true)}"
if [[ -z "$tmux_bin" ]]; then
  printf 'SKIP: tmux not found; live tmux smoke skipped\n'
  exit 0
fi

if [[ -n "$binary_override" ]]; then
  binary="$binary_override"
elif [[ -n "${CLAWGS_BINARY:-}" ]]; then
  binary="$CLAWGS_BINARY"
elif [[ -x "$repo_root/target/release/clawgs" ]]; then
  binary="$repo_root/target/release/clawgs"
elif [[ -x "$repo_root/target/debug/clawgs" ]]; then
  binary="$repo_root/target/debug/clawgs"
else
  cargo build --locked --manifest-path "$repo_root/Cargo.toml"
  binary="$repo_root/target/debug/clawgs"
fi

if [[ ! -x "$binary" ]]; then
  printf 'error: clawgs binary not found or not executable: %s\n' "$binary" >&2
  exit 1
fi

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/clawgs-tmux-smoke.XXXXXX")"
server_label="clawgs-smoke-$$"
session_name="clawgs-smoke"
socket_path="$tmp_dir/notify.sock"
stderr_file="$tmp_dir/tmux-emit.stderr"
tmux_wrapper="$tmp_dir/tmux"

cleanup() {
  "$tmux_bin" -L "$server_label" kill-server >/dev/null 2>&1 || true
  rm -rf "$tmp_dir"
}
trap cleanup EXIT INT TERM

cat >"$tmux_wrapper" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
exec "$REAL_TMUX_BIN" -L "$CLAWGS_SMOKE_TMUX_LABEL" "$@"
WRAPPER
chmod +x "$tmux_wrapper"

"$tmux_bin" -L "$server_label" new-session -d -s "$session_name" -x 120 -y 30
"$tmux_bin" -L "$server_label" send-keys -t "$session_name" \
  'printf "clawgs live tmux smoke ready\n"; sleep 60' Enter

sleep 0.3

if ! output="$(
  REAL_TMUX_BIN="$tmux_bin" \
  CLAWGS_SMOKE_TMUX_LABEL="$server_label" \
  CLAWGS_TMUX_BIN="$tmux_wrapper" \
  "$binary" tmux-emit \
    --once \
    --socket "$socket_path" \
    --max-capture-lines 80 \
    --config-json '{"enabled":false}' \
    2>"$stderr_file"
)"; then
  printf 'error: tmux-emit smoke failed\n' >&2
  cat "$stderr_file" >&2
  exit 1
fi

hello_line="$(printf '%s\n' "$output" | sed -n '1p')"
result_line="$(printf '%s\n' "$output" | sed -n '2p')"

if ! printf '%s' "$hello_line" | grep -q '"type":"hello"'; then
  printf 'error: expected first output line to be hello, got: %s\n' "$hello_line" >&2
  exit 1
fi

if ! printf '%s' "$result_line" | grep -q '"type":"sync_result"'; then
  printf 'error: expected second output line to be sync_result, got: %s\n' "$result_line" >&2
  exit 1
fi

if ! printf '%s' "$result_line" | grep -Eq '"sessions_seen":[[:space:]]*[1-9][0-9]*'; then
  printf 'error: expected sync_result metrics.sessions_seen >= 1, got: %s\n' "$result_line" >&2
  exit 1
fi

printf 'clawgs live tmux smoke passed: binary=%s server=%s\n' "$binary" "$server_label"
