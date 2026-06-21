//! Wire types for the `clawgs.emit.v2` NDJSON protocol (hello, sync, sync\_result).

use std::error::Error;
use std::fmt::{Display, Formatter};

use chrono::{DateTime, Utc};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ActionCue;

pub const HELLO_MESSAGE_TYPE: &str = "hello";
pub const SYNC_MESSAGE_TYPE: &str = "sync";
pub const SYNC_RESULT_MESSAGE_TYPE: &str = "sync_result";
pub const SYNC_RESPONSE_MESSAGE_TYPE: &str = SYNC_RESULT_MESSAGE_TYPE;
pub const EMIT_PROTOCOL_VERSION: &str = "clawgs.emit.v2";

pub const CADENCE_HOT_MIN_MS: u64 = 5_000;
pub const CADENCE_HOT_MAX_MS: u64 = 300_000;
pub const CADENCE_WARM_MAX_MS: u64 = 600_000;
pub const CADENCE_COLD_MAX_MS: u64 = 1_800_000;
pub const MODEL_MAX_CHARS: usize = 200;
pub const PROMPT_MAX_CHARS: usize = 4_000;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Busy,
    Error,
    Attention,
    Exited,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtState {
    Active,
    #[default]
    Holding,
    Sleeping,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtSource {
    #[default]
    CarryForward,
    Llm,
    StaticSleeping,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestState {
    #[default]
    Active,
    Drowsy,
    Sleeping,
    DeepSleep,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BubblePrecedence {
    #[default]
    ThoughtFirst,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadenceTier {
    Hot,
    Warm,
    Cold,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    Transcript,
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThoughtConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default = "default_cadence_hot_ms")]
    pub cadence_hot_ms: u64,
    #[serde(default = "default_cadence_warm_ms")]
    pub cadence_warm_ms: u64,
    #[serde(default = "default_cadence_cold_ms")]
    pub cadence_cold_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_prompt: Option<String>,
}

impl Default for ThoughtConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            model: String::new(),
            backend: String::new(),
            cadence_hot_ms: default_cadence_hot_ms(),
            cadence_warm_ms: default_cadence_warm_ms(),
            cadence_cold_ms: default_cadence_cold_ms(),
            agent_prompt: None,
            terminal_prompt: None,
        }
    }
}

impl ThoughtConfig {
    pub fn normalize(&mut self) {
        self.model = self.model.trim().to_string();
        self.backend = self.backend.trim().to_string();
        self.agent_prompt = normalize_optional_prompt(self.agent_prompt.take());
        self.terminal_prompt = normalize_optional_prompt(self.terminal_prompt.take());
    }

    pub fn validate(&self) -> Result<(), ThoughtConfigValidationError> {
        validate_model_field(&self.model)?;
        validate_backend_field(&self.backend)?;
        self.validate_cadences()?;
        validate_optional_len("agent_prompt", self.agent_prompt.as_deref())?;
        validate_optional_len("terminal_prompt", self.terminal_prompt.as_deref())?;
        Ok(())
    }

    fn validate_cadences(&self) -> Result<(), ThoughtConfigValidationError> {
        validate_cadence_range(
            "cadence_hot_ms",
            self.cadence_hot_ms,
            CADENCE_HOT_MIN_MS,
            CADENCE_HOT_MAX_MS,
            &CADENCE_HOT_MIN_MS.to_string(),
        )?;
        validate_cadence_range(
            "cadence_warm_ms",
            self.cadence_warm_ms,
            self.cadence_hot_ms,
            CADENCE_WARM_MAX_MS,
            &format!("cadence_hot_ms ({})", self.cadence_hot_ms),
        )?;
        validate_cadence_range(
            "cadence_cold_ms",
            self.cadence_cold_ms,
            self.cadence_warm_ms,
            CADENCE_COLD_MAX_MS,
            &format!("cadence_warm_ms ({})", self.cadence_warm_ms),
        )?;
        Ok(())
    }

    pub fn normalize_and_validate(mut self) -> Result<Self, ThoughtConfigValidationError> {
        self.normalize();
        self.validate()?;
        Ok(self)
    }

    pub fn model_override(&self) -> Option<&str> {
        let trimmed = self.model.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    pub fn backend_override(&self) -> Option<crate::emit::model_client::ModelBackend> {
        let trimmed = self.backend.trim();
        if trimmed.is_empty() {
            None
        } else {
            crate::emit::model_client::ModelBackend::from_env_value(trimmed)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThoughtConfigValidationError {
    pub field: &'static str,
    pub message: String,
}

impl ThoughtConfigValidationError {
    fn new(field: &'static str, message: String) -> Self {
        Self { field, message }
    }
}

impl Display for ThoughtConfigValidationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.field, self.message)
    }
}

impl Error for ThoughtConfigValidationError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub state: SessionState,
    pub exited: bool,
    pub tool: Option<String>,
    pub cwd: String,
    pub replay_text: String,
    pub thought: Option<String>,
    #[serde(default)]
    pub thought_state: ThoughtState,
    #[serde(default)]
    pub thought_source: ThoughtSource,
    pub objective_fingerprint: Option<String>,
    pub thought_updated_at: Option<DateTime<Utc>>,
    pub token_count: u64,
    pub context_limit: u64,
    pub last_activity_at: DateTime<Utc>,
    #[serde(default)]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_cues: Vec<ActionCue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncRequest {
    pub id: String,
    pub now: DateTime<Utc>,
    pub config: ThoughtConfig,
    pub sessions: Vec<SessionSnapshot>,
}

impl SyncRequest {
    pub fn new(
        id: impl Into<String>,
        now: DateTime<Utc>,
        config: ThoughtConfig,
        sessions: Vec<SessionSnapshot>,
    ) -> Self {
        Self {
            id: id.into(),
            now,
            config,
            sessions,
        }
    }

    pub fn with_generated_now(
        id: impl Into<String>,
        config: ThoughtConfig,
        sessions: Vec<SessionSnapshot>,
    ) -> Self {
        Self::new(id, Utc::now(), config, sessions)
    }
}

impl Serialize for SyncRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("SyncRequest", 5)?;
        state.serialize_field("type", SYNC_MESSAGE_TYPE)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("now", &self.now)?;
        state.serialize_field("config", &SyncRequestConfigWireRef::from(&self.config))?;
        state.serialize_field("sessions", &self.sessions)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for SyncRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SyncRequestWire::deserialize(deserializer)?;
        if wire.message_type != SYNC_MESSAGE_TYPE {
            return Err(serde::de::Error::custom(format!(
                "expected sync request type {SYNC_MESSAGE_TYPE:?}, got {:?}",
                wire.message_type
            )));
        }
        Ok(Self {
            id: wire.id,
            now: wire.now,
            config: wire.config.into_runtime_config(),
            sessions: wire.sessions,
        })
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SyncRequestWire {
    #[serde(rename = "type")]
    message_type: String,
    id: String,
    now: DateTime<Utc>,
    config: SyncRequestConfigWire,
    sessions: Vec<SessionSnapshot>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SyncRequestConfigWire {
    enabled: bool,
    model: String,
    #[serde(default)]
    backend: String,
    cadence_hot_ms: u64,
    cadence_warm_ms: u64,
    cadence_cold_ms: u64,
    #[serde(default, deserialize_with = "deserialize_wire_prompt_string")]
    agent_prompt: String,
    #[serde(default, deserialize_with = "deserialize_wire_prompt_string")]
    terminal_prompt: String,
}

impl SyncRequestConfigWire {
    fn into_runtime_config(self) -> ThoughtConfig {
        let mut config = ThoughtConfig {
            enabled: self.enabled,
            model: self.model,
            backend: self.backend,
            cadence_hot_ms: self.cadence_hot_ms,
            cadence_warm_ms: self.cadence_warm_ms,
            cadence_cold_ms: self.cadence_cold_ms,
            agent_prompt: string_to_optional_prompt(self.agent_prompt),
            terminal_prompt: string_to_optional_prompt(self.terminal_prompt),
        };
        config.normalize();
        config
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct SyncRequestConfigWireRef<'a> {
    enabled: bool,
    model: &'a str,
    backend: &'a str,
    cadence_hot_ms: u64,
    cadence_warm_ms: u64,
    cadence_cold_ms: u64,
    agent_prompt: &'a str,
    terminal_prompt: &'a str,
}

impl<'a> From<&'a ThoughtConfig> for SyncRequestConfigWireRef<'a> {
    fn from(value: &'a ThoughtConfig) -> Self {
        Self {
            enabled: value.enabled,
            model: value.model.as_str(),
            backend: value.backend.as_str(),
            cadence_hot_ms: value.cadence_hot_ms,
            cadence_warm_ms: value.cadence_warm_ms,
            cadence_cold_ms: value.cadence_cold_ms,
            agent_prompt: value.agent_prompt.as_deref().unwrap_or_default(),
            terminal_prompt: value.terminal_prompt.as_deref().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncUpdate {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    #[serde(default)]
    pub thought_state: ThoughtState,
    #[serde(default)]
    pub thought_source: ThoughtSource,
    #[serde(default)]
    pub objective_changed: bool,
    #[serde(default)]
    pub bubble_precedence: BubblePrecedence,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective_fingerprint: Option<String>,
    #[serde(default)]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_cues: Vec<ActionCue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<TimingInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cues: Option<CueInfo>,
}

pub type ThoughtUpdate = SyncUpdate;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionDeltaKind {
    Started,
    Changed,
    Unchanged,
    Exited,
    Removed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionDeltaField {
    State,
    Exited,
    Tool,
    Cwd,
    ReplayText,
    Activity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDelta {
    pub session_id: String,
    pub kind: SessionDeltaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<SessionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_fields: Vec<SessionDeltaField>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub transcript_ambiguous: bool,
}

impl SessionDelta {
    pub(crate) fn removed(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            kind: SessionDeltaKind::Removed,
            state: None,
            tool: None,
            cwd: None,
            changed_fields: Vec::new(),
            transcript_ambiguous: false,
        }
    }

    pub(crate) fn observed(
        session: &SessionSnapshot,
        kind: SessionDeltaKind,
        changed_fields: Vec<SessionDeltaField>,
        transcript_ambiguous: bool,
    ) -> Self {
        Self {
            session_id: session.session_id.clone(),
            kind,
            state: Some(session.state),
            tool: session.tool.clone(),
            cwd: Some(session.cwd.clone()),
            changed_fields,
            transcript_ambiguous,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimingInfo {
    pub run_started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_finished_at: Option<DateTime<Utc>>,
    pub run_elapsed_ms: u64,
    pub idle_elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CueInfo {
    pub cadence_tier: CadenceTier,
    pub cadence_ms: u64,
    pub next_llm_eligible_at: DateTime<Utc>,
    pub context_source: ContextSource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncMetrics {
    pub sessions_seen: u64,
    pub llm_calls: u64,
    pub suppressed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_backend_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub protocol: String,
    pub engine_version: String,
}

impl HelloMessage {
    pub fn new() -> Self {
        Self {
            msg_type: HELLO_MESSAGE_TYPE.to_string(),
            protocol: EMIT_PROTOCOL_VERSION.to_string(),
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl Default for HelloMessage {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncResultMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub id: String,
    pub stream_instance_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_deltas: Vec<SessionDelta>,
    pub updates: Vec<ThoughtUpdate>,
    pub metrics: SyncMetrics,
}

impl SyncResultMessage {
    pub fn new(
        id: impl Into<String>,
        stream_instance_id: impl Into<String>,
        updates: Vec<ThoughtUpdate>,
        metrics: SyncMetrics,
    ) -> Self {
        Self {
            msg_type: SYNC_RESULT_MESSAGE_TYPE,
            id: id.into(),
            stream_instance_id: stream_instance_id.into(),
            session_deltas: Vec::new(),
            updates,
            metrics,
        }
    }

    pub fn with_session_deltas(mut self, session_deltas: Vec<SessionDelta>) -> Self {
        self.session_deltas = session_deltas;
        self
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ErrorMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub code: String,
    pub message: String,
}

impl ErrorMessage {
    pub fn new(id: Option<String>, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            msg_type: "error",
            id,
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncResponse {
    pub request_id: String,
    pub stream_instance_id: Option<String>,
    pub updates: Vec<SyncUpdate>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum WireRequestId {
    Numeric(u64),
    Stringy(String),
}

fn parse_wire_request_id(value: WireRequestId) -> String {
    match value {
        WireRequestId::Numeric(v) => v.to_string(),
        WireRequestId::Stringy(v) => v,
    }
}

fn deserialize_request_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = WireRequestId::deserialize(deserializer)?;
    Ok(parse_wire_request_id(raw))
}

fn deserialize_optional_request_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<WireRequestId>::deserialize(deserializer)?;
    Ok(raw.map(parse_wire_request_id))
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonInboundMessage {
    Hello {
        protocol: String,
    },
    #[serde(rename = "sync_result", alias = "sync_response")]
    SyncResponse {
        #[serde(
            rename = "id",
            alias = "request_id",
            deserialize_with = "deserialize_request_id"
        )]
        request_id: String,
        #[serde(default)]
        stream_instance_id: Option<String>,
        #[serde(default)]
        updates: Vec<SyncUpdate>,
    },
    Error {
        code: String,
        message: String,
        #[serde(
            default,
            rename = "id",
            alias = "request_id",
            deserialize_with = "deserialize_optional_request_id"
        )]
        request_id: Option<String>,
    },
}

impl DaemonInboundMessage {
    pub fn message_type(&self) -> &'static str {
        match self {
            Self::Hello { .. } => HELLO_MESSAGE_TYPE,
            Self::SyncResponse { .. } => SYNC_RESPONSE_MESSAGE_TYPE,
            Self::Error { .. } => "error",
        }
    }
}

fn validate_optional_len(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), ThoughtConfigValidationError> {
    if value.map(|value| value.chars().count()).unwrap_or_default() > PROMPT_MAX_CHARS {
        return Err(ThoughtConfigValidationError::new(
            field,
            format!("must be <= {PROMPT_MAX_CHARS} characters"),
        ));
    }
    Ok(())
}

fn validate_model_field(model: &str) -> Result<(), ThoughtConfigValidationError> {
    if model.chars().count() > MODEL_MAX_CHARS {
        return Err(ThoughtConfigValidationError::new(
            "model",
            format!("must be <= {MODEL_MAX_CHARS} characters"),
        ));
    }
    Ok(())
}

fn validate_backend_field(backend: &str) -> Result<(), ThoughtConfigValidationError> {
    if backend.is_empty() {
        return Ok(());
    }
    use crate::emit::model_client::ModelBackend;
    if ModelBackend::from_env_value(backend).is_none() {
        return Err(ThoughtConfigValidationError::new(
            "backend",
            format!(
                "unrecognized backend {backend:?}; expected one of: openrouter, grok (legacy claude/codex aliases accepted)"
            ),
        ));
    }
    Ok(())
}

fn validate_cadence_range(
    field: &'static str,
    value: u64,
    min: u64,
    max: u64,
    min_label: &str,
) -> Result<(), ThoughtConfigValidationError> {
    if !(min..=max).contains(&value) {
        return Err(ThoughtConfigValidationError::new(
            field,
            format!("must be between {min_label} and {max} (inclusive)"),
        ));
    }
    Ok(())
}

fn normalize_optional_prompt(value: Option<String>) -> Option<String> {
    value
        .map(|prompt| prompt.trim().to_string())
        .filter(|prompt| !prompt.is_empty())
}

fn string_to_optional_prompt(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn deserialize_wire_prompt_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

const fn default_enabled() -> bool {
    true
}

const fn default_cadence_hot_ms() -> u64 {
    15_000
}

const fn default_cadence_warm_ms() -> u64 {
    45_000
}

const fn default_cadence_cold_ms() -> u64 {
    120_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session() -> SessionSnapshot {
        let now = Utc::now();
        SessionSnapshot {
            session_id: "sess-1".to_string(),
            state: SessionState::Busy,
            exited: false,
            tool: Some("Codex".to_string()),
            cwd: "/tmp".to_string(),
            replay_text: "cargo test".to_string(),
            thought: Some("Running tests".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            rest_state: RestState::Drowsy,
            objective_fingerprint: Some("obj-1".to_string()),
            thought_updated_at: Some(now),
            token_count: 12,
            context_limit: 100,
            last_activity_at: now,
            commit_candidate: false,
            action_cues: Vec::new(),
        }
    }

    #[test]
    fn daemon_inbound_message_type_returns_canonical_strings() {
        let hello = DaemonInboundMessage::Hello {
            protocol: "clawgs.emit.v2".to_string(),
        };
        assert_eq!(hello.message_type(), HELLO_MESSAGE_TYPE);

        let sync = DaemonInboundMessage::SyncResponse {
            request_id: "req-1".to_string(),
            stream_instance_id: None,
            updates: Vec::new(),
        };
        assert_eq!(sync.message_type(), SYNC_RESPONSE_MESSAGE_TYPE);

        let error = DaemonInboundMessage::Error {
            code: "boom".to_string(),
            message: "kapow".to_string(),
            request_id: None,
        };
        assert_eq!(error.message_type(), "error");
    }

    #[test]
    fn config_validation_accepts_defaults() {
        let cfg = ThoughtConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn normalize_converts_empty_prompts_to_none() {
        let mut cfg = ThoughtConfig {
            agent_prompt: Some(String::new()),
            terminal_prompt: Some(String::new()),
            ..ThoughtConfig::default()
        };

        cfg.normalize();
        assert!(cfg.agent_prompt.is_none());
        assert!(cfg.terminal_prompt.is_none());
    }

    #[test]
    fn normalize_treats_whitespace_only_prompts_as_empty() {
        let mut cfg = ThoughtConfig {
            agent_prompt: Some("   ".to_string()),
            terminal_prompt: Some("\t\n  ".to_string()),
            ..ThoughtConfig::default()
        };

        cfg.normalize();
        assert!(cfg.agent_prompt.is_none());
        assert!(cfg.terminal_prompt.is_none());
    }

    #[test]
    fn normalize_trims_surrounding_whitespace_on_prompts() {
        let mut cfg = ThoughtConfig {
            agent_prompt: Some("  you are a status reporter  ".to_string()),
            ..ThoughtConfig::default()
        };

        cfg.normalize();
        assert_eq!(
            cfg.agent_prompt.as_deref(),
            Some("you are a status reporter")
        );
    }

    #[test]
    fn hot_cadence_must_be_in_range() {
        let cfg = ThoughtConfig {
            cadence_hot_ms: CADENCE_HOT_MIN_MS - 1,
            ..ThoughtConfig::default()
        };

        let err = cfg
            .validate()
            .expect_err("hot cadence below range should fail");
        assert_eq!(err.field, "cadence_hot_ms");
    }

    #[test]
    fn warm_cadence_must_be_at_least_hot() {
        let cfg = ThoughtConfig {
            cadence_hot_ms: 10_000,
            cadence_warm_ms: 9_999,
            ..ThoughtConfig::default()
        };

        let err = cfg
            .validate()
            .expect_err("warm cadence below hot cadence should fail");
        assert_eq!(err.field, "cadence_warm_ms");
    }

    #[test]
    fn cold_cadence_must_be_at_least_warm() {
        let cfg = ThoughtConfig {
            cadence_warm_ms: 50_000,
            cadence_cold_ms: 49_000,
            ..ThoughtConfig::default()
        };

        let err = cfg
            .validate()
            .expect_err("cold cadence below warm cadence should fail");
        assert_eq!(err.field, "cadence_cold_ms");
    }

    #[test]
    fn sync_request_serializes_expected_wire_shape() {
        let request =
            SyncRequest::with_generated_now("7", ThoughtConfig::default(), vec![sample_session()]);
        let json = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(json["type"], SYNC_MESSAGE_TYPE);
        assert_eq!(json["id"], "7");
        assert!(chrono::DateTime::parse_from_rfc3339(
            json["now"]
                .as_str()
                .expect("now should be an RFC3339 string")
        )
        .is_ok());
        assert_eq!(json["config"]["enabled"], true);
        assert_eq!(json["config"]["model"], "");
        assert_eq!(json["config"]["backend"], "");
        assert_eq!(json["config"]["cadence_hot_ms"], 15_000);
        assert_eq!(json["config"]["cadence_warm_ms"], 45_000);
        assert_eq!(json["config"]["cadence_cold_ms"], 120_000);
        assert_eq!(json["config"]["agent_prompt"], "");
        assert_eq!(json["config"]["terminal_prompt"], "");
        assert_eq!(json["sessions"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["sessions"][0]["session_id"], "sess-1");
        assert_eq!(json["sessions"][0]["state"], "busy");
    }

    #[test]
    fn sync_request_deserializes_null_prompt_fields_to_none() {
        let raw = serde_json::json!({
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
        .to_string();

        let request: SyncRequest = serde_json::from_str(&raw).expect("sync request should parse");
        assert_eq!(request.id, "req-1");
        assert!(request.config.agent_prompt.is_none());
        assert!(request.config.terminal_prompt.is_none());
        assert!(request.config.backend.is_empty());
    }

    #[test]
    fn sync_request_deserializes_missing_backend_to_empty() {
        let raw = serde_json::json!({
            "type": "sync",
            "id": "req-2",
            "now": "2026-02-26T21:00:00Z",
            "config": {
                "enabled": true,
                "model": "",
                "cadence_hot_ms": 15000,
                "cadence_warm_ms": 45000,
                "cadence_cold_ms": 120000
            },
            "sessions": []
        })
        .to_string();

        let request: SyncRequest = serde_json::from_str(&raw).expect("sync request should parse");
        assert!(request.config.backend.is_empty());
        assert!(request.config.backend_override().is_none());
    }

    #[test]
    fn sync_request_rejects_missing_or_wrong_message_type() {
        let missing_type = serde_json::json!({
            "id": "req-missing",
            "now": "2026-02-26T21:00:00Z",
            "config": {
                "enabled": true,
                "model": "",
                "cadence_hot_ms": 15000,
                "cadence_warm_ms": 45000,
                "cadence_cold_ms": 120000
            },
            "sessions": []
        })
        .to_string();
        assert!(
            serde_json::from_str::<SyncRequest>(&missing_type).is_err(),
            "type is required by clawgs.emit.v2"
        );

        let wrong_type = serde_json::json!({
            "type": "sync_result",
            "id": "req-wrong",
            "now": "2026-02-26T21:00:00Z",
            "config": {
                "enabled": true,
                "model": "",
                "cadence_hot_ms": 15000,
                "cadence_warm_ms": 45000,
                "cadence_cold_ms": 120000
            },
            "sessions": []
        })
        .to_string();
        let err =
            serde_json::from_str::<SyncRequest>(&wrong_type).expect_err("wrong type must fail");
        assert!(err.to_string().contains("expected sync request type"));
    }

    #[test]
    fn sync_request_rejects_missing_sessions() {
        let raw = serde_json::json!({
            "type": "sync",
            "id": "req-missing-sessions",
            "now": "2026-02-26T21:00:00Z",
            "config": {
                "enabled": true,
                "model": "",
                "cadence_hot_ms": 15000,
                "cadence_warm_ms": 45000,
                "cadence_cold_ms": 120000
            }
        })
        .to_string();

        let err = serde_json::from_str::<SyncRequest>(&raw)
            .expect_err("sessions is required by clawgs.emit.v2");
        assert!(err.to_string().contains("missing field `sessions`"));
    }

    #[test]
    fn sync_request_rejects_unknown_fields_from_closed_schema_objects() {
        let base = serde_json::json!({
            "type": "sync",
            "id": "req-extra",
            "now": "2026-02-26T21:00:00Z",
            "config": {
                "enabled": true,
                "model": "",
                "cadence_hot_ms": 15000,
                "cadence_warm_ms": 45000,
                "cadence_cold_ms": 120000
            },
            "sessions": [serde_json::to_value(sample_session()).expect("session json")]
        });

        let mut top_level_extra = base.clone();
        top_level_extra["extra"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<SyncRequest>(top_level_extra).is_err(),
            "sync request is closed by the v2 schema"
        );

        let mut config_extra = base.clone();
        config_extra["config"]["extra"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<SyncRequest>(config_extra).is_err(),
            "sync config is closed by the v2 schema"
        );

        let mut session_extra = base;
        session_extra["sessions"][0]["extra"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<SyncRequest>(session_extra).is_err(),
            "session snapshot is closed by the v2 schema"
        );
    }

    #[test]
    fn config_validation_rejects_unknown_backend() {
        let cfg = ThoughtConfig {
            backend: "gemini".to_string(),
            ..ThoughtConfig::default()
        };
        let err = cfg.validate().expect_err("unknown backend should fail");
        assert_eq!(err.field, "backend");
        assert!(err.message.contains("gemini"));
    }

    #[test]
    fn config_validation_accepts_known_backends() {
        for backend in ["openrouter", "grok", "grok_cli", "claude", "codex", ""] {
            let cfg = ThoughtConfig {
                backend: backend.to_string(),
                ..ThoughtConfig::default()
            };
            assert!(
                cfg.validate().is_ok(),
                "backend {:?} should be valid",
                backend
            );
        }
    }

    #[test]
    fn sync_update_serializes_timing_and_cues() {
        let now = Utc::now();
        let update = SyncUpdate {
            session_id: "sess-1".to_string(),
            stream_instance_id: Some("stream-1".to_string()),
            emission_seq: Some(3),
            thought: Some("Reviewing auth fallback".to_string()),
            token_count: 12,
            context_limit: 100,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::Llm,
            objective_changed: false,
            bubble_precedence: BubblePrecedence::ThoughtFirst,
            at: now,
            objective_fingerprint: Some("obj-1".to_string()),
            rest_state: RestState::Active,
            commit_candidate: false,
            action_cues: Vec::new(),
            timing: Some(TimingInfo {
                run_started_at: now,
                run_finished_at: None,
                run_elapsed_ms: 0,
                idle_elapsed_ms: 0,
            }),
            cues: Some(CueInfo {
                cadence_tier: CadenceTier::Hot,
                cadence_ms: 15_000,
                next_llm_eligible_at: now,
                context_source: ContextSource::Transcript,
            }),
        };

        let json = serde_json::to_value(&update).expect("update should serialize");
        assert_eq!(json["timing"]["run_elapsed_ms"], 0);
        assert_eq!(json["cues"]["cadence_tier"], "hot");
        assert_eq!(json["cues"]["context_source"], "transcript");
    }

    #[test]
    fn sync_result_serializes_session_deltas_when_present() {
        let result =
            SyncResultMessage::new("req-1", "stream-1", Vec::new(), SyncMetrics::default())
                .with_session_deltas(vec![SessionDelta {
                    session_id: "sess-1".to_string(),
                    kind: SessionDeltaKind::Changed,
                    state: Some(SessionState::Busy),
                    tool: Some("codex".to_string()),
                    cwd: Some("/tmp/project".to_string()),
                    changed_fields: vec![
                        SessionDeltaField::ReplayText,
                        SessionDeltaField::Activity,
                    ],
                    transcript_ambiguous: true,
                }]);

        let json = serde_json::to_value(&result).expect("result should serialize");
        assert_eq!(json["session_deltas"][0]["kind"], "changed");
        assert_eq!(
            json["session_deltas"][0]["changed_fields"][0],
            "replay_text"
        );
        assert_eq!(json["session_deltas"][0]["transcript_ambiguous"], true);
    }

    #[test]
    fn inbound_message_deserializes_legacy_sync_response_alias() {
        let raw = r#"{
            "type": "sync_response",
            "request_id": 17,
            "updates": []
        }"#;
        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("legacy sync_response alias should deserialize");

        assert!(matches!(message, DaemonInboundMessage::SyncResponse { .. }));

        if let DaemonInboundMessage::SyncResponse {
            request_id,
            stream_instance_id,
            updates,
        } = message
        {
            assert_eq!(request_id, "17");
            assert_eq!(stream_instance_id.as_deref(), None);
            assert!(updates.is_empty());
            assert_eq!(SYNC_RESPONSE_MESSAGE_TYPE, "sync_result");
        }
    }

    #[test]
    fn inbound_message_rejects_sync_request_envelope_as_response() {
        let raw = r#"{
            "type": "sync",
            "id": "req-1",
            "now": "2026-02-26T21:00:00Z",
            "config": {
                "enabled": true,
                "model": "",
                "cadence_hot_ms": 15000,
                "cadence_warm_ms": 45000,
                "cadence_cold_ms": 120000
            },
            "sessions": []
        }"#;
        assert!(
            serde_json::from_str::<DaemonInboundMessage>(raw).is_err(),
            "daemon output parser must not treat sync requests as responses"
        );
    }

    #[test]
    fn hello_roundtrip_is_stable() {
        let hello = HelloMessage::new();
        let encoded = serde_json::to_string(&hello).expect("hello should serialize");
        let decoded: HelloMessage = serde_json::from_str(&encoded).expect("hello should parse");
        assert_eq!(decoded, hello);
    }

    #[test]
    fn thoughtless_update_roundtrips_through_inbound_message() {
        // The engine emits updates with no `thought` (clear/passive updates).
        // Because `thought` is `skip_serializing_if = "Option::is_none"`, the
        // serialized JSON omits the key entirely. A consumer parsing the
        // daemon's stdout via `DaemonInboundMessage::SyncResponse` (whose
        // `updates: Vec<SyncUpdate>`) must be able to deserialize that — i.e.
        // `thought` must default to None when absent, matching the committed
        // schema, which lists `thought` as not-required.
        let now = Utc::now();
        let update = SyncUpdate {
            session_id: "sess-1".to_string(),
            stream_instance_id: None,
            emission_seq: Some(1),
            thought: None,
            token_count: 0,
            context_limit: 0,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            objective_changed: false,
            bubble_precedence: BubblePrecedence::ThoughtFirst,
            at: now,
            objective_fingerprint: None,
            rest_state: RestState::Active,
            commit_candidate: false,
            action_cues: Vec::new(),
            timing: None,
            cues: None,
        };
        let encoded = serde_json::to_value(&update).expect("update should serialize");
        assert!(
            encoded.get("thought").is_none(),
            "thoughtless update must omit the thought key"
        );

        let inbound = serde_json::json!({
            "type": "sync_result",
            "id": "req-1",
            "updates": [encoded],
        })
        .to_string();
        let parsed: DaemonInboundMessage = serde_json::from_str(&inbound)
            .expect("daemon consumers must parse thoughtless updates");
        match parsed {
            DaemonInboundMessage::SyncResponse { updates, .. } => {
                assert_eq!(updates.len(), 1);
                assert!(updates[0].thought.is_none());
            }
            other => panic!("expected sync_result, got {other:?}"),
        }
    }
}
