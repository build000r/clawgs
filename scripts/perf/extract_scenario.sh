#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/perf/extract_scenario.sh [--dry-run] [--synthesize-only]

Synthesizes deterministic large Claude/Codex JSONL fixtures, then runs
`clawgs extract` over both fixtures with stdout discarded.

Options:
  --dry-run          Print fixture metadata and commands without extracting.
  --synthesize-only  Regenerate fixtures, verify sha256 stability, then exit.
  -h, --help         Print this help text.

Environment:
  CLAWGS_BIN                  Binary to run. Default: target/release-perf/clawgs
  EXTRACT_SCENARIO_LOOPS      Number of extract passes over both fixtures. Default: 1
  EXTRACT_SCENARIO_TURNS      Replay turns per tool fixture. Default: 2000
USAGE
}

die() {
  printf 'error: %s\n' "$1" >&2
  exit 2
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

require_positive_int() {
  local name="$1"
  local value="$2"
  if ! [[ "$value" =~ ^[1-9][0-9]*$ ]]; then
    die "$name must be a positive integer"
  fi
}

line_count() {
  wc -l < "$1" | tr -d '[:space:]'
}

byte_count() {
  wc -c < "$1" | tr -d '[:space:]'
}

synthesize_tool_fixture() {
  local tool="$1"
  local sample="$2"
  local turns="$3"
  local output="$4"

  jq -c -n \
    --slurpfile records "$sample" \
    --arg tool "$tool" \
    --argjson turns "$turns" \
    '
    def lpad($width):
      tostring as $s
      | if ($s | length) >= $width then
          $s
        else
          ("000000000000" + $s)[-$width:]
        end;

    def hash_num($turn; $index; $salt):
      (($turn * 1103515245 + $index * 2654435761 + $salt) % 1000000000000) | floor;

    def timestamp_for_hash($hash):
      ($hash % 60 | floor) as $minute
      | (($hash / 60) % 60 | floor) as $second
      | (($hash / 3600) % 1000 | floor) as $millis
      | "2026-04-22T00:\($minute | lpad(2)):\($second | lpad(2)).\($millis | lpad(3))Z";

    def claude_record($id; $timestamp; $offset):
      .timestamp = $timestamp
      | .id = $id
      | if .type == "user" and (.message.content? | type) == "string" then
          .message.content = (.message.content + " [" + $id + "]")
        elif .type == "assistant" then
          .message.id = $id
          | .message.usage.input_tokens = ((.message.usage.input_tokens // 0) + $offset)
          | if (.message.content? | type) == "array" then
              .message.content |= map(
                if .type == "tool_use" then
                  .id = $id
                  | .input.file_path = ("/tmp/" + $id + ".txt")
                elif .type == "text" then
                  .text = ((.text // "") + " " + $id)
                else
                  .
                end
              )
            else
              .
            end
        else
          .
        end;

    def codex_record($id; $hash; $timestamp; $offset):
      .timestamp = $timestamp
      | .id = $id
      | if .type == "session_meta" then
          .payload.cwd = ("/tmp/project-" + $hash)
        elif .type == "event_msg" and .payload.type == "user_message" then
          .payload.message = (.payload.message + " [" + $id + "]")
        elif .type == "response" then
          .payload.usage.input_tokens = ((.payload.usage.input_tokens // 0) + $offset)
        elif .type == "response_item" and .payload.type == "function_call" then
          .payload.call_id = ("call_" + $hash)
          | .payload.arguments = ({command: ("ls -la /tmp/project-" + $hash), id: $id} | tostring)
        else
          .
        end;

    if ($records | length) == 0 then
      empty
    else
      (if $tool == "claude" then 1729 else 8675309 end) as $salt
      | ($records | length) as $template_count
      | range(0; $turns) as $turn
      | range(0; $template_count) as $index
      | (hash_num($turn; $index; $salt)) as $hash_num
      | ($hash_num | lpad(12)) as $hash
      | ("perf-\($tool)-\($turn)-\($index)-\($hash)") as $id
      | (timestamp_for_hash($hash_num)) as $timestamp
      | (($hash_num % 5000) + 1 | floor) as $offset
      | $records[$index]
      | if $tool == "claude" then
          claude_record($id; $timestamp; $offset)
        else
          codex_record($id; $hash; $timestamp; $offset)
        end
    end
    ' > "$output"
}

write_fixture_metadata() {
  local metadata_path="$1"
  local claude_records codex_records claude_bytes codex_bytes
  claude_records="$(line_count "$CLAUDE_FIXTURE")"
  codex_records="$(line_count "$CODEX_FIXTURE")"
  claude_bytes="$(byte_count "$CLAUDE_FIXTURE")"
  codex_bytes="$(byte_count "$CODEX_FIXTURE")"

  jq -n \
    --arg schema "clawgs.perf.extract_fixture.v1" \
    --arg claude "$CLAUDE_FIXTURE" \
    --arg codex "$CODEX_FIXTURE" \
    --argjson turns "$TURNS_PER_TOOL" \
    --argjson claude_records "$claude_records" \
    --argjson codex_records "$codex_records" \
    --argjson claude_bytes "$claude_bytes" \
    --argjson codex_bytes "$codex_bytes" \
    '{
      schema: $schema,
      strategy: "Replay examples/demo JSONL with hash-derived timestamp and id substitution",
      turns_per_tool: $turns,
      fixtures: [
        {tool: "claude", path: $claude, records: $claude_records, bytes: $claude_bytes},
        {tool: "codex", path: $codex, records: $codex_records, bytes: $codex_bytes}
      ],
      total_records: ($claude_records + $codex_records),
      total_bytes: ($claude_bytes + $codex_bytes)
    }' > "$metadata_path"
}

verify_fixtures() {
  (cd "$FIXTURE_DIR" && sha256sum -c SHA256SUMS >/dev/null)
}

synthesize_fixtures() {
  mkdir -p "$FIXTURE_DIR"
  local tmpdir
  tmpdir="$(mktemp -d "$FIXTURE_DIR/.tmp.XXXXXX")"

  synthesize_tool_fixture "claude" "$CLAUDE_SAMPLE" "$TURNS_PER_TOOL" "$tmpdir/claude-large.jsonl"
  synthesize_tool_fixture "codex" "$CODEX_SAMPLE" "$TURNS_PER_TOOL" "$tmpdir/codex-large.jsonl"

  mv "$tmpdir/claude-large.jsonl" "$CLAUDE_FIXTURE"
  mv "$tmpdir/codex-large.jsonl" "$CODEX_FIXTURE"
  rmdir "$tmpdir"

  (cd "$FIXTURE_DIR" && sha256sum claude-large.jsonl codex-large.jsonl > SHA256SUMS)
  verify_fixtures
  write_fixture_metadata "$FIXTURE_DIR/metadata.json"
}

ensure_fixtures() {
  if [[ -s "$CLAUDE_FIXTURE" && -s "$CODEX_FIXTURE" && -s "$FIXTURE_DIR/SHA256SUMS" ]]; then
    if verify_fixtures; then
      write_fixture_metadata "$FIXTURE_DIR/metadata.json"
      return
    fi
  fi

  synthesize_fixtures
}

print_dry_run() {
  local claude_records codex_records total_records total_bytes
  claude_records="$(line_count "$CLAUDE_FIXTURE")"
  codex_records="$(line_count "$CODEX_FIXTURE")"
  total_records=$((claude_records + codex_records))
  total_bytes=$(($(byte_count "$CLAUDE_FIXTURE") + $(byte_count "$CODEX_FIXTURE")))

  printf 'binary: %s\n' "$CLAWGS"
  printf 'turns_per_tool: %s\n' "$TURNS_PER_TOOL"
  printf 'fixtures:\n'
  printf '  claude: %s (%s records)\n' "$CLAUDE_FIXTURE" "$claude_records"
  printf '  codex:  %s (%s records)\n' "$CODEX_FIXTURE" "$codex_records"
  printf 'total_records: %s\n' "$total_records"
  printf 'total_bytes: %s\n' "$total_bytes"
  printf 'commands:\n'
  printf '  %s extract --tool claude --input %s >/dev/null\n' "$CLAWGS" "$CLAUDE_FIXTURE"
  printf '  %s extract --tool codex --input %s >/dev/null\n' "$CLAWGS" "$CODEX_FIXTURE"
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "$script_dir/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$script_dir/../.." && pwd))"

ARTIFACT_DIR="$repo_root/tests/artifacts/perf/extract"
FIXTURE_DIR="$ARTIFACT_DIR/fixtures"
CLAUDE_SAMPLE="$repo_root/examples/demo/claude-sample.jsonl"
CODEX_SAMPLE="$repo_root/examples/demo/codex-sample.jsonl"
CLAUDE_FIXTURE="$FIXTURE_DIR/claude-large.jsonl"
CODEX_FIXTURE="$FIXTURE_DIR/codex-large.jsonl"
CLAWGS="${CLAWGS_BIN:-$repo_root/target/release-perf/clawgs}"
LOOPS="${EXTRACT_SCENARIO_LOOPS:-1}"
TURNS_PER_TOOL="${EXTRACT_SCENARIO_TURNS:-2000}"

dry_run=0
synthesize_only=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    --synthesize-only)
      synthesize_only=1
      shift
      ;;
    --*)
      die "unknown option: $1"
      ;;
    *)
      die "unexpected argument: $1"
      ;;
  esac
done

if [[ "$synthesize_only" -eq 1 ]]; then
  require_tool jq
  require_tool sha256sum
  require_positive_int "EXTRACT_SCENARIO_TURNS" "$TURNS_PER_TOOL"
  synthesize_fixtures
  (cd "$FIXTURE_DIR" && sha256sum -c SHA256SUMS)
  exit 0
fi

require_tool jq
require_tool sha256sum
require_positive_int "EXTRACT_SCENARIO_LOOPS" "$LOOPS"
require_positive_int "EXTRACT_SCENARIO_TURNS" "$TURNS_PER_TOOL"

ensure_fixtures

if [[ "$dry_run" -eq 1 ]]; then
  print_dry_run
  exit 0
fi

if [[ ! -x "$CLAWGS" ]]; then
  die "clawgs binary is not executable: $CLAWGS"
fi

for ((i = 0; i < LOOPS; i++)); do
  "$CLAWGS" extract --tool claude --input "$CLAUDE_FIXTURE" >/dev/null
  "$CLAWGS" extract --tool codex --input "$CODEX_FIXTURE" >/dev/null
done
