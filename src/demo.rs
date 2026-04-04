use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use chrono::{Duration, TimeZone, Utc};
use serde::Serialize;

use clawgs::emit::engine::EmitEngine;
use clawgs::emit::model_client::{ModelBackend, ModelClient};
use clawgs::emit::protocol::{
    HelloMessage, RestState, SessionSnapshot, SessionState, SyncRequest, SyncResultMessage,
    ThoughtConfig, ThoughtSource, ThoughtState,
};
use clawgs::{extract, AgentTool, ExtractOptions, ExtractOutput};

const CLAUDE_SAMPLE_PATH: &str = "embedded:examples/demo/claude-sample.jsonl";
const CODEX_SAMPLE_PATH: &str = "embedded:examples/demo/codex-sample.jsonl";
const CLAUDE_SAMPLE_INPUT: &str = include_str!("../examples/demo/claude-sample.jsonl");
const CODEX_SAMPLE_INPUT: &str = include_str!("../examples/demo/codex-sample.jsonl");
const DEMO_STREAM_ID: &str = "demo-stream";
const DEMO_THOUGHT: &str = "Turning raw transcripts into stable session state";

#[derive(Debug, Clone, Serialize)]
pub struct ExtractDemoOutput {
    pub demo: &'static str,
    pub tool: String,
    pub input_path: &'static str,
    pub input_jsonl: &'static str,
    pub output: ExtractOutput,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmitDemoOutput {
    pub demo: &'static str,
    pub hello: HelloMessage,
    pub request: SyncRequest,
    pub response: SyncResultMessage,
}

pub fn build_extract_demo(tool: AgentTool, options: ExtractOptions) -> Result<ExtractDemoOutput> {
    let sample = embedded_extract_sample(tool);
    let temp_path = write_demo_temp_file(sample.file_name, sample.input)?;
    let output = extract(tool, &temp_path, Path::new(sample.cwd), false, &options);
    let _ = fs::remove_file(&temp_path);

    let mut output = output?;
    output.source.path = sample.input_path.to_string();
    output.source.cwd = sample.cwd.to_string();
    output.source.discovered = false;

    Ok(ExtractDemoOutput {
        demo: "extract",
        tool: tool.as_str().to_string(),
        input_path: sample.input_path,
        input_jsonl: sample.input,
        output,
    })
}

pub fn build_emit_demo() -> EmitDemoOutput {
    let request = demo_sync_request();
    let mut engine = EmitEngine::with_backend(
        Box::new(DemoModelClient {
            response: DEMO_THOUGHT.to_string(),
        }),
        ModelBackend::OpenRouter,
    );
    let mut response = engine.sync(&request);
    normalize_demo_stream_ids(&mut response);

    EmitDemoOutput {
        demo: "emit",
        hello: HelloMessage::new(),
        request,
        response,
    }
}

struct EmbeddedExtractSample {
    file_name: &'static str,
    input_path: &'static str,
    input: &'static str,
    cwd: &'static str,
}

fn embedded_extract_sample(tool: AgentTool) -> EmbeddedExtractSample {
    match tool {
        AgentTool::Claude => EmbeddedExtractSample {
            file_name: "claude-sample.jsonl",
            input_path: CLAUDE_SAMPLE_PATH,
            input: CLAUDE_SAMPLE_INPUT,
            cwd: "/demo/claude-project",
        },
        AgentTool::Codex => EmbeddedExtractSample {
            file_name: "codex-sample.jsonl",
            input_path: CODEX_SAMPLE_PATH,
            input: CODEX_SAMPLE_INPUT,
            cwd: "/demo/codex-project",
        },
    }
}

fn write_demo_temp_file(file_name: &str, input: &str) -> Result<PathBuf> {
    let stamp = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(format!("clawgs-demo-{stamp}-{file_name}"));
    fs::write(&path, input).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn demo_sync_request() -> SyncRequest {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 1, 15, 30, 0)
        .single()
        .expect("valid demo timestamp");

    SyncRequest::new(
        "demo-sync-1",
        now,
        ThoughtConfig::default(),
        vec![SessionSnapshot {
            session_id: "demo:codex:1".to_string(),
            state: SessionState::Busy,
            exited: false,
            tool: Some("codex".to_string()),
            cwd: "/demo/project-a".to_string(),
            replay_text: concat!(
                "cargo test --all\n",
                "test parsers::codex::parse_extracts_task_function_call_and_tokens ... ok\n",
                "test emit::protocol::sync_request_serializes_expected_wire_shape ... ok\n",
                "reviewing whether raw agent transcripts can collapse into a stable JSON snapshot\n",
                "writing a public demo so new users can see the value without private logs\n",
            )
            .to_string(),
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            objective_fingerprint: None,
            thought_updated_at: None,
            token_count: 144_379,
            context_limit: 256_000,
            last_activity_at: now - Duration::seconds(2),
            rest_state: RestState::Active,
            commit_candidate: false,
        }],
    )
}

fn normalize_demo_stream_ids(response: &mut SyncResultMessage) {
    response.stream_instance_id = DEMO_STREAM_ID.to_string();
    for update in &mut response.updates {
        update.stream_instance_id = Some(DEMO_STREAM_ID.to_string());
    }
}

struct DemoModelClient {
    response: String,
}

impl ModelClient for DemoModelClient {
    fn complete(&self, _prompt: &str, _model_override: Option<&str>) -> Result<String, String> {
        Ok(self.response.clone())
    }
}
