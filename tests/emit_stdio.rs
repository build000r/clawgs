use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

#[test]
fn emit_stdio_writes_hello_and_sync_result() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clawgs"))
        .arg("emit")
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn clawgs emit");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            "{}",
            serde_json::json!({
                "type": "sync",
                "id": "req-1",
                "now": "2026-02-26T21:00:00Z",
                "config": {
                    "enabled": true,
                    "model": "",
                    "cadence_hot_ms": 15000,
                    "cadence_warm_ms": 45000,
                    "cadence_cold_ms": 120000,
                    "agent_prompt": null,
                    "terminal_prompt": null
                },
                "sessions": []
            })
        )
        .expect("write sync request");
    }

    let output = child.wait_with_output().expect("wait for child");
    assert!(
        output.status.success(),
        "process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let mut lines = stdout.lines();

    let hello: Value = serde_json::from_str(lines.next().expect("hello line")).expect("hello json");
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol"], "clawgs.emit.v2");

    let result: Value =
        serde_json::from_str(lines.next().expect("sync_result line")).expect("result json");
    assert_eq!(result["type"], "sync_result");
    assert_eq!(result["id"], "req-1");
    assert!(result["stream_instance_id"].as_str().is_some());
}

#[test]
fn emit_stdio_rejects_invalid_inbound_action_cue_evidence() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clawgs"))
        .arg("emit")
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn clawgs emit");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            "{}",
            serde_json::json!({
                "type": "sync",
                "id": "req-invalid-cue",
                "now": "2026-02-26T21:00:00Z",
                "config": {
                    "enabled": true,
                    "model": "",
                    "cadence_hot_ms": 15000,
                    "cadence_warm_ms": 45000,
                    "cadence_cold_ms": 120000,
                    "agent_prompt": null,
                    "terminal_prompt": null
                },
                "sessions": [
                    {
                        "session_id": "sess-1",
                        "state": "busy",
                        "exited": false,
                        "tool": "codex",
                        "cwd": "/tmp/project",
                        "replay_text": "sh",
                        "thought": null,
                        "thought_state": "holding",
                        "thought_source": "carry_forward",
                        "objective_fingerprint": null,
                        "thought_updated_at": null,
                        "token_count": 0,
                        "context_limit": 0,
                        "last_activity_at": "2026-02-26T21:00:00Z",
                        "rest_state": "active",
                        "commit_candidate": false,
                        "action_cues": [
                            {
                                "kind": "awaiting_user",
                                "status": "active",
                                "source": "transcript",
                                "confidence": "deterministic",
                                "evidence": []
                            }
                        ]
                    }
                ]
            })
        )
        .expect("write sync request");
    }

    let output = child.wait_with_output().expect("wait for child");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let mut lines = stdout.lines();
    let hello: Value = serde_json::from_str(lines.next().expect("hello line")).expect("hello json");
    assert_eq!(hello["type"], "hello");

    let error: Value = serde_json::from_str(lines.next().expect("error line")).expect("error json");
    assert_eq!(error["type"], "error");
    assert_eq!(error["id"], "req-invalid-cue");
    assert_eq!(error["code"], "invalid_request");
}
