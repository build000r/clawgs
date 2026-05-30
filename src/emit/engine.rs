// The emit engine is a state machine whose small helper functions pass the
// current session, runtime state, request config, and output buffers together.
// Grouping these into broad mutable context structs makes ownership less clear
// than the explicit arguments, so keep the signatures explicit.
#![allow(clippy::too_many_arguments)]

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration as ChronoDuration, Utc};

use crate::{
    discover_claude_paths, discover_codex_paths, extract, resolve_input, Action, ActionCue,
    ActionCueKind, AgentTool, ExtractOptions, Snapshot, ToolSelection,
};

use super::model_client::{
    build_model_client_for, validate_backend_credentials, ModelBackend, ModelClient,
};
use super::protocol::{
    BubblePrecedence, CadenceTier, ContextSource, CueInfo, RestState, SessionDelta,
    SessionDeltaField, SessionDeltaKind, SessionSnapshot, SessionState, SyncMetrics, SyncRequest,
    SyncResultMessage, ThoughtConfig, ThoughtSource, ThoughtState, ThoughtUpdate, TimingInfo,
};

const SUMMARY_HISTORY_CAP: usize = 10;
const TERMINAL_CONTEXT_CHARS: usize = 800;
const TERMINAL_MIN_MEANINGFUL_DELTA_CHARS: usize = 100;
const MAX_THOUGHT_CHARS: usize = 120;
const DROWSY_AFTER_MS: i64 = 10_000;

pub const DEFAULT_AGENT_PREAMBLE: &str = "You are a status reporter for a coding agent session.";
pub const DEFAULT_TERMINAL_PREAMBLE: &str = "Terminal session status reporter.";

struct BackendInitError {
    message: String,
    /// True when the failure was a missing credential. Sessions still get
    /// rest-state bookkeeping so the daemon can carry forward without LLM
    /// calls. False for hard construction failures, which return immediately.
    allow_carry_forward: bool,
}

/// Decides whether a session has visible state that must be wiped (thought,
/// non-holding state, mismatched rest state, or commit-candidate flag).
/// Pulled out of `clear_all_sessions` so the branch list does not inflate CC.
fn session_needs_clear(
    state: &SessionRuntimeState,
    session: &SessionSnapshot,
    next_rest_state: RestState,
) -> bool {
    runtime_state_needs_clear(state, next_rest_state) || session_carries_dirty_signal(session)
}

fn runtime_state_needs_clear(state: &SessionRuntimeState, next_rest_state: RestState) -> bool {
    state.last_emitted_thought.is_some()
        || state.thought_state != ThoughtState::Holding
        || state.rest_state != next_rest_state
        || state.commit_candidate
        || !state.action_cues.is_empty()
}

fn session_carries_dirty_signal(session: &SessionSnapshot) -> bool {
    session.thought.is_some() || session.commit_candidate || !session.action_cues.is_empty()
}

/// Tally `(tool, cwd)` group occurrences across the session list. Returned
/// empty for ≤ 1 sessions because ambiguity requires at least two snapshots
/// sharing a key — a hot path for swimmers' single-pane sync ticks.
fn compute_transcript_group_counts(sessions: &[SessionSnapshot]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    if sessions.len() > 1 {
        for session in sessions {
            if let Some(group_key) = transcript_group_key(session) {
                *counts.entry(group_key).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn compute_session_deltas(
    per_session: &mut HashMap<String, SessionRuntimeState>,
    request: &SyncRequest,
    transcript_group_counts: &HashMap<String, usize>,
) -> Vec<SessionDelta> {
    let active_ids: HashSet<&str> = request
        .sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    let mut removed_ids: Vec<String> = per_session
        .keys()
        .filter(|session_id| !active_ids.contains(session_id.as_str()))
        .cloned()
        .collect();
    removed_ids.sort();

    let mut deltas: Vec<SessionDelta> = removed_ids
        .into_iter()
        .map(|session_id| SessionDelta {
            session_id,
            kind: SessionDeltaKind::Removed,
            state: None,
            tool: None,
            cwd: None,
            changed_fields: Vec::new(),
            transcript_ambiguous: false,
        })
        .collect();

    for session in &request.sessions {
        let state = per_session
            .entry(session.session_id.clone())
            .or_insert_with(|| SessionRuntimeState::initialize_from_session(session, request.now));
        let current = ObservedSessionFacts::from_session(session);
        let previous = state.last_observed.as_ref();
        let changed_fields = session_changed_fields(previous, &current);
        let kind = session_delta_kind(previous, session, &changed_fields);

        deltas.push(SessionDelta {
            session_id: session.session_id.clone(),
            kind,
            state: Some(session.state),
            tool: session.tool.clone(),
            cwd: Some(session.cwd.clone()),
            changed_fields,
            transcript_ambiguous: transcript_group_is_ambiguous(session, transcript_group_counts),
        });
        state.last_observed = Some(current);
    }

    deltas
}

fn session_delta_kind(
    previous: Option<&ObservedSessionFacts>,
    session: &SessionSnapshot,
    changed_fields: &[SessionDeltaField],
) -> SessionDeltaKind {
    if session.exited || session.state == SessionState::Exited {
        SessionDeltaKind::Exited
    } else if previous.is_none() {
        SessionDeltaKind::Started
    } else if changed_fields.is_empty() {
        SessionDeltaKind::Unchanged
    } else {
        SessionDeltaKind::Changed
    }
}

fn session_changed_fields(
    previous: Option<&ObservedSessionFacts>,
    current: &ObservedSessionFacts,
) -> Vec<SessionDeltaField> {
    let Some(previous) = previous else {
        return Vec::new();
    };

    let mut fields = Vec::new();
    if previous.state != current.state {
        fields.push(SessionDeltaField::State);
    }
    if previous.exited != current.exited {
        fields.push(SessionDeltaField::Exited);
    }
    if previous.tool != current.tool {
        fields.push(SessionDeltaField::Tool);
    }
    if previous.cwd != current.cwd {
        fields.push(SessionDeltaField::Cwd);
    }
    if previous.replay_fingerprint != current.replay_fingerprint {
        fields.push(SessionDeltaField::ReplayText);
    }
    if previous.last_activity_at != current.last_activity_at {
        fields.push(SessionDeltaField::Activity);
    }

    fields
}

struct SessionRuntimeState {
    summary_history: Vec<String>,
    last_terminal_context: Option<String>,
    last_call_at: Option<DateTime<Utc>>,
    run_started_at: DateTime<Utc>,
    run_finished_at: Option<DateTime<Utc>>,
    last_emitted_thought: Option<String>,
    sleeping_emitted: bool,
    thought_state: ThoughtState,
    thought_source: ThoughtSource,
    rest_state: RestState,
    commit_candidate: bool,
    action_cues: Vec<ActionCue>,
    objective_fingerprint: Option<String>,
    objective_stable_since: DateTime<Utc>,
    claimed_jsonl_path: Option<PathBuf>,
    claimed_cwd: Option<String>,
    emission_seq: u64,
    last_observed: Option<ObservedSessionFacts>,
}

#[derive(Clone, PartialEq, Eq)]
struct ObservedSessionFacts {
    state: SessionState,
    exited: bool,
    tool: Option<String>,
    cwd: String,
    replay_fingerprint: u64,
    last_activity_at: DateTime<Utc>,
}

impl ObservedSessionFacts {
    fn from_session(session: &SessionSnapshot) -> Self {
        Self {
            state: session.state,
            exited: session.exited,
            tool: session.tool.clone(),
            cwd: session.cwd.clone(),
            replay_fingerprint: hash_string(&session.replay_text),
            last_activity_at: session.last_activity_at,
        }
    }
}

impl SessionRuntimeState {
    fn initialize_from_session(session: &SessionSnapshot, now: DateTime<Utc>) -> Self {
        let mut summary_history = Vec::new();
        if let Some(thought) = session.thought.as_ref() {
            summary_history.push(thought.clone());
        }

        let thought_updated_at = session.thought_updated_at.unwrap_or(now);
        let run_started_at = initial_run_started_at(session, now);

        Self {
            summary_history,
            last_terminal_context: Some(trim_terminal_context(&session.replay_text)),
            last_call_at: session.thought_updated_at,
            run_started_at,
            run_finished_at: initial_run_finished_at(session, now),
            last_emitted_thought: session.thought.clone(),
            sleeping_emitted: session.thought_state == ThoughtState::Sleeping
                && matches!(
                    session.rest_state,
                    RestState::Sleeping | RestState::DeepSleep
                ),
            thought_state: session.thought_state,
            thought_source: session.thought_source,
            rest_state: session.rest_state,
            commit_candidate: session.commit_candidate,
            action_cues: session.action_cues.clone(),
            objective_fingerprint: session.objective_fingerprint.clone(),
            objective_stable_since: thought_updated_at,
            claimed_jsonl_path: None,
            claimed_cwd: None,
            emission_seq: 0,
            last_observed: None,
        }
    }

    fn next_emission_seq(&mut self) -> u64 {
        self.emission_seq = self.emission_seq.saturating_add(1);
        self.emission_seq
    }

    fn cadence_tier(&self, config: &ThoughtConfig, now: DateTime<Utc>) -> CadenceTier {
        let objective_age_ms = (now - self.objective_stable_since).num_milliseconds();
        if objective_age_ms >= config.cadence_cold_ms as i64 {
            CadenceTier::Cold
        } else if objective_age_ms >= config.cadence_warm_ms as i64 {
            CadenceTier::Warm
        } else {
            CadenceTier::Hot
        }
    }

    fn cadence_for_state(&self, config: &ThoughtConfig, now: DateTime<Utc>) -> u64 {
        match self.cadence_tier(config, now) {
            CadenceTier::Cold => config.cadence_cold_ms,
            CadenceTier::Warm => config.cadence_warm_ms,
            CadenceTier::Hot => config.cadence_hot_ms,
        }
    }

    fn should_call_for_cadence(&self, config: &ThoughtConfig, now: DateTime<Utc>) -> bool {
        match self.last_call_at {
            Some(last_call) => {
                let elapsed_ms = (now - last_call).num_milliseconds();
                elapsed_ms >= self.cadence_for_state(config, now) as i64
            }
            None => true,
        }
    }
}

struct PreparedSessionContext {
    context_snapshot: Option<Snapshot>,
    context_source: ContextSource,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: Vec<ActionCue>,
    objective_fingerprint: String,
    objective_changed: bool,
}

pub struct EmitEngine {
    clients: HashMap<ModelBackend, Box<dyn ModelClient>>,
    default_backend: ModelBackend,
    per_session: HashMap<String, SessionRuntimeState>,
    stream_instance_id: String,
}

impl EmitEngine {
    pub fn new(model_client: Box<dyn ModelClient>) -> Self {
        Self::with_backend(model_client, super::model_client::resolve_model_backend())
    }

    pub fn with_backend(model_client: Box<dyn ModelClient>, default_backend: ModelBackend) -> Self {
        let mut clients: HashMap<ModelBackend, Box<dyn ModelClient>> = HashMap::new();
        clients.insert(default_backend, model_client);
        Self {
            clients,
            default_backend,
            per_session: HashMap::new(),
            stream_instance_id: format!(
                "stream-{}-{}",
                Utc::now().timestamp_millis(),
                std::process::id()
            ),
        }
    }

    pub fn sync(&mut self, request: &SyncRequest) -> SyncResultMessage {
        let mut updates = Vec::new();
        let mut metrics = SyncMetrics::default();
        let transcript_group_counts = compute_transcript_group_counts(&request.sessions);
        let session_deltas =
            compute_session_deltas(&mut self.per_session, request, &transcript_group_counts);

        let active_ids: HashSet<&str> = request
            .sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect();
        self.per_session
            .retain(|session_id, _| active_ids.contains(session_id.as_str()));

        if !request.config.enabled {
            self.clear_all_sessions(request, &mut updates, &mut metrics);
            return self.build_sync_result(&request.id, updates, metrics, session_deltas);
        }

        let backend = request
            .config
            .backend_override()
            .unwrap_or(self.default_backend);

        if let Err(err) = self.ensure_client(backend) {
            metrics.last_backend_error = Some(err.message);
            if err.allow_carry_forward {
                self.carry_forward_sessions(
                    request,
                    &transcript_group_counts,
                    &mut updates,
                    &mut metrics,
                );
            }
            return self.build_sync_result(&request.id, updates, metrics, session_deltas);
        }

        let Some(model_client) = self.clients.get(&backend).map(|client| &**client) else {
            metrics.last_backend_error = Some(format!(
                "{}: model client cache missing after initialization",
                backend.as_str()
            ));
            return self.build_sync_result(&request.id, updates, metrics, session_deltas);
        };
        let stream_instance_id = self.stream_instance_id.clone();

        for session in &request.sessions {
            process_session(
                model_client,
                &stream_instance_id,
                &mut self.per_session,
                request,
                session,
                &transcript_group_counts,
                &mut updates,
                &mut metrics,
            );
        }

        self.build_sync_result(&request.id, updates, metrics, session_deltas)
    }

    fn build_sync_result(
        &self,
        request_id: &str,
        updates: Vec<ThoughtUpdate>,
        metrics: SyncMetrics,
        session_deltas: Vec<SessionDelta>,
    ) -> SyncResultMessage {
        SyncResultMessage::new(
            request_id.to_string(),
            self.stream_instance_id.clone(),
            updates,
            metrics,
        )
        .with_session_deltas(session_deltas)
    }

    /// Ensures `self.clients[backend]` is populated. Returns `Err` with a
    /// reason and a flag for whether sessions should still get carry-forward
    /// rest-state updates (true on credential failure, false when client
    /// construction itself fails — the latter would have populated metrics
    /// regardless).
    fn ensure_client(&mut self, backend: ModelBackend) -> Result<(), BackendInitError> {
        if self.clients.contains_key(&backend) {
            return Ok(());
        }
        if let Err(err) = validate_backend_credentials(backend) {
            return Err(BackendInitError {
                message: err,
                allow_carry_forward: true,
            });
        }
        match build_model_client_for(backend) {
            Ok(client) => {
                self.clients.insert(backend, client);
                Ok(())
            }
            Err(err) => Err(BackendInitError {
                message: format!("{}: {err}", backend.as_str()),
                allow_carry_forward: false,
            }),
        }
    }

    fn carry_forward_sessions(
        &mut self,
        request: &SyncRequest,
        transcript_group_counts: &HashMap<String, usize>,
        updates: &mut Vec<ThoughtUpdate>,
        metrics: &mut SyncMetrics,
    ) {
        let stream_instance_id = self.stream_instance_id.clone();
        for session in &request.sessions {
            metrics.sessions_seen += 1;
            metrics.suppressed += 1;
            let state = self
                .per_session
                .entry(session.session_id.clone())
                .or_insert_with(|| {
                    SessionRuntimeState::initialize_from_session(session, request.now)
                });

            if handle_exited_session(
                &stream_instance_id,
                state,
                session,
                &request.config,
                request.now,
                updates,
                metrics,
            ) {
                continue;
            }

            invalidate_stale_claim(state, session);
            let (context_snapshot, resolved_path) = context_snapshot_for_session_with_claim(
                session,
                state.claimed_jsonl_path.as_deref(),
                transcript_group_is_ambiguous(session, transcript_group_counts),
            );
            let context_source = context_source_for_snapshot(context_snapshot.as_ref());
            store_resolved_claim(state, session, resolved_path);
            let next_rest_state =
                rest_state_for_session(session, context_snapshot.as_ref(), request.now);
            let next_commit_candidate =
                commit_candidate_for_context(context_snapshot.as_ref(), session);
            let next_action_cues = action_cues_for_context(context_snapshot.as_ref(), session);

            emit_passive_state_change_if_needed(
                updates,
                &stream_instance_id,
                state,
                session,
                &request.config,
                context_source,
                next_rest_state,
                next_commit_candidate,
                &next_action_cues,
                request.now,
            );
        }
    }

    fn clear_all_sessions(
        &mut self,
        request: &SyncRequest,
        updates: &mut Vec<ThoughtUpdate>,
        metrics: &mut SyncMetrics,
    ) {
        for session in &request.sessions {
            metrics.sessions_seen += 1;
            let state = self
                .per_session
                .entry(session.session_id.clone())
                .or_insert_with(|| {
                    SessionRuntimeState::initialize_from_session(session, request.now)
                });
            let next_rest_state = clear_rest_state_for_session(session, request.now);

            if session_needs_clear(state, session, next_rest_state) {
                updates.push(clear_thought_update(
                    &self.stream_instance_id,
                    state,
                    session,
                    &request.config,
                    ContextSource::Terminal,
                    request.now,
                    next_rest_state,
                    false,
                    &[],
                    state.objective_fingerprint.clone(),
                ));
                state.last_emitted_thought = None;
                state.thought_state = ThoughtState::Holding;
                state.thought_source = ThoughtSource::CarryForward;
                state.rest_state = next_rest_state;
                state.commit_candidate = false;
                state.action_cues.clear();
                state.sleeping_emitted = false;
            } else {
                metrics.suppressed += 1;
            }
        }
    }
}

/// Drop a stale JSONL claim when the pane has switched CWDs since we last
/// pinned a transcript file. Keeping the old claim would attribute another
/// project's transcript to this session.
fn invalidate_stale_claim(state: &mut SessionRuntimeState, session: &SessionSnapshot) {
    if state
        .claimed_cwd
        .as_deref()
        .is_some_and(|prev| prev != session.cwd)
    {
        state.claimed_jsonl_path = None;
        state.claimed_cwd = None;
    }
}

/// Record the freshly resolved transcript path (if any) and pin the CWD that
/// produced it, so the next observation can detect a switch.
fn store_resolved_claim(
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    resolved_path: Option<std::path::PathBuf>,
) {
    if resolved_path.is_some() {
        state.claimed_cwd = Some(session.cwd.clone());
    }
    state.claimed_jsonl_path = resolved_path;
}

fn handle_exited_session(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    now: DateTime<Utc>,
    updates: &mut Vec<ThoughtUpdate>,
    metrics: &mut SyncMetrics,
) -> bool {
    if !session.exited && session.state != SessionState::Exited {
        return false;
    }

    let next_rest_state = RestState::DeepSleep;
    if runtime_state_needs_clear(state, next_rest_state) || session_carries_dirty_signal(session) {
        updates.push(clear_thought_update(
            stream_instance_id,
            state,
            session,
            config,
            ContextSource::Terminal,
            now,
            next_rest_state,
            false,
            &[],
            state.objective_fingerprint.clone(),
        ));
        state.last_emitted_thought = None;
        state.thought_state = ThoughtState::Holding;
        state.thought_source = ThoughtSource::CarryForward;
        state.rest_state = next_rest_state;
        state.commit_candidate = false;
        state.action_cues.clear();
        state.sleeping_emitted = false;
        state.last_terminal_context = Some(trim_terminal_context(&session.replay_text));
    } else {
        metrics.suppressed += 1;
    }

    true
}

fn process_session(
    model_client: &dyn ModelClient,
    stream_instance_id: &str,
    per_session: &mut HashMap<String, SessionRuntimeState>,
    request: &SyncRequest,
    session: &SessionSnapshot,
    transcript_group_counts: &HashMap<String, usize>,
    updates: &mut Vec<ThoughtUpdate>,
    metrics: &mut SyncMetrics,
) {
    metrics.sessions_seen += 1;

    let state = per_session
        .entry(session.session_id.clone())
        .or_insert_with(|| SessionRuntimeState::initialize_from_session(session, request.now));

    if handle_exited_session(
        stream_instance_id,
        state,
        session,
        &request.config,
        request.now,
        updates,
        metrics,
    ) {
        return;
    }

    invalidate_stale_claim(state, session);

    let (context_snapshot, resolved_path) = context_snapshot_for_session_with_claim(
        session,
        state.claimed_jsonl_path.as_deref(),
        transcript_group_is_ambiguous(session, transcript_group_counts),
    );
    let context_source = context_source_for_snapshot(context_snapshot.as_ref());
    store_resolved_claim(state, session, resolved_path);
    let next_rest_state = rest_state_for_session(session, context_snapshot.as_ref(), request.now);
    let next_commit_candidate = commit_candidate_for_context(context_snapshot.as_ref(), session);
    let next_action_cues = action_cues_for_context(context_snapshot.as_ref(), session);

    if handle_sleeping_session(
        stream_instance_id,
        state,
        session,
        context_snapshot.as_ref(),
        &request.config,
        context_source,
        request.now,
        next_rest_state,
        next_commit_candidate,
        &next_action_cues,
        updates,
        metrics,
    ) {
        return;
    }

    let woke_from_sleep = wake_from_sleep_if_needed(
        stream_instance_id,
        state,
        session,
        &request.config,
        context_source,
        request.now,
        next_rest_state,
        next_commit_candidate,
        &next_action_cues,
        updates,
    );

    let Some(prepared) = prepare_session_context(
        stream_instance_id,
        state,
        session,
        &request.config,
        request.now,
        context_snapshot,
        context_source,
        next_rest_state,
        next_commit_candidate,
        next_action_cues,
        updates,
        metrics,
    ) else {
        return;
    };

    if suppress_for_cadence_or_terminal_delta(
        stream_instance_id,
        state,
        session,
        prepared.context_snapshot.as_ref(),
        &request.config,
        prepared.context_source,
        prepared.objective_changed,
        woke_from_sleep,
        request.now,
        prepared.next_rest_state,
        prepared.next_commit_candidate,
        &prepared.next_action_cues,
        updates,
        metrics,
    ) {
        return;
    }

    emit_session_thought(
        model_client,
        stream_instance_id,
        state,
        request,
        session,
        &prepared,
        updates,
        metrics,
    );
}

fn wake_from_sleep_if_needed(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    context_source: ContextSource,
    now: DateTime<Utc>,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    updates: &mut Vec<ThoughtUpdate>,
) -> bool {
    if state.thought_state != ThoughtState::Sleeping {
        return false;
    }

    restart_run(state, now);
    state.objective_stable_since = now;
    state.last_call_at = None;
    updates.push(clear_thought_update(
        stream_instance_id,
        state,
        session,
        config,
        context_source,
        now,
        next_rest_state,
        next_commit_candidate,
        next_action_cues,
        state.objective_fingerprint.clone(),
    ));
    state.thought_state = ThoughtState::Holding;
    state.thought_source = ThoughtSource::CarryForward;
    state.rest_state = next_rest_state;
    state.commit_candidate = next_commit_candidate;
    state.action_cues = next_action_cues.to_vec();
    state.sleeping_emitted = false;
    state.last_emitted_thought = None;
    true
}

fn prepare_session_context(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    now: DateTime<Utc>,
    context_snapshot: Option<Snapshot>,
    context_source: ContextSource,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: Vec<ActionCue>,
    updates: &mut Vec<ThoughtUpdate>,
    metrics: &mut SyncMetrics,
) -> Option<PreparedSessionContext> {
    if suppress_for_initial_context(
        stream_instance_id,
        state,
        session,
        config,
        context_source,
        now,
        context_snapshot.as_ref(),
        next_rest_state,
        next_commit_candidate,
        &next_action_cues,
        updates,
        metrics,
    ) {
        return None;
    }

    let objective_fingerprint =
        objective_fingerprint_for_session(context_snapshot.as_ref(), session);
    let objective_changed = update_objective_fingerprint(state, &objective_fingerprint, now);

    Some(PreparedSessionContext {
        context_snapshot,
        context_source,
        next_rest_state,
        next_commit_candidate,
        next_action_cues,
        objective_fingerprint,
        objective_changed,
    })
}

fn transcript_group_is_ambiguous(
    session: &SessionSnapshot,
    transcript_group_counts: &HashMap<String, usize>,
) -> bool {
    transcript_group_key(session)
        .and_then(|group_key| transcript_group_counts.get(&group_key).copied())
        .unwrap_or_default()
        > 1
}

fn objective_fingerprint_for_session(
    context_snapshot: Option<&Snapshot>,
    session: &SessionSnapshot,
) -> String {
    context_snapshot
        .map(|snapshot| context_focus_fingerprint(snapshot, &session.state).to_string())
        .unwrap_or_else(|| terminal_objective_fingerprint(&session.replay_text, &session.state))
}

fn emit_session_thought(
    model_client: &dyn ModelClient,
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    request: &SyncRequest,
    session: &SessionSnapshot,
    prepared: &PreparedSessionContext,
    updates: &mut Vec<ThoughtUpdate>,
    metrics: &mut SyncMetrics,
) {
    state.last_call_at = Some(request.now);
    let prompt = prompt_for_session(prepared.context_snapshot.as_ref(), session, state, request);
    let Some(raw_thought) =
        complete_session_thought(model_client, &prompt, request, session, state, metrics)
    else {
        emit_passive_state_change_if_needed(
            updates,
            stream_instance_id,
            state,
            session,
            &request.config,
            prepared.context_source,
            prepared.next_rest_state,
            prepared.next_commit_candidate,
            &prepared.next_action_cues,
            request.now,
        );
        return;
    };
    metrics.llm_calls += 1;

    let thought = sanitize_thought_text(&raw_thought);
    if thought.is_empty() {
        if !emit_passive_state_change_if_needed(
            updates,
            stream_instance_id,
            state,
            session,
            &request.config,
            prepared.context_source,
            prepared.next_rest_state,
            prepared.next_commit_candidate,
            &prepared.next_action_cues,
            request.now,
        ) {
            metrics.suppressed += 1;
        }
        return;
    }

    if handle_duplicate_generated_thought(
        &thought,
        updates,
        stream_instance_id,
        state,
        session,
        &request.config,
        prepared.context_source,
        prepared.next_rest_state,
        prepared.next_commit_candidate,
        &prepared.next_action_cues,
        request.now,
        metrics,
    ) {
        return;
    }

    publish_generated_thought(
        stream_instance_id,
        state,
        session,
        &request.config,
        &thought,
        prepared,
        request.now,
        updates,
    );
}

fn prompt_for_session(
    context_snapshot: Option<&Snapshot>,
    session: &SessionSnapshot,
    state: &SessionRuntimeState,
    request: &SyncRequest,
) -> String {
    context_snapshot
        .map(|snapshot| {
            build_context_prompt(
                snapshot,
                &session.state,
                &state.summary_history,
                request.config.agent_prompt.as_deref(),
            )
        })
        .unwrap_or_else(|| {
            build_terminal_prompt(
                &session.replay_text,
                &session.state,
                state.last_terminal_context.as_deref(),
                request.config.terminal_prompt.as_deref(),
            )
        })
}

fn complete_session_thought(
    model_client: &dyn ModelClient,
    prompt: &str,
    request: &SyncRequest,
    session: &SessionSnapshot,
    state: &mut SessionRuntimeState,
    metrics: &mut SyncMetrics,
) -> Option<String> {
    match model_client.complete(prompt, request.config.model_override()) {
        Ok(value) => Some(value),
        Err(err) => {
            metrics.suppressed += 1;
            if metrics.last_backend_error.is_none() {
                metrics.last_backend_error = Some(err);
            }
            state.last_terminal_context = Some(trim_terminal_context(&session.replay_text));
            None
        }
    }
}

fn handle_duplicate_generated_thought(
    thought: &str,
    updates: &mut Vec<ThoughtUpdate>,
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    context_source: ContextSource,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    now: DateTime<Utc>,
    metrics: &mut SyncMetrics,
) -> bool {
    if !is_duplicate_thought(state.last_emitted_thought.as_deref(), thought) {
        return false;
    }

    if emit_passive_state_change_if_needed(
        updates,
        stream_instance_id,
        state,
        session,
        config,
        context_source,
        next_rest_state,
        next_commit_candidate,
        next_action_cues,
        now,
    ) {
        return true;
    }
    metrics.suppressed += 1;
    true
}

fn publish_generated_thought(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    thought: &str,
    prepared: &PreparedSessionContext,
    now: DateTime<Utc>,
    updates: &mut Vec<ThoughtUpdate>,
) {
    let next_state = next_thought_state(prepared.objective_changed);
    let token_count = token_count_for_context(prepared.context_snapshot.as_ref(), session);
    updates.push(thought_update(
        stream_instance_id,
        state,
        session,
        Some(thought.to_string()),
        token_count,
        session.context_limit,
        next_state,
        ThoughtSource::Llm,
        prepared.objective_changed,
        now,
        Some(prepared.objective_fingerprint.clone()),
        prepared.next_rest_state,
        prepared.next_commit_candidate,
        &prepared.next_action_cues,
        config,
        prepared.context_source,
    ));

    state.last_emitted_thought = Some(thought.to_string());
    state.summary_history.push(thought.to_string());
    if state.summary_history.len() > SUMMARY_HISTORY_CAP {
        let start = state.summary_history.len() - SUMMARY_HISTORY_CAP;
        state.summary_history = state.summary_history.split_off(start);
    }
    state.thought_state = next_state;
    state.thought_source = ThoughtSource::Llm;
    state.rest_state = prepared.next_rest_state;
    state.commit_candidate = prepared.next_commit_candidate;
    state.action_cues = prepared.next_action_cues.clone();
    state.sleeping_emitted = false;
    state.last_terminal_context = Some(trim_terminal_context(&session.replay_text));
}

fn next_thought_state(objective_changed: bool) -> ThoughtState {
    if objective_changed {
        ThoughtState::Active
    } else {
        ThoughtState::Holding
    }
}

fn token_count_for_context(context_snapshot: Option<&Snapshot>, session: &SessionSnapshot) -> u64 {
    context_snapshot
        .map(|snapshot| snapshot.token_count)
        .unwrap_or(session.token_count)
}

/// Suppression rule for sleeping sync ticks: emit only when entering sleep,
/// when re-emission was lost, or when one of the carried fields changes.
/// Pulled out of `handle_sleeping_session` to keep that function's branch
/// count low.
fn sleeping_emit_needed(
    state: &SessionRuntimeState,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    carried_thought: &Option<String>,
) -> bool {
    state.thought_state != ThoughtState::Sleeping
        || !state.sleeping_emitted
        || state.rest_state != next_rest_state
        || state.commit_candidate != next_commit_candidate
        || state.action_cues.as_slice() != next_action_cues
        || state.last_emitted_thought != *carried_thought
}

fn handle_sleeping_session(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    context_snapshot: Option<&Snapshot>,
    config: &ThoughtConfig,
    context_source: ContextSource,
    now: DateTime<Utc>,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    updates: &mut Vec<ThoughtUpdate>,
    metrics: &mut SyncMetrics,
) -> bool {
    if !is_sleeping_rest_state(next_rest_state) {
        return false;
    }

    let carried_thought = sleeping_thought_for_update(state, session, context_snapshot);
    let carried_source = sleeping_thought_source(state);
    let should_emit_sleeping = sleeping_emit_needed(
        state,
        next_rest_state,
        next_commit_candidate,
        next_action_cues,
        &carried_thought,
    );
    let token_count = token_count_for_context(context_snapshot, session);

    freeze_run(state, now);
    if should_emit_sleeping {
        updates.push(thought_update(
            stream_instance_id,
            state,
            session,
            carried_thought.clone(),
            token_count,
            session.context_limit,
            ThoughtState::Sleeping,
            carried_source,
            false,
            now,
            state.objective_fingerprint.clone(),
            next_rest_state,
            next_commit_candidate,
            next_action_cues,
            config,
            context_source,
        ));
    } else {
        metrics.suppressed += 1;
    }

    state.sleeping_emitted = true;
    state.thought_state = ThoughtState::Sleeping;
    state.thought_source = carried_source;
    state.rest_state = next_rest_state;
    state.commit_candidate = next_commit_candidate;
    state.action_cues = next_action_cues.to_vec();
    state.last_emitted_thought = carried_thought;
    state.last_terminal_context = Some(trim_terminal_context(&session.replay_text));
    true
}

fn suppress_for_initial_context(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    context_source: ContextSource,
    now: DateTime<Utc>,
    context_snapshot: Option<&Snapshot>,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    updates: &mut Vec<ThoughtUpdate>,
    metrics: &mut SyncMetrics,
) -> bool {
    if is_initial_thought_candidate(state, session)
        && !has_adequate_initial_context(
            context_snapshot,
            &session.replay_text,
            next_commit_candidate,
            next_action_cues,
        )
    {
        state.last_terminal_context = Some(trim_terminal_context(&session.replay_text));
        if emit_passive_state_change_if_needed(
            updates,
            stream_instance_id,
            state,
            session,
            config,
            context_source,
            next_rest_state,
            next_commit_candidate,
            next_action_cues,
            now,
        ) {
            return true;
        }
        metrics.suppressed += 1;
        return true;
    }

    false
}

fn update_objective_fingerprint(
    state: &mut SessionRuntimeState,
    objective_fingerprint: &str,
    now: DateTime<Utc>,
) -> bool {
    let objective_changed = state.objective_fingerprint.as_deref() != Some(objective_fingerprint);
    if objective_changed {
        state.objective_stable_since = now;
        if state.objective_fingerprint.is_some() {
            restart_run(state, now);
        }
        state.objective_fingerprint = Some(objective_fingerprint.to_string());
    }
    objective_changed
}

fn clear_thought_update(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    context_source: ContextSource,
    now: DateTime<Utc>,
    rest_state: RestState,
    commit_candidate: bool,
    action_cues: &[ActionCue],
    objective_fingerprint: Option<String>,
) -> ThoughtUpdate {
    thought_update(
        stream_instance_id,
        state,
        session,
        None,
        session.token_count,
        session.context_limit,
        ThoughtState::Holding,
        ThoughtSource::CarryForward,
        false,
        now,
        objective_fingerprint,
        rest_state,
        commit_candidate,
        action_cues,
        config,
        context_source,
    )
}

fn emit_passive_state_change_if_needed(
    updates: &mut Vec<ThoughtUpdate>,
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    context_source: ContextSource,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    now: DateTime<Utc>,
) -> bool {
    let first_observation_has_visible_state = state.emission_seq == 0
        && state.last_emitted_thought.is_none()
        && (next_commit_candidate || !next_action_cues.is_empty());
    if state.rest_state == next_rest_state
        && state.commit_candidate == next_commit_candidate
        && state.action_cues.as_slice() == next_action_cues
        && !first_observation_has_visible_state
    {
        return false;
    }

    let (thought_state, thought_source, thought) =
        passive_state_payload(state, session, next_rest_state);
    updates.push(thought_update(
        stream_instance_id,
        state,
        session,
        thought.clone(),
        session.token_count,
        session.context_limit,
        thought_state,
        thought_source,
        false,
        now,
        state.objective_fingerprint.clone(),
        next_rest_state,
        next_commit_candidate,
        next_action_cues,
        config,
        context_source,
    ));
    state.thought_state = thought_state;
    state.thought_source = thought_source;
    state.rest_state = next_rest_state;
    state.commit_candidate = next_commit_candidate;
    state.action_cues = next_action_cues.to_vec();
    state.last_emitted_thought = thought;
    state.sleeping_emitted = is_sleeping_rest_state(next_rest_state);
    true
}

fn passive_state_payload(
    state: &SessionRuntimeState,
    session: &SessionSnapshot,
    next_rest_state: RestState,
) -> (ThoughtState, ThoughtSource, Option<String>) {
    if is_sleeping_rest_state(next_rest_state) {
        (
            ThoughtState::Sleeping,
            state.thought_source,
            current_thought_for_update(state, session),
        )
    } else if state.thought_state == ThoughtState::Sleeping {
        (ThoughtState::Holding, ThoughtSource::CarryForward, None)
    } else {
        (
            state.thought_state,
            state.thought_source,
            current_thought_for_update(state, session),
        )
    }
}

fn current_thought_for_update(
    state: &SessionRuntimeState,
    session: &SessionSnapshot,
) -> Option<String> {
    state
        .last_emitted_thought
        .clone()
        .or_else(|| session.thought.clone())
}

fn sleeping_thought_for_update(
    state: &SessionRuntimeState,
    session: &SessionSnapshot,
    context_snapshot: Option<&Snapshot>,
) -> Option<String> {
    context_snapshot
        .and_then(|snapshot| snapshot.awaiting_user_text.as_deref())
        .map(sanitize_thought_text)
        .filter(|thought| !thought.is_empty())
        .or_else(|| current_thought_for_update(state, session))
}

fn thought_update(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    thought: Option<String>,
    token_count: u64,
    context_limit: u64,
    thought_state: ThoughtState,
    thought_source: ThoughtSource,
    objective_changed: bool,
    at: DateTime<Utc>,
    objective_fingerprint: Option<String>,
    rest_state: RestState,
    commit_candidate: bool,
    action_cues: &[ActionCue],
    config: &ThoughtConfig,
    context_source: ContextSource,
) -> ThoughtUpdate {
    ThoughtUpdate {
        session_id: session.session_id.clone(),
        stream_instance_id: Some(stream_instance_id.to_string()),
        emission_seq: Some(state.next_emission_seq()),
        thought,
        token_count,
        context_limit,
        thought_state,
        thought_source,
        objective_changed,
        bubble_precedence: BubblePrecedence::ThoughtFirst,
        at,
        objective_fingerprint,
        rest_state,
        commit_candidate,
        action_cues: action_cues.to_vec(),
        timing: Some(timing_info_for_update(state, session, at)),
        cues: Some(cue_info_for_update(state, config, at, context_source)),
    }
}

fn rest_state_for_session(
    session: &SessionSnapshot,
    context_snapshot: Option<&Snapshot>,
    now: DateTime<Utc>,
) -> RestState {
    match (session.exited, &session.state) {
        (true, _) | (_, SessionState::Exited) => RestState::DeepSleep,
        _ if context_snapshot.is_some_and(|snapshot| snapshot.awaiting_user_input) => {
            RestState::Sleeping
        }
        _ if context_snapshot.is_none()
            && valid_action_cues_contain(&session.action_cues, ActionCueKind::AwaitingUser) =>
        {
            RestState::Sleeping
        }
        (false, SessionState::Idle | SessionState::Attention) => {
            idle_rest_state((now - session.last_activity_at).num_milliseconds().max(0))
        }
        _ => RestState::Active,
    }
}

fn commit_candidate_for_context(
    context_snapshot: Option<&Snapshot>,
    session: &SessionSnapshot,
) -> bool {
    if session.exited || session.state == SessionState::Exited {
        return false;
    }

    match context_snapshot {
        Some(snapshot) => snapshot
            .commit_signal
            .as_ref()
            .is_some_and(|signal| signal.candidate),
        None => {
            session.commit_candidate
                || valid_action_cues_contain(&session.action_cues, ActionCueKind::CommitReady)
        }
    }
}

fn action_cues_for_context(
    context_snapshot: Option<&Snapshot>,
    session: &SessionSnapshot,
) -> Vec<ActionCue> {
    if session.exited || session.state == SessionState::Exited {
        return Vec::new();
    }

    context_snapshot
        .map(|snapshot| snapshot.action_cues.clone())
        .unwrap_or_else(|| valid_action_cues(&session.action_cues))
}

fn valid_action_cues(action_cues: &[ActionCue]) -> Vec<ActionCue> {
    action_cues
        .iter()
        .filter(|cue| cue.is_valid())
        .cloned()
        .collect()
}

fn valid_action_cues_contain(action_cues: &[ActionCue], kind: ActionCueKind) -> bool {
    action_cues
        .iter()
        .any(|cue| cue.kind == kind && cue.is_valid())
}

fn clear_rest_state_for_session(session: &SessionSnapshot, now: DateTime<Utc>) -> RestState {
    match (session.exited, &session.state) {
        (true, _) | (_, SessionState::Exited) => RestState::DeepSleep,
        (false, SessionState::Idle | SessionState::Attention) => {
            idle_rest_state((now - session.last_activity_at).num_milliseconds().max(0))
        }
        _ => RestState::Active,
    }
}

fn idle_rest_state(idle_ms: i64) -> RestState {
    if idle_ms >= DROWSY_AFTER_MS {
        RestState::Drowsy
    } else {
        RestState::Active
    }
}

fn is_sleeping_rest_state(rest_state: RestState) -> bool {
    matches!(rest_state, RestState::Sleeping | RestState::DeepSleep)
}

fn should_suppress_for_cadence(
    state: &SessionRuntimeState,
    config: &ThoughtConfig,
    objective_changed: bool,
    woke_from_sleep: bool,
    now: DateTime<Utc>,
) -> bool {
    !objective_changed && !woke_from_sleep && !state.should_call_for_cadence(config, now)
}

fn should_suppress_for_terminal_delta(
    session: &SessionSnapshot,
    state: &SessionRuntimeState,
    context_snapshot: Option<&Snapshot>,
    objective_changed: bool,
    woke_from_sleep: bool,
) -> bool {
    context_snapshot.is_none()
        && !objective_changed
        && !woke_from_sleep
        && !has_meaningful_terminal_delta(
            &session.replay_text,
            state.last_terminal_context.as_deref(),
        )
}

fn suppress_for_cadence_or_terminal_delta(
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    context_snapshot: Option<&Snapshot>,
    config: &ThoughtConfig,
    context_source: ContextSource,
    objective_changed: bool,
    woke_from_sleep: bool,
    now: DateTime<Utc>,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    updates: &mut Vec<ThoughtUpdate>,
    metrics: &mut SyncMetrics,
) -> bool {
    if should_suppress_for_cadence(state, config, objective_changed, woke_from_sleep, now) {
        return suppress_with_passive_state_change(
            updates,
            stream_instance_id,
            state,
            session,
            config,
            context_source,
            next_rest_state,
            next_commit_candidate,
            next_action_cues,
            now,
            metrics,
        );
    }

    if should_suppress_for_terminal_delta(
        session,
        state,
        context_snapshot,
        objective_changed,
        woke_from_sleep,
    ) {
        return suppress_with_passive_state_change(
            updates,
            stream_instance_id,
            state,
            session,
            config,
            context_source,
            next_rest_state,
            next_commit_candidate,
            next_action_cues,
            now,
            metrics,
        );
    }

    false
}

fn suppress_with_passive_state_change(
    updates: &mut Vec<ThoughtUpdate>,
    stream_instance_id: &str,
    state: &mut SessionRuntimeState,
    session: &SessionSnapshot,
    config: &ThoughtConfig,
    context_source: ContextSource,
    next_rest_state: RestState,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
    now: DateTime<Utc>,
    metrics: &mut SyncMetrics,
) -> bool {
    if emit_passive_state_change_if_needed(
        updates,
        stream_instance_id,
        state,
        session,
        config,
        context_source,
        next_rest_state,
        next_commit_candidate,
        next_action_cues,
        now,
    ) {
        true
    } else {
        metrics.suppressed += 1;
        true
    }
}

fn context_source_for_snapshot(context_snapshot: Option<&Snapshot>) -> ContextSource {
    if context_snapshot.is_some() {
        ContextSource::Transcript
    } else {
        ContextSource::Terminal
    }
}

fn initial_run_started_at(session: &SessionSnapshot, now: DateTime<Utc>) -> DateTime<Utc> {
    clamp_to_now(
        session
            .thought_updated_at
            .unwrap_or(session.last_activity_at),
        now,
    )
}

fn initial_run_finished_at(session: &SessionSnapshot, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if session.exited || is_sleeping_rest_state(session.rest_state) {
        Some(clamp_to_now(session.thought_updated_at.unwrap_or(now), now))
    } else {
        None
    }
}

fn clamp_to_now(value: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    if value > now {
        now
    } else {
        value
    }
}

fn restart_run(state: &mut SessionRuntimeState, now: DateTime<Utc>) {
    state.run_started_at = now;
    state.run_finished_at = None;
}

fn freeze_run(state: &mut SessionRuntimeState, now: DateTime<Utc>) {
    if state.run_finished_at.is_none() {
        state.run_finished_at = Some(now);
    }
}

fn timing_info_for_update(
    state: &SessionRuntimeState,
    session: &SessionSnapshot,
    now: DateTime<Utc>,
) -> TimingInfo {
    let run_end = state.run_finished_at.unwrap_or(now);
    TimingInfo {
        run_started_at: state.run_started_at,
        run_finished_at: state.run_finished_at,
        run_elapsed_ms: saturating_elapsed_ms(state.run_started_at, run_end),
        idle_elapsed_ms: (now - session.last_activity_at).num_milliseconds().max(0) as u64,
    }
}

fn cue_info_for_update(
    state: &SessionRuntimeState,
    config: &ThoughtConfig,
    now: DateTime<Utc>,
    context_source: ContextSource,
) -> CueInfo {
    let cadence_ms = state.cadence_for_state(config, now);
    CueInfo {
        cadence_tier: state.cadence_tier(config, now),
        cadence_ms,
        next_llm_eligible_at: state
            .last_call_at
            .map(|last_call| last_call + ChronoDuration::milliseconds(cadence_ms as i64))
            .unwrap_or(now),
        context_source,
    }
}

fn saturating_elapsed_ms(start: DateTime<Utc>, end: DateTime<Utc>) -> u64 {
    (end - start).num_milliseconds().max(0) as u64
}

fn sleeping_thought_source(state: &SessionRuntimeState) -> ThoughtSource {
    match state.thought_source {
        ThoughtSource::StaticSleeping => ThoughtSource::CarryForward,
        source => source,
    }
}

fn transcript_group_key(session: &SessionSnapshot) -> Option<String> {
    let selection = tool_selection_for_session(session.tool.as_deref())?;
    let tool = match selection {
        ToolSelection::Claude => "claude",
        ToolSelection::Codex => "codex",
        ToolSelection::Auto => return None,
    };

    Some(format!("{tool}:{}", session.cwd))
}

fn claimed_context_snapshot(
    selection: ToolSelection,
    cwd: &Path,
    existing_claim: Option<&Path>,
) -> Option<(Option<Snapshot>, Option<PathBuf>)> {
    existing_claim
        .filter(|claimed| claimed.exists())
        .and_then(|claimed| resolved_claim_snapshot(selection, cwd, claimed))
}

fn resolved_claim_snapshot(
    selection: ToolSelection,
    cwd: &Path,
    claimed: &Path,
) -> Option<(Option<Snapshot>, Option<PathBuf>)> {
    let resolved = resolve_input(selection, cwd, Some(claimed)).ok()?;
    let snapshot = extract(
        resolved.tool,
        &resolved.path,
        cwd,
        resolved.discovered,
        &ExtractOptions::default(),
    )
    .ok()?
    .snapshot;

    Some((Some(snapshot), Some(claimed.to_path_buf())))
}

fn preferred_context_snapshot(
    selection: ToolSelection,
    cwd: &Path,
) -> (Option<Snapshot>, Option<PathBuf>) {
    let Some(agent_tool) = selection_agent_tool(selection) else {
        return (None, None);
    };
    let Some(path) = transcript_candidates(agent_tool, cwd).into_iter().next() else {
        return (None, None);
    };
    (extract_snapshot(agent_tool, &path, cwd), Some(path))
}

fn selection_agent_tool(selection: ToolSelection) -> Option<AgentTool> {
    match selection {
        ToolSelection::Claude => Some(AgentTool::Claude),
        ToolSelection::Codex => Some(AgentTool::Codex),
        ToolSelection::Auto => None,
    }
}

fn transcript_candidates(agent_tool: AgentTool, cwd: &Path) -> Vec<PathBuf> {
    match agent_tool {
        AgentTool::Claude => discover_claude_paths(cwd),
        AgentTool::Codex => discover_codex_paths(cwd),
    }
}

fn extract_snapshot(agent_tool: AgentTool, path: &Path, cwd: &Path) -> Option<Snapshot> {
    extract(agent_tool, path, cwd, true, &ExtractOptions::default())
        .ok()
        .map(|output| output.snapshot)
}

fn context_snapshot_for_session_with_claim(
    session: &SessionSnapshot,
    existing_claim: Option<&Path>,
    group_is_ambiguous: bool,
) -> (Option<Snapshot>, Option<PathBuf>) {
    let Some(selection) = tool_selection_for_session(session.tool.as_deref()) else {
        return (None, None);
    };
    let cwd = Path::new(&session.cwd);

    if let Some(claimed_snapshot) = claimed_context_snapshot(selection, cwd, existing_claim) {
        return claimed_snapshot;
    }

    if group_is_ambiguous {
        return (None, None);
    }

    preferred_context_snapshot(selection, cwd)
}

fn is_initial_thought_candidate(state: &SessionRuntimeState, session: &SessionSnapshot) -> bool {
    state.emission_seq == 0 && session.thought.is_none()
}

fn has_adequate_initial_context(
    context_snapshot: Option<&Snapshot>,
    replay_text: &str,
    next_commit_candidate: bool,
    next_action_cues: &[ActionCue],
) -> bool {
    context_snapshot.is_some_and(snapshot_has_meaningful_context)
        || has_meaningful_terminal_delta(replay_text, None)
        || next_commit_candidate
        || !next_action_cues.is_empty()
}

fn snapshot_has_meaningful_context(snapshot: &Snapshot) -> bool {
    snapshot
        .user_task
        .as_deref()
        .map(normalize_for_focus)
        .is_some_and(|task| !task.is_empty())
        || snapshot.current_tool.is_some()
        || !snapshot.recent_actions.is_empty()
}

fn tool_selection_for_session(tool: Option<&str>) -> Option<ToolSelection> {
    let tool = tool?.to_lowercase();
    if tool.contains("claude") {
        Some(ToolSelection::Claude)
    } else if tool.contains("codex") {
        Some(ToolSelection::Codex)
    } else {
        None
    }
}

fn build_context_prompt(
    snapshot: &Snapshot,
    state: &SessionState,
    summary_history: &[String],
    custom_preamble: Option<&str>,
) -> String {
    let mut parts = vec![
        custom_preamble
            .unwrap_or(DEFAULT_AGENT_PREAMBLE)
            .to_string(),
        format!("State: {}", state_label(state)),
    ];
    parts.extend(task_lines(snapshot.user_task.as_deref()));
    parts.extend(summary_history_lines(summary_history));
    parts.extend(action_lines(&snapshot.recent_actions));
    parts.extend(current_tool_lines(snapshot.current_tool.as_ref()));
    parts.extend(context_prompt_tail());
    parts.join("\n")
}

fn task_lines(task: Option<&str>) -> Vec<String> {
    task.map(|task| vec![format!("Task: {task}")])
        .unwrap_or_default()
}

fn summary_history_lines(summary_history: &[String]) -> Vec<String> {
    if summary_history.is_empty() {
        return Vec::new();
    }

    let mut lines = vec!["Recent status:".to_string()];
    lines.extend(
        summary_history
            .iter()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|status| format!("  {status}")),
    );
    lines
}

fn action_lines(actions: &[Action]) -> Vec<String> {
    if actions.is_empty() {
        return Vec::new();
    }

    let mut lines = vec!["Actions:".to_string()];
    lines.extend(actions.iter().map(format_action_line));
    lines
}

fn format_action_line(action: &Action) -> String {
    if action.tool == "said" {
        format!("  said: {}", action.detail.as_deref().unwrap_or_default())
    } else {
        format!(
            "  {}{}",
            action.tool,
            detail_suffix(action.detail.as_deref())
        )
    }
}

fn current_tool_lines(action: Option<&Action>) -> Vec<String> {
    action
        .map(|action| {
            vec![format!(
                "Now: {}{}",
                action.tool,
                detail_suffix(action.detail.as_deref())
            )]
        })
        .unwrap_or_default()
}

fn detail_suffix(detail: Option<&str>) -> String {
    detail.map(|value| format!(": {value}")).unwrap_or_default()
}

fn context_prompt_tail() -> Vec<String> {
    vec![
        String::new(),
        "Write a 1-line status (max 60 chars). Explain the PURPOSE and WHY, not the tool or command.".to_string(),
        "Do not speculate about anticipated future steps.".to_string(),
        "Reply with ONLY the status line, nothing else.".to_string(),
    ]
}

fn build_terminal_prompt(
    context: &str,
    state: &SessionState,
    prev_context: Option<&str>,
    custom_preamble: Option<&str>,
) -> String {
    let preamble = custom_preamble.unwrap_or(DEFAULT_TERMINAL_PREAMBLE);
    let clean = trim_terminal_context(context);
    let clean_prev = prev_context.map(trim_terminal_context);

    let context_block = if let Some(prev) = clean_prev {
        let tail: String = prev
            .chars()
            .rev()
            .take(200)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        match clean.find(&tail) {
            Some(index) => {
                let delta = clean[index + tail.len()..].trim();
                if delta.is_empty() {
                    format!("Screen:\n{clean}")
                } else {
                    format!("New output:\n{delta}")
                }
            }
            None => format!("Screen:\n{clean}"),
        }
    } else {
        format!("Screen:\n{clean}")
    };

    format!(
        "{preamble}\n\
         State: {}\n\
         {context_block}\n\n\
         Write a 1-line status (max 60 chars). Infer the PURPOSE behind what's on screen - WHY is this happening, not WHAT command is running.\n\
         Do not speculate about anticipated future steps.\n\
         Reply with ONLY the status line, nothing else.",
        state_label(state)
    )
}

fn state_label(state: &SessionState) -> &'static str {
    match state {
        SessionState::Idle => "idle",
        SessionState::Busy => "busy",
        SessionState::Error => "error",
        SessionState::Attention => "attention",
        SessionState::Exited => "exited",
    }
}

fn has_meaningful_terminal_delta(current: &str, previous: Option<&str>) -> bool {
    let clean = trim_terminal_context(current);
    let clean_prev = previous.map(trim_terminal_context).unwrap_or_default();

    if clean != clean_prev {
        return changed_non_whitespace_chars(&clean, &clean_prev)
            >= TERMINAL_MIN_MEANINGFUL_DELTA_CHARS;
    }
    false
}

fn changed_non_whitespace_chars(current: &str, previous: &str) -> usize {
    // Iterate char-by-char without materializing Vec<char>. We need a shared
    // prefix and shared suffix in chars (not bytes) so a multibyte char that
    // spans the boundary doesn't get double-counted. Both inputs are bounded
    // by trim_terminal_context() so we trade allocation for two linear scans.
    let prefix_chars = shared_prefix_chars(current, previous);
    let prefix_bytes_cur = char_offset_to_byte(current, prefix_chars);
    let prefix_bytes_prev = char_offset_to_byte(previous, prefix_chars);

    let cur_tail = &current[prefix_bytes_cur..];
    let prev_tail = &previous[prefix_bytes_prev..];
    let suffix_chars = shared_suffix_chars(cur_tail, prev_tail);
    let cur_changed_end = byte_offset_from_end(cur_tail, suffix_chars);

    cur_tail[..cur_changed_end]
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count()
}

fn shared_prefix_chars(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

fn shared_suffix_chars(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

fn char_offset_to_byte(value: &str, char_offset: usize) -> usize {
    value
        .char_indices()
        .nth(char_offset)
        .map(|(byte, _)| byte)
        .unwrap_or(value.len())
}

fn byte_offset_from_end(value: &str, suffix_chars: usize) -> usize {
    let mut iter = value.char_indices().rev();
    let mut last_byte = value.len();
    for _ in 0..suffix_chars {
        match iter.next() {
            Some((byte, _)) => last_byte = byte,
            None => return 0,
        }
    }
    last_byte
}

fn sanitize_thought_text(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    for word in raw.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(word);
    }
    truncate_with_ellipsis(normalized, MAX_THOUGHT_CHARS)
}

/// Returns `text` unchanged when it fits within `max_chars` (counting Unicode
/// code points, not bytes). Otherwise cuts to `max_chars - 3` chars and
/// appends `...`. Walks the string at most three times in the truncation
/// path; the fast path exits after `max_chars` iterations without scanning
/// the rest of the buffer.
fn truncate_with_ellipsis(text: String, max_chars: usize) -> String {
    let mut iter = text.char_indices();
    for _ in 0..=max_chars {
        if iter.next().is_none() {
            return text;
        }
    }
    let cut = text
        .char_indices()
        .nth(max_chars - 3)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len());
    let mut trimmed = String::with_capacity(cut + 3);
    trimmed.push_str(&text[..cut]);
    trimmed.push_str("...");
    trimmed
}

fn is_duplicate_thought(previous: Option<&str>, next: &str) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    normalize_for_compare(previous) == normalize_for_compare(next)
}

fn normalize_for_compare(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn trim_terminal_context(context: &str) -> String {
    let stripped = strip_ansi(context);
    // Walk char_indices from the back to find the byte offset that yields
    // exactly TERMINAL_CONTEXT_CHARS chars of suffix. If we run out, the
    // string is already short enough and we return it whole.
    let mut iter = stripped.char_indices().rev();
    let mut cut = stripped.len();
    for _ in 0..TERMINAL_CONTEXT_CHARS {
        match iter.next() {
            Some((byte, _)) => cut = byte,
            None => return stripped,
        }
    }
    stripped[cut..].to_string()
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => skip_escape_sequence(&mut chars),
            _ if should_skip_control(ch) => continue,
            _ => output.push(ch),
        }
    }
    output
}

fn should_skip_control(ch: char) -> bool {
    ch.is_control() && ch != '\n' && ch != '\t'
}

fn skip_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.peek().copied() {
        Some('[') => skip_csi_sequence(chars),
        Some(']') => skip_osc_sequence(chars),
        Some(_) => {
            chars.next();
        }
        None => {}
    }
}

fn skip_csi_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    chars.next();
    for next in chars.by_ref() {
        if next.is_ascii_alphabetic() || next == '~' {
            break;
        }
    }
}

fn skip_osc_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    chars.next();
    while let Some(next) = chars.next() {
        if next == '\x07' || osc_escape_terminated(next, chars) {
            break;
        }
    }
}

fn osc_escape_terminated(next: char, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    if next != '\x1b' || chars.peek() != Some(&'\\') {
        return false;
    }
    chars.next();
    true
}

fn context_focus_fingerprint(snapshot: &Snapshot, state: &SessionState) -> u64 {
    let mut parts = vec![format!("state={}", state_label(state))];

    push_normalized_focus_part(&mut parts, "task", snapshot.user_task.as_deref());
    push_normalized_focus_part(
        &mut parts,
        "now",
        snapshot
            .current_tool
            .as_ref()
            .map(|tool| tool.tool.as_str()),
    );

    let recent_tools = recent_focus_tools(snapshot);
    if !recent_tools.is_empty() {
        parts.push(format!("recent={}", recent_tools.join(",")));
    }

    hash_string(&parts.join("|"))
}

fn push_normalized_focus_part(parts: &mut Vec<String>, label: &str, raw: Option<&str>) {
    if let Some(value) = raw {
        let normalized = normalize_for_focus(value);
        if !normalized.is_empty() {
            parts.push(format!("{label}={normalized}"));
        }
    }
}

fn recent_focus_tools(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .recent_actions
        .iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|action| normalize_for_focus(&action.tool))
        .filter(|tool| !tool.is_empty())
        .collect()
}

fn terminal_objective_fingerprint(context: &str, state: &SessionState) -> String {
    let clean = strip_ansi(context);
    let preview = clean
        .lines()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("|");

    let material = format!(
        "state={}|{}",
        state_label(state),
        normalize_for_focus(&preview)
    );
    hash_string(&material).to_string()
}

fn normalize_for_focus(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for word in value.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.make_ascii_lowercase();
    // Cover non-ASCII case folding (rare in tool/task labels, but cheap).
    if out.chars().any(|ch| ch.is_uppercase()) {
        out = out.to_lowercase();
    }
    out
}

fn hash_string(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    strip_ansi(value).hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::MutexGuard;

    use chrono::Duration;
    use tempfile::tempdir;

    use super::*;

    /// Acquire the crate-wide process-env lock so engine tests cannot race
    /// model_client tests on `CLAWGS_GROK_BIN` or `OPENROUTER_API_KEY`.
    fn lock_env() -> MutexGuard<'static, ()> {
        crate::test_support::process_env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
    use crate::emit::model_client::{ModelBackend, ModelClient};
    use crate::emit::protocol::{SessionSnapshot, SessionState, SyncRequest, ThoughtConfig};

    #[test]
    fn changed_non_whitespace_chars_handles_multibyte_prefix_and_suffix() {
        // Identical strings — zero changed.
        assert_eq!(changed_non_whitespace_chars("héllo世界", "héllo世界"), 0);
        // Differ in middle, multibyte prefix and suffix shared.
        assert_eq!(
            changed_non_whitespace_chars("héABC世界", "héXYZ世界"),
            3,
            "should count only the 3 changed chars in the middle"
        );
        // Whitespace inside the change is excluded from the count.
        assert_eq!(changed_non_whitespace_chars("a b c", "a x c"), 1);
    }

    #[test]
    fn trim_terminal_context_keeps_suffix_under_limit() {
        // Below TERMINAL_CONTEXT_CHARS, content is preserved verbatim (after
        // ANSI/control stripping).
        let small = "hello";
        assert_eq!(trim_terminal_context(small), small);
    }

    #[test]
    fn sanitize_thought_text_collapses_whitespace_and_truncates() {
        assert_eq!(sanitize_thought_text("  foo   bar\n\tbaz "), "foo bar baz");
        assert_eq!(sanitize_thought_text("   "), "");
        let long = "a".repeat(MAX_THOUGHT_CHARS + 50);
        let trimmed = sanitize_thought_text(&long);
        assert!(trimmed.ends_with("..."));
        assert_eq!(trimmed.chars().count(), MAX_THOUGHT_CHARS);
    }

    struct MockModelClient {
        response: String,
    }

    impl ModelClient for MockModelClient {
        fn complete(&self, _prompt: &str, _model_override: Option<&str>) -> Result<String, String> {
            Ok(self.response.clone())
        }
    }

    struct FailingModelClient {
        error: String,
    }

    impl ModelClient for FailingModelClient {
        fn complete(&self, _prompt: &str, _model_override: Option<&str>) -> Result<String, String> {
            Err(self.error.clone())
        }
    }

    fn mock_engine(response: &str) -> EmitEngine {
        EmitEngine::with_backend(
            Box::new(MockModelClient {
                response: response.to_string(),
            }),
            ModelBackend::OpenRouter,
        )
    }

    fn failing_engine(error: &str) -> EmitEngine {
        EmitEngine::with_backend(
            Box::new(FailingModelClient {
                error: error.to_string(),
            }),
            ModelBackend::OpenRouter,
        )
    }

    fn sample_session(now: DateTime<Utc>) -> SessionSnapshot {
        SessionSnapshot {
            session_id: "sess-1".to_string(),
            state: SessionState::Busy,
            exited: false,
            tool: None,
            cwd: "/tmp/project".to_string(),
            replay_text: concat!(
                "running cargo test --all\n",
                "test auth::login_rejects_missing_token ... FAILED\n",
                "assertion failed: status should stay unauthorized after missing token\n",
                "reviewing auth middleware header parsing and session fallback handling\n"
            )
            .to_string(),
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            objective_fingerprint: None,
            thought_updated_at: None,
            token_count: 1000,
            context_limit: 192_000,
            last_activity_at: now,
            rest_state: RestState::Active,
            commit_candidate: false,
            action_cues: Vec::new(),
        }
    }

    fn awaiting_user_cue() -> crate::ActionCue {
        crate::ActionCue {
            kind: crate::ActionCueKind::AwaitingUser,
            status: crate::ActionCueStatus::Active,
            source: crate::ActionCueSource::Transcript,
            confidence: crate::ActionCueConfidence::Deterministic,
            evidence: vec!["awaiting_user_input".to_string()],
        }
    }

    fn commit_ready_cue() -> crate::ActionCue {
        crate::ActionCue {
            kind: crate::ActionCueKind::CommitReady,
            status: crate::ActionCueStatus::Active,
            source: crate::ActionCueSource::Transcript,
            confidence: crate::ActionCueConfidence::Deterministic,
            evidence: vec![
                "edit_seen".to_string(),
                "validation_succeeded".to_string(),
                "dirty_tree_checked_after_latest_edit".to_string(),
                "commit_not_seen_after_latest_edit".to_string(),
            ],
        }
    }

    fn invalid_awaiting_user_cue() -> crate::ActionCue {
        crate::ActionCue {
            kind: crate::ActionCueKind::AwaitingUser,
            status: crate::ActionCueStatus::Active,
            source: crate::ActionCueSource::Transcript,
            confidence: crate::ActionCueConfidence::Deterministic,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn emits_thought_for_eligible_active_session() {
        let now = Utc::now();
        let mut engine = mock_engine("investigating failing auth tests");

        let request = SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now)],
        };

        let result = engine.sync(&request);
        assert_eq!(result.msg_type, "sync_result");
        assert_eq!(result.updates.len(), 1);
        assert_eq!(
            result.updates[0].thought.as_deref(),
            Some("investigating failing auth tests")
        );
        assert_eq!(result.updates[0].thought_source, ThoughtSource::Llm);
        assert!(!result.stream_instance_id.is_empty());
        assert_eq!(result.metrics.llm_calls, 1);
    }

    #[test]
    fn inbound_action_cues_carry_without_transcript_context() {
        let now = Utc::now();
        let mut engine = mock_engine("investigating failing auth tests");
        let mut session = sample_session(now);
        session.commit_candidate = true;
        session.action_cues = vec![commit_ready_cue()];

        let request = SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session],
        };

        let result = engine.sync(&request);

        assert_eq!(result.updates.len(), 1);
        assert!(result.updates[0].commit_candidate);
        assert_eq!(result.updates[0].action_cues, vec![commit_ready_cue()]);
    }

    #[test]
    fn transcript_without_commit_signal_clears_inbound_commit_candidate() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.commit_candidate = true;
        let context = Snapshot {
            user_task: Some("Claude task".to_string()),
            current_tool: None,
            token_count: 42,
            awaiting_user_input: false,
            awaiting_user_text: None,
            recent_actions: Vec::new(),
            commit_signal: None,
            action_cues: Vec::new(),
        };

        assert!(!commit_candidate_for_context(Some(&context), &session));
    }

    #[test]
    fn first_inbound_awaiting_user_cue_sleeps_without_model_call() {
        let now = Utc::now();
        let mut engine = failing_engine("model backend offline");
        let mut session = sample_session(now);
        session.replay_text = "sh".to_string();
        session.action_cues = vec![awaiting_user_cue()];

        let result = engine.sync(&SyncRequest {
            id: "req-inbound-fail".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.metrics.llm_calls, 0);
        assert_eq!(result.updates[0].action_cues, vec![awaiting_user_cue()]);
        assert_eq!(result.updates[0].rest_state, RestState::Sleeping);
        assert!(result.metrics.last_backend_error.is_none());
    }

    #[test]
    fn first_inbound_commit_candidate_emits_when_model_fails() {
        let now = Utc::now();
        let mut engine = failing_engine("model backend offline");
        let mut session = sample_session(now);
        session.replay_text = "sh".to_string();
        session.commit_candidate = true;

        let result = engine.sync(&SyncRequest {
            id: "req-inbound-commit-fail".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert!(result.updates[0].commit_candidate);
        assert!(result.updates[0].action_cues.is_empty());
        assert_eq!(
            result.metrics.last_backend_error.as_deref(),
            Some("model backend offline")
        );
    }

    #[test]
    fn inbound_awaiting_user_cue_sets_sleeping_rest_state_without_transcript() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.action_cues = vec![awaiting_user_cue()];

        assert_eq!(
            rest_state_for_session(&session, None, now),
            RestState::Sleeping
        );
    }

    #[test]
    fn invalid_inbound_action_cues_are_not_treated_as_facts() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.action_cues = vec![invalid_awaiting_user_cue()];

        assert_eq!(action_cues_for_context(None, &session), Vec::new());
        assert_eq!(
            rest_state_for_session(&session, None, now),
            RestState::Active
        );
    }

    #[test]
    fn initial_context_suppression_emits_passive_clear_for_stale_inbound_cues() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.replay_text = "$ ".to_string();
        session.commit_candidate = true;
        session.action_cues = vec![commit_ready_cue()];
        let mut state = SessionRuntimeState::initialize_from_session(&session, now);
        let context = Snapshot {
            user_task: None,
            current_tool: None,
            token_count: 0,
            awaiting_user_input: false,
            awaiting_user_text: None,
            recent_actions: Vec::new(),
            commit_signal: Some(crate::CommitSignal::default()),
            action_cues: Vec::new(),
        };
        let mut updates = Vec::new();
        let mut metrics = SyncMetrics::default();

        let suppressed = suppress_for_initial_context(
            "stream-test",
            &mut state,
            &session,
            &ThoughtConfig::default(),
            ContextSource::Transcript,
            now,
            Some(&context),
            RestState::Active,
            false,
            &[],
            &mut updates,
            &mut metrics,
        );

        assert!(suppressed);
        assert_eq!(updates.len(), 1);
        assert!(!updates[0].commit_candidate);
        assert!(updates[0].action_cues.is_empty());
        assert!(!state.commit_candidate);
        assert!(state.action_cues.is_empty());
    }

    #[test]
    fn inbound_commit_ready_cue_sets_legacy_commit_candidate_without_transcript() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.commit_candidate = false;
        session.action_cues = vec![commit_ready_cue()];

        assert!(commit_candidate_for_context(None, &session));
    }

    #[test]
    fn exited_session_clears_visible_action_cues_after_model_failure() {
        let now = Utc::now();
        let mut engine = failing_engine("model backend offline");
        let mut active = sample_session(now);
        active.replay_text = "sh".to_string();
        active.commit_candidate = true;
        active.action_cues = vec![commit_ready_cue()];

        let first = engine.sync(&SyncRequest {
            id: "req-active".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![active.clone()],
        });
        assert_eq!(first.updates.len(), 1);
        assert!(first.updates[0].commit_candidate);
        assert_eq!(first.updates[0].action_cues, vec![commit_ready_cue()]);

        let mut exited = active;
        exited.state = SessionState::Exited;
        exited.exited = true;
        exited.commit_candidate = false;
        exited.action_cues.clear();

        let second = engine.sync(&SyncRequest {
            id: "req-exited".to_string(),
            now: now + Duration::seconds(1),
            config: ThoughtConfig::default(),
            sessions: vec![exited],
        });

        assert_eq!(second.updates.len(), 1);
        assert_eq!(second.updates[0].rest_state, RestState::DeepSleep);
        assert!(!second.updates[0].commit_candidate);
        assert!(second.updates[0].action_cues.is_empty());
        assert!(second.updates[0].thought.is_none());
    }

    #[test]
    fn repeat_active_syncs_stay_quiet() {
        let now = Utc::now();
        let mut engine = mock_engine("investigating failing auth tests");

        let first = SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now)],
        };
        let first_result = engine.sync(&first);
        assert_eq!(first_result.updates.len(), 1);
        assert_eq!(first_result.metrics.llm_calls, 1);

        let second = SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(1),
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now + Duration::seconds(1))],
        };
        let second_result = engine.sync(&second);
        assert_eq!(second_result.updates.len(), 0);
        assert_eq!(second_result.metrics.llm_calls, 0);
        assert!(second_result.metrics.suppressed > 0);
    }

    #[test]
    fn model_completion_failure_suppresses_update_and_reports_error() {
        let now = Utc::now();
        let mut engine = failing_engine("grok headless failed: not authenticated");

        let request = SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now)],
        };

        let result = engine.sync(&request);
        assert!(result.updates.is_empty());
        assert_eq!(result.metrics.llm_calls, 0);
        assert_eq!(
            result.metrics.last_backend_error.as_deref(),
            Some("grok headless failed: not authenticated")
        );
        assert!(result.metrics.suppressed > 0);
    }

    #[test]
    fn model_failure_still_emits_action_cue_state_change() {
        let temp = tempdir().expect("tempdir");
        let transcript = temp.path().join("cue-only.jsonl");
        fs::write(
            &transcript,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Ship preview-first widget fix\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n"
            ),
        )
        .expect("write transcript");

        let now = Utc::now();
        let mut engine = failing_engine("model backend offline");
        let mut session = sample_session(now);
        session.tool = Some("codex".to_string());
        session.cwd = "/tmp/project".to_string();

        engine.per_session.insert(
            session.session_id.clone(),
            SessionRuntimeState::initialize_from_session(&session, now),
        );
        engine
            .per_session
            .get_mut(&session.session_id)
            .expect("state")
            .claimed_jsonl_path = Some(transcript);

        let result = engine.sync(&SyncRequest {
            id: "req-cue-only".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.metrics.llm_calls, 0);
        assert_eq!(
            result.updates[0].action_cues[0].kind,
            crate::ActionCueKind::ValidationMissingAfterEdit
        );
        assert!(!result.updates[0].commit_candidate);
        assert_eq!(
            result.metrics.last_backend_error.as_deref(),
            Some("model backend offline")
        );
    }

    #[test]
    fn ensure_client_carries_forward_when_override_backend_credentials_missing() {
        // The default backend has a working mock client, but the request
        // overrides to the Grok CLI backend. With CLAWGS_GROK_BIN pointed at
        // a path that doesn't exist, validate_backend_credentials should fail with
        // allow_carry_forward=true, which means metrics record the error and
        // every active session is carried forward (suppressed) rather than
        // emitted.
        let _lock = lock_env();
        let prior_grok = std::env::var("CLAWGS_GROK_BIN").ok();
        std::env::set_var("CLAWGS_GROK_BIN", "/nonexistent/grok-zzz");

        let now = Utc::now();
        let mut engine = mock_engine("ignored — never invoked");

        let config = ThoughtConfig {
            backend: "claude".to_string(),
            ..ThoughtConfig::default()
        };
        let request = SyncRequest {
            id: "req-1".to_string(),
            now,
            config,
            sessions: vec![sample_session(now)],
        };
        let result = engine.sync(&request);

        assert!(
            result.updates.is_empty(),
            "no LLM emits when client init fails"
        );
        assert_eq!(result.metrics.llm_calls, 0);
        assert!(
            result
                .metrics
                .last_backend_error
                .as_deref()
                .is_some_and(|err| err.starts_with("grok:") && err.contains("not found")),
            "expected grok credential error, got {:?}",
            result.metrics.last_backend_error
        );
        assert!(
            result.metrics.suppressed > 0,
            "carry-forward must record at least one suppressed session"
        );

        match prior_grok {
            Some(value) => std::env::set_var("CLAWGS_GROK_BIN", value),
            None => std::env::remove_var("CLAWGS_GROK_BIN"),
        }
    }

    #[test]
    fn credential_failure_still_emits_action_cue_state_change() {
        let _lock = lock_env();
        let prior_grok = std::env::var("CLAWGS_GROK_BIN").ok();
        std::env::set_var("CLAWGS_GROK_BIN", "/nonexistent/grok-zzz");

        let temp = tempdir().expect("tempdir");
        let transcript = temp.path().join("cue-only.jsonl");
        fs::write(
            &transcript,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Ship preview-first widget fix\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n"
            ),
        )
        .expect("write transcript");

        let now = Utc::now();
        let mut engine = mock_engine("ignored");
        let mut session = sample_session(now);
        session.tool = Some("codex".to_string());
        session.cwd = "/tmp/project".to_string();

        engine.per_session.insert(
            session.session_id.clone(),
            SessionRuntimeState::initialize_from_session(&session, now),
        );
        engine
            .per_session
            .get_mut(&session.session_id)
            .expect("state")
            .claimed_jsonl_path = Some(transcript);

        let result = engine.sync(&SyncRequest {
            id: "req-credential-cue".to_string(),
            now,
            config: ThoughtConfig {
                backend: "claude".to_string(),
                ..ThoughtConfig::default()
            },
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.metrics.llm_calls, 0);
        assert!(
            result
                .metrics
                .last_backend_error
                .as_deref()
                .is_some_and(|err| err.starts_with("grok:") && err.contains("not found")),
            "expected grok credential error, got {:?}",
            result.metrics.last_backend_error
        );
        assert_eq!(
            result.updates[0].action_cues[0].kind,
            crate::ActionCueKind::ValidationMissingAfterEdit
        );

        match prior_grok {
            Some(value) => std::env::set_var("CLAWGS_GROK_BIN", value),
            None => std::env::remove_var("CLAWGS_GROK_BIN"),
        }
    }

    #[test]
    fn credential_failure_emits_first_inbound_action_cue() {
        let _lock = lock_env();
        let prior_grok = std::env::var("CLAWGS_GROK_BIN").ok();
        std::env::set_var("CLAWGS_GROK_BIN", "/nonexistent/grok-zzz");

        let now = Utc::now();
        let mut engine = mock_engine("ignored");
        let mut session = sample_session(now);
        session.replay_text = "sh".to_string();
        session.action_cues = vec![awaiting_user_cue()];

        let result = engine.sync(&SyncRequest {
            id: "req-credential-inbound-cue".to_string(),
            now,
            config: ThoughtConfig {
                backend: "claude".to_string(),
                ..ThoughtConfig::default()
            },
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.metrics.llm_calls, 0);
        assert!(
            result
                .metrics
                .last_backend_error
                .as_deref()
                .is_some_and(|err| err.starts_with("grok:") && err.contains("not found")),
            "expected grok credential error, got {:?}",
            result.metrics.last_backend_error
        );
        assert_eq!(result.updates[0].action_cues, vec![awaiting_user_cue()]);
        assert_eq!(result.updates[0].rest_state, RestState::Sleeping);
        assert_eq!(result.updates[0].thought_state, ThoughtState::Sleeping);

        match prior_grok {
            Some(value) => std::env::set_var("CLAWGS_GROK_BIN", value),
            None => std::env::remove_var("CLAWGS_GROK_BIN"),
        }
    }

    #[test]
    fn credential_failure_wake_clears_stale_sleeping_thought() {
        let _lock = lock_env();
        let prior_grok = std::env::var("CLAWGS_GROK_BIN").ok();
        std::env::set_var("CLAWGS_GROK_BIN", "/nonexistent/grok-zzz");

        let now = Utc::now();
        let mut engine = mock_engine("ignored");
        let mut session = sample_session(now);
        session.thought = Some("Waiting on your confirmation.".to_string());
        session.thought_state = ThoughtState::Sleeping;
        session.rest_state = RestState::Sleeping;
        engine.per_session.insert(
            session.session_id.clone(),
            SessionRuntimeState::initialize_from_session(&session, now),
        );

        let mut resumed = session;
        resumed.thought = None;
        resumed.thought_state = ThoughtState::Holding;
        resumed.rest_state = RestState::Active;
        resumed.replay_text = "cargo test resumed after user input".to_string();

        let result = engine.sync(&SyncRequest {
            id: "req-credential-wake".to_string(),
            now,
            config: ThoughtConfig {
                backend: "claude".to_string(),
                ..ThoughtConfig::default()
            },
            sessions: vec![resumed],
        });

        assert_eq!(result.updates.len(), 1);
        assert!(result.updates[0].thought.is_none());
        assert_eq!(result.updates[0].rest_state, RestState::Active);
        assert_eq!(result.updates[0].thought_state, ThoughtState::Holding);

        match prior_grok {
            Some(value) => std::env::set_var("CLAWGS_GROK_BIN", value),
            None => std::env::remove_var("CLAWGS_GROK_BIN"),
        }
    }

    #[test]
    fn ensure_client_builds_real_client_when_credentials_validate() {
        // Default backend has a mock client; the request overrides to Grok CLI.
        // With CLAWGS_GROK_BIN pointed at /usr/bin/true, validate_backend_credentials
        // succeeds and build_model_client_for actually constructs a
        // GrokCliModelClient. The subsequent complete() call against
        // /usr/bin/true exits 0 with empty stdout and surfaces an error,
        // which is enough to confirm the build branch executed.
        let _lock = lock_env();
        let prior_grok = std::env::var("CLAWGS_GROK_BIN").ok();
        std::env::set_var("CLAWGS_GROK_BIN", "/usr/bin/true");

        let now = Utc::now();
        let mut engine = mock_engine("never invoked because override picks grok");
        let config = ThoughtConfig {
            backend: "claude".to_string(),
            ..ThoughtConfig::default()
        };
        let request = SyncRequest {
            id: "req-build".to_string(),
            now,
            config,
            sessions: vec![sample_session(now)],
        };

        let result = engine.sync(&request);
        // build_model_client_for(Grok) ran, complete() then ran on /usr/bin/true.
        // Either it surfaced a backend_error from the empty CLI output or it
        // emitted a thought (unlikely for /usr/bin/true). Either way,
        // ensure_client did not short-circuit on credentials.
        assert!(
            result
                .metrics
                .last_backend_error
                .as_deref()
                .is_none_or(|err| !err.contains("not found")),
            "expected ensure_client to pass validation, got {:?}",
            result.metrics.last_backend_error
        );

        match prior_grok {
            Some(value) => std::env::set_var("CLAWGS_GROK_BIN", value),
            None => std::env::remove_var("CLAWGS_GROK_BIN"),
        }
    }

    #[test]
    fn idle_rest_state_caps_idle_sessions_at_drowsy() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.state = SessionState::Idle;

        session.last_activity_at = now - Duration::milliseconds(9_999);
        assert_eq!(
            rest_state_for_session(&session, None, now),
            RestState::Active
        );

        session.last_activity_at = now - Duration::milliseconds(10_000);
        assert_eq!(
            rest_state_for_session(&session, None, now),
            RestState::Drowsy
        );

        session.last_activity_at = now - Duration::milliseconds(30_000);
        assert_eq!(
            rest_state_for_session(&session, None, now),
            RestState::Drowsy
        );

        session.last_activity_at = now - Duration::milliseconds(60_000);
        assert_eq!(
            rest_state_for_session(&session, None, now),
            RestState::Drowsy
        );
    }

    #[test]
    fn awaiting_user_context_sleeps_immediately() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.state = SessionState::Idle;
        let context = Snapshot {
            user_task: None,
            current_tool: None,
            token_count: 0,
            awaiting_user_input: true,
            awaiting_user_text: Some("Need your approval to continue.".to_string()),
            recent_actions: Vec::new(),
            commit_signal: None,
            action_cues: Vec::new(),
        };

        assert_eq!(
            rest_state_for_session(&session, Some(&context), now),
            RestState::Sleeping
        );
    }

    #[test]
    fn sleeping_transition_uses_transcript_reply_text() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.state = SessionState::Attention;

        let mut state = SessionRuntimeState::initialize_from_session(&session, now);
        let mut updates = Vec::new();
        let mut metrics = SyncMetrics::default();
        let context = Snapshot {
            user_task: None,
            current_tool: None,
            token_count: 42,
            awaiting_user_input: true,
            awaiting_user_text: Some("Need your decision on the migration.".to_string()),
            recent_actions: Vec::new(),
            commit_signal: None,
            action_cues: Vec::new(),
        };
        let action_cues = vec![awaiting_user_cue()];

        let handled = handle_sleeping_session(
            "stream-a",
            &mut state,
            &session,
            Some(&context),
            &ThoughtConfig::default(),
            ContextSource::Transcript,
            now,
            RestState::Sleeping,
            false,
            &action_cues,
            &mut updates,
            &mut metrics,
        );

        assert!(handled);
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0].thought.as_deref(),
            Some("Need your decision on the migration.")
        );
        assert_eq!(
            updates[0].token_count, 42,
            "sleeping updates driven by transcript context should preserve transcript token count"
        );
        assert_eq!(updates[0].thought_state, ThoughtState::Sleeping);
        assert_eq!(updates[0].thought_source, ThoughtSource::CarryForward);
        assert_eq!(updates[0].rest_state, RestState::Sleeping);
        assert_eq!(updates[0].action_cues, action_cues);
        let timing = updates[0].timing.as_ref().expect("timing");
        assert_eq!(timing.run_finished_at, Some(now));
    }

    #[test]
    fn emitted_sleeping_update_serializes_rest_state_explicitly() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.state = SessionState::Attention;
        session.thought = Some("Waiting on your confirmation.".to_string());
        let mut state = SessionRuntimeState::initialize_from_session(&session, now);
        let mut updates = Vec::new();
        let mut metrics = SyncMetrics::default();

        handle_sleeping_session(
            "stream-a",
            &mut state,
            &session,
            None,
            &ThoughtConfig::default(),
            ContextSource::Transcript,
            now,
            RestState::Sleeping,
            false,
            &[],
            &mut updates,
            &mut metrics,
        );

        let serialized =
            serde_json::to_value(&updates[0]).expect("sleeping update should serialize");
        assert_eq!(
            serialized
                .get("rest_state")
                .and_then(|value| value.as_str()),
            Some("sleeping")
        );
        assert_eq!(
            serialized.get("thought").and_then(|value| value.as_str()),
            Some("Waiting on your confirmation.")
        );
    }

    #[test]
    fn disabled_config_clears_existing_thought() {
        let now = Utc::now();
        let mut engine = mock_engine("unused");

        let mut session = sample_session(now);
        session.thought = Some("existing thought".to_string());
        session.thought_state = ThoughtState::Active;

        let config = ThoughtConfig {
            enabled: false,
            ..ThoughtConfig::default()
        };

        let request = SyncRequest {
            id: "req-1".to_string(),
            now,
            config,
            sessions: vec![session],
        };

        let result = engine.sync(&request);
        assert_eq!(result.updates.len(), 1);
        assert!(result.updates[0].thought.is_none());
        assert_eq!(result.updates[0].thought_state, ThoughtState::Holding);
        assert!(!result.updates[0].commit_candidate);
    }

    #[test]
    fn disabled_config_clears_awaiting_user_cue_without_sleep_mismatch() {
        let now = Utc::now();
        let mut engine = mock_engine("unused");

        let mut session = sample_session(now);
        session.action_cues = vec![awaiting_user_cue()];

        let result = engine.sync(&SyncRequest {
            id: "req-disabled-cue".to_string(),
            now,
            config: ThoughtConfig {
                enabled: false,
                ..ThoughtConfig::default()
            },
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.updates[0].rest_state, RestState::Active);
        assert!(result.updates[0].action_cues.is_empty());
    }

    #[test]
    fn duplicate_thought_still_emits_commit_candidate_change() {
        let cwd = PathBuf::from("/tmp/project");
        let temp = tempdir().expect("tempdir");
        let transcript = temp.path().join("candidate.jsonl");

        let now = Utc::now();
        let mut engine = mock_engine("stabilizing preview widget behavior");
        let mut session = sample_session(now);

        let first = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session.clone()],
        });
        assert_eq!(first.updates.len(), 1);
        assert_eq!(first.metrics.llm_calls, 1);

        engine
            .per_session
            .get_mut("sess-1")
            .expect("state should exist")
            .claimed_jsonl_path = Some(transcript.clone());

        fs::write(
            &transcript,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Ship preview-first widget fix\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test --manifest-path clawgs/Cargo.toml codex -- --nocapture\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: abc123\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nvalidation passed\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_fresh_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_fresh_status\",\"output\":\"Chunk ID: status\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M src/widget.tsx\\n\"}}\n"
            ),
        )
        .expect("rewrite transcript");

        session.tool = Some("codex".to_string());
        session.cwd = cwd.display().to_string();
        session.replay_text = "cargo test --manifest-path clawgs/Cargo.toml".to_string();

        let second = engine.sync(&SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(60),
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });
        assert_eq!(second.updates.len(), 1);
        assert_eq!(
            second.updates[0].thought.as_deref(),
            Some("stabilizing preview widget behavior")
        );
        assert!(second.updates[0].commit_candidate);
        assert_eq!(
            second.updates[0].action_cues[0].kind,
            crate::ActionCueKind::CommitReady
        );
        assert_eq!(second.updates[0].emission_seq, Some(2));
    }

    #[test]
    fn claimed_path_persists_across_sync_ticks() {
        let now = Utc::now();
        let mut engine = mock_engine("working on tests");

        // First sync — no tool set, so no JSONL claim, but state is created
        let request = SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now)],
        };
        engine.sync(&request);

        // Manually inject a claimed path to simulate a successful discovery
        let fake_path = PathBuf::from("/tmp/fake-session.jsonl");
        engine
            .per_session
            .get_mut("sess-1")
            .unwrap()
            .claimed_jsonl_path = Some(fake_path.clone());

        // Second sync — state should retain the claimed path
        let request2 = SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(60),
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now + Duration::seconds(60))],
        };
        engine.sync(&request2);

        // The state persists (not reset) — claimed_jsonl_path may be cleared
        // by the sync logic since the file doesn't exist, but the state entry
        // itself survives across ticks.
        assert!(
            engine.per_session.contains_key("sess-1"),
            "state should persist across sync ticks"
        );
    }

    #[test]
    fn session_removal_drops_claim() {
        let now = Utc::now();
        let mut engine = mock_engine("working");

        // First sync creates state for sess-1
        let request = SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now)],
        };
        engine.sync(&request);
        assert!(engine.per_session.contains_key("sess-1"));

        // Second sync with no sessions — retain() should drop sess-1
        let request2 = SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(1),
            config: ThoughtConfig::default(),
            sessions: vec![],
        };
        let result = engine.sync(&request2);
        assert!(
            !engine.per_session.contains_key("sess-1"),
            "state and claim should be dropped when session removed"
        );
        assert_eq!(result.session_deltas.len(), 1);
        assert_eq!(result.session_deltas[0].session_id, "sess-1");
        assert_eq!(result.session_deltas[0].kind, SessionDeltaKind::Removed);
    }

    #[test]
    fn session_deltas_report_started_changed_and_unchanged() {
        let now = Utc::now();
        let mut engine = mock_engine("disabled");
        let config = ThoughtConfig {
            enabled: false,
            ..ThoughtConfig::default()
        };

        let first = sample_session(now);
        let first_result = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: config.clone(),
            sessions: vec![first.clone()],
        });
        assert_eq!(first_result.session_deltas.len(), 1);
        assert_eq!(
            first_result.session_deltas[0].kind,
            SessionDeltaKind::Started
        );
        assert!(first_result.session_deltas[0].changed_fields.is_empty());

        let mut changed = first;
        changed.replay_text.push_str("\nnew output");
        changed.last_activity_at = now + Duration::seconds(5);
        let changed_result = engine.sync(&SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(5),
            config: config.clone(),
            sessions: vec![changed.clone()],
        });
        assert_eq!(
            changed_result.session_deltas[0].kind,
            SessionDeltaKind::Changed
        );
        assert_eq!(
            changed_result.session_deltas[0].changed_fields,
            vec![SessionDeltaField::ReplayText, SessionDeltaField::Activity]
        );

        let unchanged_result = engine.sync(&SyncRequest {
            id: "req-3".to_string(),
            now: now + Duration::seconds(6),
            config,
            sessions: vec![changed],
        });
        assert_eq!(
            unchanged_result.session_deltas[0].kind,
            SessionDeltaKind::Unchanged
        );
        assert!(unchanged_result.session_deltas[0].changed_fields.is_empty());
    }

    #[test]
    fn no_op_scan_does_not_advance_emission_seq() {
        let now = Utc::now();
        let mut session = sample_session(now);
        session.state = SessionState::Attention;
        session.thought = Some("Waiting on approval.".to_string());
        session.thought_source = ThoughtSource::CarryForward;

        let mut state = SessionRuntimeState::initialize_from_session(&session, now);
        let mut first_updates = Vec::new();
        let mut first_metrics = SyncMetrics::default();

        handle_sleeping_session(
            "stream-a",
            &mut state,
            &session,
            None,
            &ThoughtConfig::default(),
            ContextSource::Transcript,
            now,
            RestState::Sleeping,
            false,
            &[],
            &mut first_updates,
            &mut first_metrics,
        );
        assert_eq!(first_updates.len(), 1);
        assert_eq!(first_updates[0].emission_seq, Some(1));

        let mut second_updates = Vec::new();
        let mut second_metrics = SyncMetrics::default();
        handle_sleeping_session(
            "stream-a",
            &mut state,
            &session,
            None,
            &ThoughtConfig::default(),
            ContextSource::Transcript,
            now + Duration::seconds(1),
            RestState::Sleeping,
            false,
            &[],
            &mut second_updates,
            &mut second_metrics,
        );
        assert!(second_updates.is_empty());
        assert_eq!(state.emission_seq, 1);
    }

    #[test]
    fn waking_from_sleep_clears_waiting_thought() {
        let now = Utc::now();
        let mut engine = mock_engine("working through auth fallback");

        let active = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now)],
        });
        assert_eq!(active.updates.len(), 1);

        let sleeping_at = now + Duration::seconds(31);
        let state = engine
            .per_session
            .get_mut("sess-1")
            .expect("state should exist after first sync");
        state.thought_state = ThoughtState::Sleeping;
        state.thought_source = ThoughtSource::CarryForward;
        state.rest_state = RestState::Sleeping;
        state.sleeping_emitted = true;
        state.last_emitted_thought = Some("Waiting on your reply.".to_string());
        state.run_finished_at = Some(sleeping_at);

        let waking_now = now + Duration::seconds(32);
        let waking = sample_session(waking_now);
        let waking_result = engine.sync(&SyncRequest {
            id: "req-2".to_string(),
            now: waking_now,
            config: ThoughtConfig::default(),
            sessions: vec![waking],
        });

        assert_eq!(waking_result.updates.len(), 2);
        let clear_timing = waking_result.updates[0]
            .timing
            .as_ref()
            .expect("clear timing");
        assert_eq!(clear_timing.run_started_at, waking_now);
        assert!(clear_timing.run_finished_at.is_none());
        assert_eq!(clear_timing.run_elapsed_ms, 0);

        let clear_cues = waking_result.updates[0].cues.as_ref().expect("clear cues");
        assert_eq!(clear_cues.cadence_tier, CadenceTier::Hot);
        assert_eq!(clear_cues.next_llm_eligible_at, waking_now);
        assert!(waking_result.updates[0].thought.is_none());
        assert_eq!(
            waking_result.updates[1].thought.as_deref(),
            Some("working through auth fallback")
        );
    }

    #[test]
    fn multiple_historical_transcripts_bind_newest_candidate() {
        let _lock = crate::test_support::home_env_lock()
            .lock()
            .expect("home lock");
        let home = tempdir().expect("tempdir");
        std::env::set_var("HOME", home.path());

        let cwd = PathBuf::from("/tmp/shared");
        let codex_day = home
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("03")
            .join("08");
        fs::create_dir_all(&codex_day).expect("create codex dir");
        fs::write(
            codex_day.join("rollout-a.jsonl"),
            format!(
                concat!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                    "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"old rollout\"}}}}\n",
                    "{{\"type\":\"response\",\"payload\":{{\"usage\":{{\"input_tokens\":111}}}}}}\n"
                ),
                cwd.display()
            ),
        )
        .expect("write older codex transcript");
        fs::write(
            codex_day.join("rollout-b.jsonl"),
            format!(
                concat!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                    "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"new rollout\"}}}}\n",
                    "{{\"type\":\"response\",\"payload\":{{\"usage\":{{\"input_tokens\":222}}}}}}\n"
                ),
                cwd.display()
            ),
        )
        .expect("write newer codex transcript");

        let now = Utc::now();
        let mut engine = mock_engine("watching logs");
        let mut session = sample_session(now);
        session.session_id = "tmux:work:1.0:%1".to_string();
        session.tool = Some("codex".to_string());
        session.cwd = cwd.display().to_string();
        session.replay_text = "tail -f logs/app.log".to_string();

        let result = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.metrics.llm_calls, 1);
        let state = engine
            .per_session
            .get("tmux:work:1.0:%1")
            .expect("pane state should exist");
        assert_eq!(
            state
                .claimed_jsonl_path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("rollout-b.jsonl")
        );
    }

    #[test]
    fn unusable_claimed_transcript_falls_back_to_valid_candidate() {
        let _lock = crate::test_support::home_env_lock()
            .lock()
            .expect("home lock");
        let home = tempdir().expect("tempdir");
        std::env::set_var("HOME", home.path());

        let cwd = PathBuf::from("/tmp/project-with-stale-claim");
        let codex_day = home
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("03")
            .join("08");
        fs::create_dir_all(&codex_day).expect("create codex dir");
        let valid = codex_day.join("rollout-valid.jsonl");
        fs::write(
            &valid,
            format!(
                concat!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                    "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"fresh valid transcript\"}}}}\n"
                ),
                cwd.display()
            ),
        )
        .expect("write valid codex transcript");
        let bad_claim = home.path().join("stale-claim.jsonl");
        fs::create_dir(&bad_claim).expect("create unreadable claim substitute");

        let now = Utc::now();
        let mut session = sample_session(now);
        session.tool = Some("codex".to_string());
        session.cwd = cwd.display().to_string();

        let (snapshot, resolved_path) =
            context_snapshot_for_session_with_claim(&session, Some(&bad_claim), false);

        assert_eq!(resolved_path, Some(valid));
        assert_eq!(
            snapshot.and_then(|snapshot| snapshot.user_task),
            Some("fresh valid transcript".to_string())
        );
    }

    #[test]
    fn multiple_live_sessions_same_cwd_fall_back_to_terminal_only() {
        let _lock = crate::test_support::home_env_lock()
            .lock()
            .expect("home lock");
        let home = tempdir().expect("tempdir");
        std::env::set_var("HOME", home.path());

        let cwd = PathBuf::from("/tmp/shared");
        let codex_day = home
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("03")
            .join("08");
        fs::create_dir_all(&codex_day).expect("create codex dir");
        fs::write(
            codex_day.join("rollout-a.jsonl"),
            format!(
                concat!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                    "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"shared rollout\"}}}}\n"
                ),
                cwd.display()
            ),
        )
        .expect("write codex transcript");

        let now = Utc::now();
        let mut engine = mock_engine("watching logs");
        let mut first = sample_session(now);
        first.session_id = "tmux:work:1.0:%1".to_string();
        first.tool = Some("codex".to_string());
        first.cwd = cwd.display().to_string();
        first.replay_text = "$ ".to_string();

        let mut second = first.clone();
        second.session_id = "tmux:work:1.0:%2".to_string();

        let result = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![first, second],
        });

        assert!(result.updates.is_empty());
        assert!(result.metrics.suppressed > 0);
        assert_eq!(result.session_deltas.len(), 2);
        assert!(result
            .session_deltas
            .iter()
            .all(|delta| delta.transcript_ambiguous));
        assert!(engine
            .per_session
            .get("tmux:work:1.0:%1")
            .expect("first pane state should exist")
            .claimed_jsonl_path
            .is_none());
        assert!(engine
            .per_session
            .get("tmux:work:1.0:%2")
            .expect("second pane state should exist")
            .claimed_jsonl_path
            .is_none());
    }

    #[test]
    fn short_bootstrap_terminal_context_suppresses_first_thought() {
        let now = Utc::now();
        let mut engine = mock_engine("too early");
        let mut session = sample_session(now);
        session.replay_text = "$ ".to_string();

        let result = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });

        assert!(result.updates.is_empty());
        assert!(result.metrics.suppressed > 0);
        assert_eq!(
            engine
                .per_session
                .get("sess-1")
                .expect("state should exist")
                .emission_seq,
            0
        );
    }

    #[test]
    fn meaningful_terminal_output_emits_thought() {
        let now = Utc::now();
        let mut engine = mock_engine("isolating auth regression");
        let mut bootstrap = sample_session(now);
        bootstrap.replay_text = "$ ".to_string();

        let first = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![bootstrap],
        });
        assert!(first.updates.is_empty());

        let second = engine.sync(&SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(1),
            config: ThoughtConfig::default(),
            sessions: vec![sample_session(now + Duration::seconds(1))],
        });

        assert_eq!(second.updates.len(), 1);
        assert_eq!(
            second.updates[0].thought.as_deref(),
            Some("isolating auth regression")
        );
        assert_eq!(second.metrics.llm_calls, 1);
    }

    #[test]
    fn rest_state_transition_emits_passive_update_without_thought() {
        let now = Utc::now();
        let mut engine = mock_engine("reviewing auth fallback");
        let mut first_session = sample_session(now);
        first_session.state = SessionState::Attention;

        let first = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![first_session],
        });
        assert_eq!(first.updates.len(), 1);

        let mut second_session = sample_session(now + Duration::seconds(20));
        second_session.state = SessionState::Attention;
        second_session.last_activity_at = now + Duration::seconds(5);

        let second = engine.sync(&SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(20),
            config: ThoughtConfig::default(),
            sessions: vec![second_session],
        });

        assert_eq!(second.updates.len(), 1);
        assert_eq!(
            second.updates[0].thought.as_deref(),
            Some("reviewing auth fallback")
        );
        assert_eq!(second.updates[0].rest_state, RestState::Drowsy);
        assert_eq!(second.updates[0].thought_state, ThoughtState::Active);
    }

    #[test]
    fn strip_ansi_removes_escape_sequences_and_controls() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("\x1b]0;title\x07hello"), "hello");
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\hello"), "hello");
        assert_eq!(strip_ansi("a\x08b\n\tc\x1bX!"), "ab\n\tc!");
    }

    #[test]
    fn unique_transcript_context_claims_path_without_emitting_thought() {
        let _lock = crate::test_support::home_env_lock()
            .lock()
            .expect("home lock");
        let home = tempdir().expect("tempdir");
        std::env::set_var("HOME", home.path());

        let codex_day = home
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("03")
            .join("08");
        fs::create_dir_all(&codex_day).expect("create codex dir");
        fs::write(
            codex_day.join("rollout-a.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Fix auth regression\"}}\n",
                "{\"type\":\"response\",\"payload\":{\"usage\":{\"input_tokens\":456}}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"command\\\":\\\"cargo test auth\\\"}\"}}\n"
            ),
        )
        .expect("write codex transcript");

        let now = Utc::now();
        let mut engine = mock_engine("fixing auth regression");
        let mut session = sample_session(now);
        session.session_id = "tmux:work:1.0:%1".to_string();
        session.tool = Some("codex".to_string());
        session.replay_text = "$ ".to_string();

        let result = engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });

        assert_eq!(result.updates.len(), 1);
        assert_eq!(
            result.updates[0].thought.as_deref(),
            Some("fixing auth regression")
        );
        assert_eq!(
            engine
                .per_session
                .get("tmux:work:1.0:%1")
                .expect("pane state")
                .claimed_jsonl_path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("rollout-a.jsonl")
        );
    }

    #[test]
    fn cwd_change_invalidates_claimed_jsonl_path() {
        let now = Utc::now();
        let mut engine = mock_engine("working on project-a");

        let mut session = sample_session(now);
        session.session_id = "tmux:9:1.0:%5".to_string();
        session.cwd = "/tmp/project-a".to_string();

        // First sync — creates state
        engine.sync(&SyncRequest {
            id: "req-1".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session.clone()],
        });

        // Manually inject a claimed path to simulate successful discovery
        let stale_path = PathBuf::from("/tmp/fake-project-a.jsonl");
        let state = engine
            .per_session
            .get_mut("tmux:9:1.0:%5")
            .expect("state should exist");
        state.claimed_jsonl_path = Some(stale_path.clone());
        state.claimed_cwd = Some("/tmp/project-a".to_string());

        // Second sync — same CWD, claim should persist
        session.last_activity_at = now + Duration::seconds(10);
        engine.sync(&SyncRequest {
            id: "req-2".to_string(),
            now: now + Duration::seconds(10),
            config: ThoughtConfig::default(),
            sessions: vec![session.clone()],
        });
        // Claim file doesn't exist so resolved_path is None, but the key thing
        // is the invalidation logic didn't run (claimed_cwd matched).

        // Re-inject claim for the CWD-change test
        let state = engine
            .per_session
            .get_mut("tmux:9:1.0:%5")
            .expect("state should exist");
        state.claimed_jsonl_path = Some(stale_path);
        state.claimed_cwd = Some("/tmp/project-a".to_string());

        // Third sync — CWD changed, claim must be invalidated
        session.cwd = "/tmp/project-b".to_string();
        session.last_activity_at = now + Duration::seconds(20);
        engine.sync(&SyncRequest {
            id: "req-3".to_string(),
            now: now + Duration::seconds(20),
            config: ThoughtConfig::default(),
            sessions: vec![session],
        });

        let state = engine
            .per_session
            .get("tmux:9:1.0:%5")
            .expect("state should exist after CWD change");
        assert!(
            state.claimed_jsonl_path.is_none(),
            "claimed_jsonl_path must be cleared when CWD changes"
        );
        assert!(
            state.claimed_cwd.is_none() || state.claimed_cwd.as_deref() != Some("/tmp/project-a"),
            "claimed_cwd must not reference the old CWD"
        );
    }

    #[test]
    fn compute_transcript_group_counts_skips_singletons() {
        let now = Utc::now();
        let counts = compute_transcript_group_counts(&[sample_session(now)]);
        assert!(
            counts.is_empty(),
            "≤ 1 session can never share a (tool, cwd) group"
        );
    }

    #[test]
    fn compute_transcript_group_counts_tallies_shared_keys() {
        let now = Utc::now();
        let mut a = sample_session(now);
        a.session_id = "a".to_string();
        a.tool = Some("claude".to_string());
        a.cwd = "/tmp/x".to_string();
        let mut b = sample_session(now);
        b.session_id = "b".to_string();
        b.tool = Some("claude".to_string());
        b.cwd = "/tmp/x".to_string();
        let mut c = sample_session(now);
        c.session_id = "c".to_string();
        c.tool = Some("codex".to_string());
        c.cwd = "/tmp/y".to_string();
        let counts = compute_transcript_group_counts(&[a, b, c]);
        let max = counts.values().copied().max().unwrap_or(0);
        assert_eq!(max, 2, "two sessions share the (claude, /tmp/x) group");
    }

    #[test]
    fn session_needs_clear_returns_false_for_idle_holding_state() {
        let now = Utc::now();
        let session = sample_session(now);
        let mut state = SessionRuntimeState::initialize_from_session(&session, now);
        state.thought_state = ThoughtState::Holding;
        state.rest_state = RestState::Active;
        state.commit_candidate = false;
        state.action_cues.clear();
        let mut idle_session = session.clone();
        idle_session.thought = None;
        idle_session.commit_candidate = false;
        idle_session.action_cues.clear();
        assert!(!session_needs_clear(
            &state,
            &idle_session,
            RestState::Active
        ));
    }

    #[test]
    fn session_needs_clear_triggers_on_each_dirty_signal() {
        let now = Utc::now();
        let session = sample_session(now);
        let mut clean_session = session.clone();
        clean_session.thought = None;
        clean_session.commit_candidate = false;
        clean_session.action_cues.clear();

        // last_emitted_thought was set by a prior tick.
        let mut state = SessionRuntimeState::initialize_from_session(&session, now);
        state.last_emitted_thought = Some("stale".to_string());
        assert!(session_needs_clear(
            &state,
            &clean_session,
            RestState::Active
        ));

        // session carries a thought that needs to be cleared.
        let base_state = SessionRuntimeState::initialize_from_session(&session, now);
        let mut dirty_session = clean_session.clone();
        dirty_session.thought = Some("hello".to_string());
        assert!(session_needs_clear(
            &base_state,
            &dirty_session,
            RestState::Active
        ));

        // rest_state mismatch triggers a clear.
        let mut state = SessionRuntimeState::initialize_from_session(&session, now);
        state.rest_state = RestState::Drowsy;
        assert!(session_needs_clear(
            &state,
            &clean_session,
            RestState::Active
        ));

        // commit_candidate flips trigger a clear.
        let base_state = SessionRuntimeState::initialize_from_session(&session, now);
        let mut commit_session = clean_session.clone();
        commit_session.commit_candidate = true;
        assert!(session_needs_clear(
            &base_state,
            &commit_session,
            RestState::Active
        ));

        // action_cues are also visible state that a disabled engine must clear.
        let base_state = SessionRuntimeState::initialize_from_session(&session, now);
        let mut cue_session = clean_session.clone();
        cue_session.action_cues = vec![awaiting_user_cue()];
        assert!(session_needs_clear(
            &base_state,
            &cue_session,
            RestState::Active
        ));
    }

    #[test]
    fn carry_forward_sessions_initializes_state_and_marks_suppressed() {
        let now = Utc::now();
        let mut engine = mock_engine("noop");
        let session = sample_session(now);
        let request = SyncRequest {
            id: "r".to_string(),
            now,
            config: ThoughtConfig::default(),
            sessions: vec![session.clone()],
        };
        let mut metrics = SyncMetrics::default();
        let mut updates = Vec::new();
        let transcript_group_counts = HashMap::new();

        engine.carry_forward_sessions(
            &request,
            &transcript_group_counts,
            &mut updates,
            &mut metrics,
        );

        assert_eq!(metrics.sessions_seen, 1);
        assert_eq!(metrics.suppressed, 1);
        assert!(updates.is_empty());
        assert!(
            engine.per_session.contains_key(&session.session_id),
            "carry-forward must initialize per-session state for unknown sessions"
        );
    }
}
