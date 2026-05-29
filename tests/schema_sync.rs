use std::path::{Path, PathBuf};

use chrono::{Duration, TimeZone, Utc};
use clawgs::emit::model_client::ModelBackend;
use clawgs::emit::protocol::{
    BubblePrecedence, CadenceTier, ContextSource, CueInfo, ErrorMessage, HelloMessage, RestState,
    SessionDelta, SessionDeltaField, SessionDeltaKind, SessionSnapshot, SessionState, SyncMetrics,
    SyncRequest, SyncResultMessage, ThoughtConfig, ThoughtSource, ThoughtState, ThoughtUpdate,
    TimingInfo,
};
use clawgs::{
    extract, ActionCue, ActionCueConfidence, ActionCueKind, ActionCueSource, ActionCueStatus,
    AgentTool, ExtractOptions,
};
use jsonschema::JSONSchema;
use serde_json::Value;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}"))
}

fn load_json(input: &str) -> Value {
    serde_json::from_str(input).expect("valid json")
}

fn validate(schema: &Value, instance: &Value) {
    let compiled = JSONSchema::compile(schema).expect("valid json schema");
    let result = compiled.validate(instance);
    if let Err(errors) = result {
        let messages: Vec<String> = errors.map(|error| error.to_string()).collect();
        panic!("schema validation failed:\n{}", messages.join("\n"));
    };
}

fn string_array_at(schema: &Value, pointer: &str) -> Vec<String> {
    schema
        .pointer(pointer)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("missing array at {pointer}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("non-string value at {pointer}: {value:?}"))
                .to_string()
        })
        .collect()
}

fn action_cue_schema_evidence(schema: &Value, kind: ActionCueKind) -> Vec<String> {
    let all_of = schema
        .pointer("/$defs/action_cue/allOf")
        .and_then(Value::as_array)
        .expect("action cue allOf");
    let kind_name = kind.as_str();
    let rule = all_of
        .iter()
        .find(|rule| {
            rule.pointer("/if/properties/kind/const")
                .and_then(Value::as_str)
                == Some(kind_name)
        })
        .unwrap_or_else(|| panic!("missing schema rule for action cue kind {kind_name}"));

    rule.pointer("/then/properties/evidence/const")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("missing evidence const for action cue kind {kind_name}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("non-string evidence for {kind_name}: {value:?}"))
                .to_string()
        })
        .collect()
}

fn validation_errors(schema: &Value, instance: &Value) -> Vec<String> {
    let compiled = JSONSchema::compile(schema).expect("valid json schema");
    compiled
        .validate(instance)
        .err()
        .map(|errors| errors.map(|error| error.to_string()).collect())
        .unwrap_or_default()
}

fn extract_fixture(tool: AgentTool, input: &Path) -> Value {
    let output = extract(
        tool,
        input,
        Path::new("/schema-sync/project"),
        false,
        &ExtractOptions {
            include_raw: true,
            ..ExtractOptions::default()
        },
    )
    .expect("extract fixture");
    serde_json::to_value(output).expect("serialize extract output")
}

fn sample_session(now: chrono::DateTime<Utc>) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "sess-1".to_string(),
        state: SessionState::Busy,
        exited: false,
        tool: Some("codex".to_string()),
        cwd: "/schema-sync/project".to_string(),
        replay_text: "cargo test --test schema_sync".to_string(),
        thought: Some("Validating schema drift".to_string()),
        thought_state: ThoughtState::Active,
        thought_source: ThoughtSource::Llm,
        objective_fingerprint: Some("objective-1".to_string()),
        thought_updated_at: Some(now),
        token_count: 144_379,
        context_limit: 256_000,
        last_activity_at: now - Duration::seconds(1),
        rest_state: RestState::Active,
        commit_candidate: true,
        action_cues: vec![commit_ready_cue()],
    }
}

fn sample_update(now: chrono::DateTime<Utc>) -> ThoughtUpdate {
    ThoughtUpdate {
        session_id: "sess-1".to_string(),
        stream_instance_id: Some("stream-1".to_string()),
        emission_seq: Some(7),
        thought: Some("Validating schema drift".to_string()),
        token_count: 144_379,
        context_limit: 256_000,
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::Llm,
        objective_changed: true,
        bubble_precedence: BubblePrecedence::ThoughtFirst,
        at: now,
        objective_fingerprint: Some("objective-1".to_string()),
        rest_state: RestState::Drowsy,
        commit_candidate: true,
        action_cues: vec![commit_ready_cue()],
        timing: Some(TimingInfo {
            run_started_at: now - Duration::minutes(3),
            run_finished_at: Some(now),
            run_elapsed_ms: 180_000,
            idle_elapsed_ms: 1_200,
        }),
        cues: Some(CueInfo {
            cadence_tier: CadenceTier::Warm,
            cadence_ms: 45_000,
            next_llm_eligible_at: now + Duration::seconds(45),
            context_source: ContextSource::Transcript,
        }),
    }
}

fn sample_session_delta() -> SessionDelta {
    SessionDelta {
        session_id: "sess-1".to_string(),
        kind: SessionDeltaKind::Changed,
        state: Some(SessionState::Busy),
        tool: Some("codex".to_string()),
        cwd: Some("/schema-sync/project".to_string()),
        changed_fields: vec![SessionDeltaField::ReplayText, SessionDeltaField::Activity],
        transcript_ambiguous: false,
    }
}

fn commit_ready_cue() -> ActionCue {
    ActionCue {
        kind: ActionCueKind::CommitReady,
        status: ActionCueStatus::Active,
        source: ActionCueSource::Transcript,
        confidence: ActionCueConfidence::Deterministic,
        evidence: vec![
            "edit_seen".to_string(),
            "validation_succeeded".to_string(),
            "dirty_tree_checked_after_latest_edit".to_string(),
            "commit_not_seen_after_latest_edit".to_string(),
        ],
    }
}

#[test]
fn schema_files_are_valid_json() {
    load_json(include_str!("../references/clawgs.v2.schema.json"));
    load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
}

#[test]
fn emit_v2_backend_schema_matches_runtime_aliases() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let schema_backends = string_array_at(&schema, "/$defs/thought_config/properties/backend/enum");
    let expected = [
        "",
        "openrouter",
        "grok",
        "grok_cli",
        "grok-cli",
        "claude",
        "claude_cli",
        "claude-cli",
        "codex",
        "codex_cli",
        "codex-cli",
    ];

    assert_eq!(schema_backends, expected);
    for backend in expected {
        assert!(
            backend.is_empty() || ModelBackend::from_env_value(backend).is_some(),
            "schema backend {backend:?} must be accepted by runtime validation"
        );
    }
}

#[test]
fn emit_v1_backend_schema_stays_historical() {
    let schema = load_json(include_str!("../references/clawgs.emit.v1.schema.json"));
    assert_eq!(
        string_array_at(&schema, "/$defs/thought_config/properties/backend/enum"),
        ["", "openrouter", "claude", "codex"]
    );
}

#[test]
fn emit_protocol_hello_example_matches_crate_version() {
    let docs = include_str!("../references/emit-protocol-v2.md");
    let expected = format!(
        r#"{{"type":"hello","protocol":"clawgs.emit.v2","engine_version":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        docs.contains(&expected),
        "emit protocol docs must show the current hello engine_version: {expected}"
    );
}

#[test]
fn action_cue_schema_evidence_matches_runtime_rules() {
    let extract_schema = load_json(include_str!("../references/clawgs.v2.schema.json"));
    let emit_schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let kinds = [
        ActionCueKind::AwaitingUser,
        ActionCueKind::CommitReady,
        ActionCueKind::ValidationMissingAfterEdit,
        ActionCueKind::DirtyCheckMissing,
    ];

    for kind in kinds {
        let runtime: Vec<String> = ActionCue::expected_evidence(kind)
            .iter()
            .map(|value| (*value).to_string())
            .collect();
        assert_eq!(
            action_cue_schema_evidence(&extract_schema, kind),
            runtime,
            "extract schema evidence drifted for {}",
            kind.as_str()
        );
        assert_eq!(
            action_cue_schema_evidence(&emit_schema, kind),
            runtime,
            "emit schema evidence drifted for {}",
            kind.as_str()
        );
    }
}

#[test]
fn claude_hook_reference_is_valid_and_targets_notify_command() {
    let snippet = load_json(include_str!("../references/claude-code-hooks.json"));
    let expected_command = "clawgs claude-hook-notify --socket \"$HOME/.tmux/clawgs-tmux.sock\"";
    for event in ["Notification", "PostToolUse", "Stop"] {
        assert_eq!(
            snippet["hooks"][event][0]["hooks"][0]["command"], expected_command,
            "hook snippet command drifted for {event}"
        );
    }

    let tmux_config = include_str!("../references/tmux-clawgs.conf");
    assert!(
        tmux_config.contains("--socket \"$HOME/.tmux/clawgs-tmux.sock\""),
        "tmux daemon snippet must use the same socket as Claude hook snippet"
    );
}

#[test]
fn extract_codex_output_validates() {
    let schema = load_json(include_str!("../references/clawgs.v2.schema.json"));
    let instance = extract_fixture(AgentTool::Codex, &fixture_path("codex-current.jsonl"));

    validate(&schema, &instance);
}

#[test]
fn extract_claude_output_validates() {
    let schema = load_json(include_str!("../references/clawgs.v2.schema.json"));
    let instance = extract_fixture(AgentTool::Claude, &fixture_path("claude-sample.jsonl"));

    validate(&schema, &instance);
}

#[test]
fn extract_schema_rejects_unknown_or_empty_action_cue_evidence() {
    let schema = load_json(include_str!("../references/clawgs.v2.schema.json"));
    let mut instance = extract_fixture(AgentTool::Codex, &fixture_path("codex-current.jsonl"));
    instance["snapshot"]["action_cues"] = serde_json::json!([
        {
            "kind": "commit_ready",
            "status": "active",
            "source": "transcript",
            "confidence": "deterministic",
            "evidence": []
        }
    ]);

    assert!(
        !validation_errors(&schema, &instance).is_empty(),
        "empty evidence must not validate"
    );

    instance["snapshot"]["action_cues"][0]["evidence"] =
        serde_json::json!(["dirty_tree_checked_after_latest_edt"]);
    assert!(
        !validation_errors(&schema, &instance).is_empty(),
        "typoed evidence label must not validate"
    );

    instance["snapshot"]["action_cues"][0]["evidence"] = serde_json::json!(["awaiting_user_input"]);
    assert!(
        !validation_errors(&schema, &instance).is_empty(),
        "evidence for the wrong cue kind must not validate"
    );
}

#[test]
fn emit_hello_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let instance = serde_json::to_value(HelloMessage::new()).expect("serialize hello");

    validate(&schema, &instance);
}

#[test]
fn emit_sync_request_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let now = Utc
        .with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("valid timestamp");
    let request = SyncRequest::new(
        "req-1",
        now,
        ThoughtConfig {
            enabled: true,
            model: "openai/gpt-5.4-mini".to_string(),
            backend: "openrouter".to_string(),
            cadence_hot_ms: 15_000,
            cadence_warm_ms: 45_000,
            cadence_cold_ms: 120_000,
            agent_prompt: Some("Summarize the current agent state.".to_string()),
            terminal_prompt: Some("Summarize terminal-only context.".to_string()),
        },
        vec![sample_session(now)],
    );
    let instance = serde_json::to_value(request).expect("serialize sync request");

    validate(&schema, &instance);
}

#[test]
fn emit_sync_request_validates_omitted_optional_prompts() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let instance = serde_json::json!({
        "type": "sync",
        "id": "req-1",
        "now": "2026-04-29T12:00:00Z",
        "config": {
            "enabled": true,
            "model": "openai/gpt-5.4-mini",
            "cadence_hot_ms": 15000,
            "cadence_warm_ms": 45000,
            "cadence_cold_ms": 120000
        },
        "sessions": []
    });

    validate(&schema, &instance);
}

#[test]
fn emit_sync_result_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let now = Utc
        .with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("valid timestamp");
    let message = SyncResultMessage::new(
        "req-1",
        "stream-1",
        vec![sample_update(now)],
        SyncMetrics {
            sessions_seen: 1,
            llm_calls: 1,
            suppressed: 0,
            last_backend_error: Some("backend unavailable".to_string()),
        },
    )
    .with_session_deltas(vec![sample_session_delta()]);
    let instance = serde_json::to_value(message).expect("serialize sync result");

    validate(&schema, &instance);
}

#[test]
fn emit_schema_rejects_unknown_session_delta_field() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let mut instance = serde_json::to_value(
        SyncResultMessage::new("req-1", "stream-1", Vec::new(), SyncMetrics::default())
            .with_session_deltas(vec![sample_session_delta()]),
    )
    .expect("serialize sync result");
    instance["session_deltas"][0]["changed_fields"] = serde_json::json!(["replay"]);

    assert!(
        !validation_errors(&schema, &instance).is_empty(),
        "unknown delta changed field must not validate"
    );
}

#[test]
fn emit_schema_rejects_unknown_or_empty_action_cue_evidence() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let now = Utc
        .with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("valid timestamp");
    let mut instance = serde_json::to_value(SyncResultMessage::new(
        "req-1",
        "stream-1",
        vec![sample_update(now)],
        SyncMetrics::default(),
    ))
    .expect("serialize sync result");
    instance["updates"][0]["action_cues"][0]["evidence"] = serde_json::json!([]);

    assert!(
        !validation_errors(&schema, &instance).is_empty(),
        "empty evidence must not validate"
    );

    instance["updates"][0]["action_cues"][0]["evidence"] =
        serde_json::json!(["validation_succeded"]);
    assert!(
        !validation_errors(&schema, &instance).is_empty(),
        "typoed evidence label must not validate"
    );

    instance["updates"][0]["action_cues"][0]["evidence"] =
        serde_json::json!(["awaiting_user_input"]);
    assert!(
        !validation_errors(&schema, &instance).is_empty(),
        "evidence for the wrong cue kind must not validate"
    );
}

#[test]
fn emit_error_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v2.schema.json"));
    let instance = serde_json::to_value(ErrorMessage::new(
        Some("req-1".to_string()),
        "invalid_config",
        "cadence_hot_ms must be between 5000 and 300000",
    ))
    .expect("serialize error");

    validate(&schema, &instance);
}
