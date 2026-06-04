use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

fn run_clawgs_json(args: &[&str]) -> Value {
    let home = TempDir::new().expect("temp home");
    let output = Command::new(env!("CARGO_BIN_EXE_clawgs"))
        .args(args)
        .env("HOME", home.path())
        .output()
        .expect("run clawgs");
    assert!(
        output.status.success(),
        "clawgs command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("json stdout")
}

fn dirty_check_missing_cue() -> Value {
    serde_json::json!({
        "kind": "dirty_check_missing",
        "status": "active",
        "source": "transcript",
        "confidence": "deterministic",
        "evidence": [
            "edit_seen",
            "validation_succeeded",
            "dirty_tree_check_not_seen_after_latest_edit",
            "commit_not_seen_after_latest_edit"
        ]
    })
}

fn awaiting_user_cue() -> Value {
    serde_json::json!({
        "kind": "awaiting_user",
        "status": "active",
        "source": "transcript",
        "confidence": "deterministic",
        "evidence": ["awaiting_user_input"]
    })
}

fn swimmers_session_snapshot() -> Value {
    serde_json::json!({
        "session_id": "swimmers:codex:1",
        "state": "busy",
        "exited": false,
        "tool": "codex",
        "cwd": "/swimmers/downstream",
        "replay_text": "sh",
        "thought": null,
        "thought_state": "holding",
        "thought_source": "carry_forward",
        "objective_fingerprint": "objective-swimmers-1",
        "thought_updated_at": null,
        "token_count": 144379,
        "context_limit": 256000,
        "last_activity_at": "2026-06-04T17:00:00Z",
        "rest_state": "active",
        "commit_candidate": false,
        "action_cues": [awaiting_user_cue()]
    })
}

#[test]
fn extract_output_keeps_swimmers_snapshot_fields() {
    let output = run_clawgs_json(&[
        "extract",
        "--tool",
        "codex",
        "--cwd",
        "/swimmers/downstream",
        "--input",
        "tests/fixtures/codex-current.jsonl",
        "--pretty",
    ]);

    assert_eq!(output["schema_version"], "clawgs.v2");
    assert_eq!(output["source"]["tool"], "codex");
    assert_eq!(output["source"]["cwd"], "/swimmers/downstream");
    assert_eq!(
        output["snapshot"]["user_task"],
        "Ship preview-first widget fix"
    );
    assert_eq!(output["snapshot"]["token_count"], 144379);
    assert_eq!(output["snapshot"]["commit_signal"]["edited"], true);
    assert_eq!(output["snapshot"]["commit_signal"]["validated"], true);
    assert_eq!(output["snapshot"]["commit_signal"]["dirty_checked"], false);
    assert_eq!(
        output["snapshot"]["action_cues"],
        serde_json::json!([dirty_check_missing_cue()])
    );
}

#[test]
fn emit_stdio_accepts_swimmers_sync_shape_and_returns_parseable_result() {
    let home = TempDir::new().expect("temp home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_clawgs"))
        .arg("emit")
        .arg("--stdio")
        .env("HOME", home.path())
        .env("CLAWGS_MODEL_BACKEND", "openrouter")
        .env_remove("OPENROUTER_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn clawgs emit");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            "{}",
            serde_json::json!({
                "type": "sync",
                "id": "swimmers-1",
                "now": "2026-06-04T17:00:00Z",
                "config": {
                    "enabled": true,
                    "model": "",
                    "backend": "openrouter",
                    "cadence_hot_ms": 15000,
                    "cadence_warm_ms": 45000,
                    "cadence_cold_ms": 120000,
                    "agent_prompt": "",
                    "terminal_prompt": ""
                },
                "sessions": [swimmers_session_snapshot()]
            })
        )
        .expect("write swimmers-shaped sync");
    }

    let output = child.wait_with_output().expect("wait for clawgs emit");
    assert!(
        output.status.success(),
        "clawgs emit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let mut lines = stdout.lines();
    let hello: Value = serde_json::from_str(lines.next().expect("hello line")).expect("hello json");
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol"], "clawgs.emit.v2");

    let result: Value =
        serde_json::from_str(lines.next().expect("sync_result line")).expect("sync_result json");
    assert_eq!(result["type"], "sync_result");
    assert_eq!(result["id"], "swimmers-1");
    assert!(result["stream_instance_id"].as_str().is_some());
    assert_eq!(result["metrics"]["llm_calls"], 0);
    assert_eq!(result["metrics"]["last_backend_error"], Value::Null);
    assert_eq!(result["updates"][0]["session_id"], "swimmers:codex:1");
    assert_eq!(result["updates"][0]["thought"], Value::Null);
    assert_eq!(result["updates"][0]["token_count"], 144379);
    assert_eq!(result["updates"][0]["context_limit"], 256000);
    assert_eq!(result["updates"][0]["thought_state"], "sleeping");
    assert_eq!(result["updates"][0]["rest_state"], "sleeping");
    assert_eq!(
        result["updates"][0]["action_cues"],
        serde_json::json!([awaiting_user_cue()])
    );
}
