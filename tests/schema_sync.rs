use std::path::{Path, PathBuf};

use chrono::{Duration, TimeZone, Utc};
use clawgs::emit::protocol::{
    BubblePrecedence, CadenceTier, ContextSource, CueInfo, ErrorMessage, HelloMessage, RestState,
    SessionSnapshot, SessionState, SyncMetrics, SyncRequest, SyncResultMessage, ThoughtConfig,
    ThoughtSource, ThoughtState, ThoughtUpdate, TimingInfo,
};
use clawgs::{extract, AgentTool, ExtractOptions};
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

#[test]
fn schema_files_are_valid_json() {
    load_json(include_str!("../references/clawgs.v1.schema.json"));
    load_json(include_str!("../references/clawgs.emit.v1.schema.json"));
}

#[test]
fn extract_codex_output_validates() {
    let schema = load_json(include_str!("../references/clawgs.v1.schema.json"));
    let instance = extract_fixture(AgentTool::Codex, &fixture_path("codex-current.jsonl"));

    validate(&schema, &instance);
}

#[test]
fn extract_claude_output_validates() {
    let schema = load_json(include_str!("../references/clawgs.v1.schema.json"));
    let instance = extract_fixture(AgentTool::Claude, &fixture_path("claude-sample.jsonl"));

    validate(&schema, &instance);
}

#[test]
fn emit_hello_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v1.schema.json"));
    let instance = serde_json::to_value(HelloMessage::new()).expect("serialize hello");

    validate(&schema, &instance);
}

#[test]
fn emit_sync_request_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v1.schema.json"));
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
fn emit_sync_result_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v1.schema.json"));
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
    );
    let instance = serde_json::to_value(message).expect("serialize sync result");

    validate(&schema, &instance);
}

#[test]
fn emit_error_validates() {
    let schema = load_json(include_str!("../references/clawgs.emit.v1.schema.json"));
    let instance = serde_json::to_value(ErrorMessage::new(
        Some("req-1".to_string()),
        "invalid_config",
        "cadence_hot_ms must be between 5000 and 300000",
    ))
    .expect("serialize error");

    validate(&schema, &instance);
}
