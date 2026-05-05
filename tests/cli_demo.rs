use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn run_demo(args: &[&str]) -> std::process::Output {
    let home = TempDir::new().expect("temp home");
    let cwd = TempDir::new().expect("temp cwd");
    let mut command = Command::new(env!("CARGO_BIN_EXE_clawgs"));
    command
        .arg("demo")
        .args(args)
        .current_dir(cwd.path())
        .env("HOME", home.path());
    command.output().expect("failed to run clawgs demo")
}

#[test]
fn demo_extract_codex_outputs_embedded_input_and_snapshot() {
    let output = run_demo(&["extract", "--tool", "codex", "--pretty"]);

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("stdout should be json");
    assert_eq!(json["demo"], "extract");
    assert_eq!(json["tool"], "codex");
    assert!(json["input_jsonl"]
        .as_str()
        .expect("input should be a string")
        .contains("Build a parser"));
    assert_eq!(json["output"]["schema_version"], "clawgs.v2");
    assert_eq!(json["output"]["source"]["tool"], "codex");
    assert_eq!(
        json["output"]["source"]["path"],
        "embedded:examples/demo/codex-sample.jsonl"
    );
    assert!(!json["output"]["source"]["discovered"]
        .as_bool()
        .expect("discovered should be bool"));
    assert_eq!(json["output"]["snapshot"]["user_task"], "Build a parser");
}

#[test]
fn demo_extract_claude_outputs_embedded_input_and_snapshot() {
    let output = run_demo(&["extract", "--tool", "claude", "--pretty"]);

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("stdout should be json");
    assert_eq!(json["demo"], "extract");
    assert_eq!(json["tool"], "claude");
    assert!(json["input_jsonl"]
        .as_str()
        .expect("input should be a string")
        .contains("Summarize logs"));
    assert_eq!(json["output"]["schema_version"], "clawgs.v2");
    assert_eq!(json["output"]["source"]["tool"], "claude");
    assert_eq!(
        json["output"]["source"]["path"],
        "embedded:examples/demo/claude-sample.jsonl"
    );
    assert!(!json["output"]["source"]["discovered"]
        .as_bool()
        .expect("discovered should be bool"));
    assert_eq!(json["output"]["snapshot"]["user_task"], "Summarize logs");
}

#[test]
fn demo_emit_outputs_canonical_exchange_without_backends() {
    let home = TempDir::new().expect("temp home");
    let cwd = TempDir::new().expect("temp cwd");
    let output = Command::new(env!("CARGO_BIN_EXE_clawgs"))
        .arg("demo")
        .arg("emit")
        .arg("--pretty")
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .env("CLAWGS_MODEL_BACKEND", "openrouter")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("failed to run clawgs demo emit");

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("stdout should be json");
    assert_eq!(json["demo"], "emit");
    assert_eq!(json["hello"]["type"], "hello");
    assert_eq!(json["request"]["type"], "sync");
    assert_eq!(json["response"]["type"], "sync_result");
    assert_eq!(json["response"]["stream_instance_id"], "demo-stream");
    assert_eq!(json["response"]["metrics"]["llm_calls"], 1);
    assert!(json["response"]["metrics"]["last_backend_error"].is_null());
    assert_eq!(
        json["response"]["updates"][0]["thought"],
        "Turning raw transcripts into stable session state"
    );
    assert_eq!(json["response"]["updates"][0]["thought_source"], "llm");
    let expected_cue = serde_json::json!({
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
    });
    assert_eq!(
        json["request"]["sessions"][0]["action_cues"],
        serde_json::json!([expected_cue.clone()])
    );
    assert_eq!(
        json["response"]["updates"][0]["action_cues"],
        serde_json::json!([expected_cue])
    );
}

#[test]
fn demo_extract_rejects_invalid_tool_values() {
    let output = run_demo(&["extract", "--tool", "invalid"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(stderr.contains("possible values"));
    assert!(stderr.contains("claude"));
    assert!(stderr.contains("codex"));
}

#[test]
fn demo_extract_preserves_limit_validation() {
    let output = run_demo(&["extract", "--tool", "codex", "--max-actions", "0"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(stderr.contains("--max-actions must be greater than 0"));
}
