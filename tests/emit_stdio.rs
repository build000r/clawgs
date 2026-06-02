use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

fn sync_request(id: &str, sessions: Vec<Value>) -> Value {
    serde_json::json!({
        "type": "sync",
        "id": id,
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
        "sessions": sessions
    })
}

fn session_snapshot(replay_text: &str) -> Value {
    serde_json::json!({
        "session_id": "sess-1",
        "state": "busy",
        "exited": false,
        "tool": "codex",
        "cwd": "/tmp/project",
        "replay_text": replay_text,
        "thought": null,
        "thought_state": "holding",
        "thought_source": "carry_forward",
        "objective_fingerprint": null,
        "thought_updated_at": null,
        "token_count": 1000,
        "context_limit": 192000,
        "last_activity_at": "2026-02-26T21:00:00Z",
        "rest_state": "active",
        "commit_candidate": false,
        "action_cues": []
    })
}

fn run_emit_stdio(lines: &[Value], envs: &[(&str, &str)], remove_envs: &[&str]) -> Vec<Value> {
    let home = TempDir::new().expect("temp home");
    let mut command = Command::new(env!("CARGO_BIN_EXE_clawgs"));
    command
        .arg("emit")
        .arg("--stdio")
        .env("HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    for key in remove_envs {
        command.env_remove(key);
    }

    let mut child = command.spawn().expect("failed to spawn clawgs emit");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for line in lines {
            writeln!(stdin, "{line}").expect("write sync request");
        }
    }

    let output = child.wait_with_output().expect("wait for child");
    assert!(
        output.status.success(),
        "process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("stdout utf8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect()
}

fn fake_grok_script(temp_dir: &TempDir) -> std::path::PathBuf {
    let script_path = temp_dir.path().join("fake-grok");
    std::fs::write(
        &script_path,
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf 'fake grok thought\\n'\n",
    )
    .expect("write fake grok");
    let mut permissions = std::fs::metadata(&script_path)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).expect("chmod fake grok");
    script_path
}

fn fake_grok_marker_script(temp_dir: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let script_path = temp_dir.path().join("fake-grok-marker");
    let marker_path = temp_dir.path().join("grok-invoked");
    std::fs::write(
        &script_path,
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf 'invoked\\n' > \"${FAKE_GROK_MARKER:?}\"\nprintf 'fake grok thought\\n'\n",
    )
    .expect("write fake grok marker");
    let mut permissions = std::fs::metadata(&script_path)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).expect("chmod fake grok marker");
    (script_path, marker_path)
}

#[test]
fn perf_emit_stdio_scenario_is_offline_by_default() {
    let temp_dir = TempDir::new().expect("temp dir");
    let (fake_grok, marker_path) = fake_grok_marker_script(&temp_dir);

    let output = Command::new("bash")
        .arg("scripts/perf/emit_stdio_scenario.sh")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("CLAWGS_PERF_BIN", env!("CARGO_BIN_EXE_clawgs"))
        .env("CLAWGS_MODEL_BACKEND", "grok")
        .env("CLAWGS_GROK_BIN", fake_grok)
        .env("FAKE_GROK_MARKER", &marker_path)
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("run perf scenario");

    assert!(
        output.status.success(),
        "scenario failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("scenario B offline run complete"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !marker_path.exists(),
        "offline perf scenario should not invoke a model backend"
    );
}

#[test]
fn perf_emit_stdio_scenario_model_enabled_is_opt_in() {
    let temp_dir = TempDir::new().expect("temp dir");
    let (fake_grok, marker_path) = fake_grok_marker_script(&temp_dir);

    let output = Command::new("bash")
        .arg("scripts/perf/emit_stdio_scenario.sh")
        .arg("--model-enabled")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("CLAWGS_PERF_BIN", env!("CARGO_BIN_EXE_clawgs"))
        .env("CLAWGS_MODEL_BACKEND", "grok")
        .env("CLAWGS_GROK_BIN", fake_grok)
        .env("FAKE_GROK_MARKER", &marker_path)
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("run perf scenario");

    assert!(
        output.status.success(),
        "scenario failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("scenario B model-enabled run complete"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        marker_path.exists(),
        "model-enabled perf scenario should preserve backend invocation"
    );
}

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
fn emit_stdio_reports_missing_openrouter_key_as_sync_result() {
    let lines = run_emit_stdio(
        &[
            sync_request("req-bootstrap", vec![session_snapshot("$ ")]),
            sync_request(
                "req-openrouter",
                vec![session_snapshot(
                    "running cargo test --all\nreviewing OpenRouter credential fallback behavior after a meaningful terminal update that should trigger the model path\n",
                )],
            ),
        ],
        &[("CLAWGS_MODEL_BACKEND", "openrouter")],
        &["OPENROUTER_API_KEY"],
    );

    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[1]["type"], "sync_result");
    assert_eq!(lines[2]["type"], "sync_result");
    assert_eq!(lines[2]["id"], "req-openrouter");
    assert!(lines[2]["metrics"]["last_backend_error"]
        .as_str()
        .expect("backend error")
        .contains("OPENROUTER_API_KEY not set"));
}

#[test]
fn emit_stdio_legacy_backend_aliases_map_to_grok_missing_binary_errors() {
    for alias in ["claude", "codex"] {
        let lines = run_emit_stdio(
            &[
                sync_request("req-bootstrap", vec![session_snapshot("$ ")]),
                sync_request(
                    &format!("req-{alias}"),
                    vec![session_snapshot(
                        "running cargo test --all\nreviewing legacy backend alias fallback behavior after a meaningful terminal update that should trigger the model path\n",
                    )],
                ),
            ],
            &[
                ("CLAWGS_MODEL_BACKEND", alias),
                ("CLAWGS_GROK_BIN", "/definitely/missing-grok"),
            ],
            &["OPENROUTER_API_KEY"],
        );

        assert_eq!(lines[0]["type"], "hello");
        assert_eq!(lines[1]["type"], "sync_result");
        assert_eq!(lines[2]["type"], "sync_result");
        let error = lines[2]["metrics"]["last_backend_error"]
            .as_str()
            .expect("backend error");
        assert!(
            error.contains("grok") && error.contains("No such file or directory"),
            "unexpected backend error for alias {alias}: {error}"
        );
    }
}

#[test]
fn emit_stdio_uses_fake_grok_binary_for_live_thought() {
    let temp_dir = TempDir::new().expect("temp dir");
    let fake_grok = fake_grok_script(&temp_dir);
    let fake_grok = fake_grok.to_str().expect("fake grok path");
    let lines = run_emit_stdio(
        &[
            sync_request("req-bootstrap", vec![session_snapshot("$ ")]),
            sync_request(
                "req-grok",
                vec![session_snapshot(
                    "running cargo test --all\nreviewing backend selection and no credential behavior after a meaningful terminal update that should trigger the model path and call the fake Grok binary\n",
                )],
            ),
        ],
        &[
            ("CLAWGS_MODEL_BACKEND", "grok"),
            ("CLAWGS_GROK_BIN", fake_grok),
        ],
        &["OPENROUTER_API_KEY"],
    );

    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[1]["type"], "sync_result");
    assert_eq!(lines[2]["type"], "sync_result");
    assert_eq!(lines[2]["id"], "req-grok");
    assert_eq!(lines[2]["updates"][0]["thought"], "fake grok thought");
    assert_eq!(lines[2]["metrics"]["llm_calls"], 1);
    assert!(lines[2]["metrics"]["last_backend_error"].is_null());
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
