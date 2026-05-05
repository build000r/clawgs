# Clawgs Emit Protocol v2

`clawgs emit --stdio` speaks line-delimited JSON (`NDJSON`) over stdin/stdout.
The machine-validatable JSON Schema lives at `references/clawgs.emit.v2.schema.json`.

## Startup

On boot, the daemon writes:

```json
{"type":"hello","protocol":"clawgs.emit.v2","engine_version":"0.2.0"}
```

## Request

Send one `sync` object per line:

```json
{
  "type": "sync",
  "id": "req-123",
  "now": "2026-02-26T21:00:00Z",
  "config": {
    "enabled": true,
    "model": "",
    "backend": "",
    "cadence_hot_ms": 15000,
    "cadence_warm_ms": 45000,
    "cadence_cold_ms": 120000,
    "agent_prompt": null,
    "terminal_prompt": null
  },
  "sessions": []
}
```

`agent_prompt` and `terminal_prompt` are optional; missing, `null`, and blank
strings are normalized to no override. The JSON Schema covers scalar cadence
ranges, while the daemon also validates ordering at runtime:
`cadence_hot_ms <= cadence_warm_ms <= cadence_cold_ms`.

## Success Response

```json
{
  "type": "sync_result",
  "id": "req-123",
  "stream_instance_id": "stream-1",
  "updates": [
    {
      "session_id": "sess-1",
      "stream_instance_id": "stream-1",
      "emission_seq": 7,
      "thought": "Validating fallback handling",
      "token_count": 144379,
      "context_limit": 256000,
      "thought_state": "holding",
      "thought_source": "llm",
      "objective_changed": false,
      "bubble_precedence": "thought_first",
      "at": "2026-03-29T21:00:00Z",
      "objective_fingerprint": "obj-123",
      "rest_state": "active",
      "commit_candidate": true,
      "action_cues": [
        {
          "kind": "commit_ready",
          "status": "active",
          "source": "transcript",
          "confidence": "deterministic",
          "evidence": [
            "edit_seen",
            "validation_succeeded",
            "dirty_tree_checked_after_latest_edit",
            "commit_not_seen_after_latest_edit"
          ]
        }
      ],
      "timing": {
        "run_started_at": "2026-03-29T20:52:14Z",
        "run_elapsed_ms": 466000,
        "idle_elapsed_ms": 1200
      },
      "cues": {
        "cadence_tier": "warm",
        "cadence_ms": 45000,
        "next_llm_eligible_at": "2026-03-29T21:00:30Z",
        "context_source": "transcript"
      }
    }
  ],
  "metrics": {
    "sessions_seen": 1,
    "llm_calls": 1,
    "suppressed": 0
  }
}
```

### Update Fields

- `timing.run_started_at`: start of the current active run.
- `timing.run_finished_at`: present only after the run has stopped and the elapsed timer is frozen.
- `timing.run_elapsed_ms`: live elapsed time while active, frozen elapsed time while stopped.
- `timing.idle_elapsed_ms`: milliseconds since the pane last showed activity.
- `action_cues`: optional deterministic attention facts derived from transcript evidence. This is separate from cadence `cues`; downstream tools should treat these as facts, not commands.
- `action_cues[].kind`: one of `awaiting_user`, `commit_ready`, `validation_missing_after_edit`, or `dirty_check_missing`.
- `action_cues[].status`: currently `active`.
- `action_cues[].source`: currently `transcript`.
- `action_cues[].confidence`: currently `deterministic`.
- `action_cues[].evidence`: non-empty compact labels for the transcript facts that triggered the cue. Labels are schema-enumerated and kind-specific, and edit-dependent labels are scoped to observations after the latest edit.
- Kind-specific evidence arrays:
  - `awaiting_user`: `["awaiting_user_input"]`
  - `commit_ready`: `["edit_seen", "validation_succeeded", "dirty_tree_checked_after_latest_edit", "commit_not_seen_after_latest_edit"]`
  - `validation_missing_after_edit`: `["edit_seen", "fresh_validation_not_seen", "commit_not_seen_after_latest_edit"]`
  - `dirty_check_missing`: `["edit_seen", "validation_succeeded", "dirty_tree_check_not_seen_after_latest_edit", "commit_not_seen_after_latest_edit"]`
- `emit --stdio` rejects inbound sessions whose `action_cues` evidence does not
  match the kind-specific arrays. Library callers that invoke `EmitEngine`
  directly get invalid inbound cues filtered as non-facts instead.
- `cues.cadence_tier`: current cadence bucket, one of `hot`, `warm`, or `cold`.
- `cues.cadence_ms`: minimum milliseconds between eligible LLM emits for the current cadence tier.
- `cues.next_llm_eligible_at`: next wall-clock time when a fresh LLM emit may run.
- `cues.context_source`: whether the status came from transcript context or terminal-only context.

## Error Response

```json
{
  "type": "error",
  "id": "req-123",
  "code": "invalid_config",
  "message": "cadence_hot_ms must be between 5000 and 300000"
}
```
