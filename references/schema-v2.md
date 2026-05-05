# Clawgs Schema v2

`clawgs extract` emits a single JSON document with `schema_version: "clawgs.v2"`.
The machine-validatable JSON Schema lives at `references/clawgs.v2.schema.json`.

## Top-Level Fields

- `schema_version` (`string`): fixed to `"clawgs.v2"`
- `source` (`object`): source metadata
- `snapshot` (`object`): normalized context snapshot
- `stats` (`object`): parse and input metrics
- `generated_at` (`string`): ISO-8601 UTC timestamp
- `raw_events` (`array`, optional): included only with `--include-raw`

## source

- `tool` (`"claude" | "codex"`)
- `path` (`string`): file path used for extraction
- `discovered` (`boolean`): `true` when path came from discovery logic
- `cwd` (`string`): cwd used for discovery/matching

## snapshot

- `user_task` (`string | null`): latest detected user prompt/task
- `current_tool` (`Action | null`): latest detected tool/thinking action
- `token_count` (`number`): latest observed `input_tokens`
- `awaiting_user_input` (`boolean`, optional): present and `true` when the latest transcript state appears to be waiting for the user
- `awaiting_user_text` (`string | null`, optional): short text associated with the awaiting-user state, when detected
- `recent_actions` (`Action[]`): bounded action list, oldest to newest
- `commit_signal` (`CommitSignal`, optional): Codex-only commit-readiness nudge derived from transcript evidence
- `action_cues` (`ActionCue[]`, optional): deterministic attention facts derived from transcript evidence; omitted when no cue is active

### Action

- `tool` (`string`): tool or activity label
- `detail` (`string | null`): short normalized detail
- `kind` (`"tool_use" | "text" | "thinking" | "function_call" | "other"`)
- `ts` (`string | null`): timestamp when available in source event

### CommitSignal

- `candidate` (`boolean`): `true` when edits were observed, validation succeeded after the latest edit, the dirty tree was checked after the latest edit, and no commit was seen after the latest edit
- `edited` (`boolean`): `true` when a completed edit action such as `apply_patch` was observed
- `validated` (`boolean`): `true` when a successful test/lint/typecheck command was paired with successful command output after the latest edit
- `dirty_checked` (`boolean`): `true` when the transcript shows a git dirty-tree check with successful command output after the latest edit
- `commit_seen` (`boolean`): `true` when the transcript shows a git commit command with successful command output after the latest edit

### ActionCue

- `kind` (`"awaiting_user" | "commit_ready" | "validation_missing_after_edit" | "dirty_check_missing"`): active cue category; `validation_missing_after_edit` means no successful validation has been observed after the latest edit
- `status` (`"active"`): current cue state
- `source` (`"transcript"`): cue evidence source
- `confidence` (`"deterministic"`): cue was derived from parser-visible facts, not an LLM judgment
- `evidence` (`string[]`): non-empty compact evidence labels used to derive the cue; labels are schema-enumerated and kind-specific, and edit-dependent labels are scoped to observations after the latest edit

Kind-specific evidence arrays:

- `awaiting_user`: `["awaiting_user_input"]`
- `commit_ready`: `["edit_seen", "validation_succeeded", "dirty_tree_checked_after_latest_edit", "commit_not_seen_after_latest_edit"]`
- `validation_missing_after_edit`: `["edit_seen", "fresh_validation_not_seen", "commit_not_seen_after_latest_edit"]`
- `dirty_check_missing`: `["edit_seen", "validation_succeeded", "dirty_tree_check_not_seen_after_latest_edit", "commit_not_seen_after_latest_edit"]`

## stats

- `events_seen` (`number`): successfully parsed JSONL lines
- `malformed_lines_skipped` (`number`): non-JSON lines ignored
- `bytes_read` (`number`): file bytes read

## Example

```json
{
  "schema_version": "clawgs.v2",
  "source": {
    "tool": "codex",
    "path": "/tmp/rollout-abc.jsonl",
    "discovered": false,
    "cwd": "/tmp/project"
  },
  "snapshot": {
    "user_task": "Build a parser",
    "current_tool": {
      "tool": "exec_command",
      "detail": "ls -la",
      "kind": "function_call",
      "ts": null
    },
    "token_count": 1212,
    "recent_actions": [
      {
        "tool": "exec_command",
        "detail": "ls -la",
        "kind": "function_call",
        "ts": null
      }
    ],
    "commit_signal": {
      "candidate": false,
      "edited": false,
      "validated": false,
      "dirty_checked": false,
      "commit_seen": false
    }
  },
  "stats": {
    "events_seen": 4,
    "malformed_lines_skipped": 0,
    "bytes_read": 286
  },
  "generated_at": "2026-02-26T20:11:56Z"
}
```
