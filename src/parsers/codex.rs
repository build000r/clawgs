//! Codex JSONL transcript parser.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use super::{
    extract_timestamp, is_harness_markup, push_action, truncate, visit_jsonl, visit_jsonl_str,
    JsonlStats, ParseSnapshot,
};
use crate::{Action, CommitSignal, ExtractOptions};

struct ToolCallObservation {
    action: Action,
    call_id: Option<String>,
    command: Option<String>,
    session_id: Option<String>,
    marks_edit: bool,
}

pub(crate) fn parse(path: &Path, options: &ExtractOptions) -> Result<ParseSnapshot> {
    let mut state = CodexParseState::new();
    let stats = visit_jsonl(path, options.include_raw, |entry| {
        state.ingest(entry, options);
    })?;

    Ok(state.into_snapshot(stats))
}

pub(crate) fn parse_str(input: &str, options: &ExtractOptions) -> Result<ParseSnapshot> {
    let mut state = CodexParseState::new();
    let stats = visit_jsonl_str(input, options.include_raw, |entry| {
        state.ingest(entry, options);
    })?;

    Ok(state.into_snapshot(stats))
}

struct CodexParseState {
    user_task: Option<String>,
    recent_actions: Vec<Action>,
    current_tool: Option<Action>,
    token_count: u64,
    awaiting_user_input: bool,
    awaiting_user_text: Option<String>,
    pending_validation_commands: HashMap<String, String>,
    pending_validation_sessions: HashMap<String, String>,
    pending_dirty_check_commands: HashSet<String>,
    pending_commit_commands: HashSet<String>,
    commit_signal: CommitSignal,
}

impl CodexParseState {
    fn new() -> Self {
        Self {
            user_task: None,
            recent_actions: Vec::new(),
            current_tool: None,
            token_count: 0,
            awaiting_user_input: false,
            awaiting_user_text: None,
            pending_validation_commands: HashMap::new(),
            pending_validation_sessions: HashMap::new(),
            pending_dirty_check_commands: HashSet::new(),
            pending_commit_commands: HashSet::new(),
            commit_signal: CommitSignal::default(),
        }
    }

    fn ingest(&mut self, entry: &Value, options: &ExtractOptions) {
        let ts = extract_timestamp(entry);
        update_user_task(entry, options, &mut self.user_task);
        update_token_count(entry, &mut self.token_count);
        update_awaiting_user_state(
            entry,
            options,
            &mut self.awaiting_user_input,
            &mut self.awaiting_user_text,
        );

        observe_and_record(
            function_call_observation(entry, options, &ts),
            &mut self.pending_validation_commands,
            &mut self.pending_validation_sessions,
            &mut self.pending_dirty_check_commands,
            &mut self.pending_commit_commands,
            &mut self.commit_signal,
            &mut self.recent_actions,
            &mut self.current_tool,
            options.max_actions,
        );
        observe_and_record(
            custom_tool_call_observation(entry, options, &ts),
            &mut self.pending_validation_commands,
            &mut self.pending_validation_sessions,
            &mut self.pending_dirty_check_commands,
            &mut self.pending_commit_commands,
            &mut self.commit_signal,
            &mut self.recent_actions,
            &mut self.current_tool,
            options.max_actions,
        );

        update_command_output_signals(
            entry,
            &mut self.pending_validation_commands,
            &mut self.pending_validation_sessions,
            &mut self.pending_dirty_check_commands,
            &mut self.pending_commit_commands,
            &mut self.commit_signal,
        );

        if let Some(action) = reasoning_event_action(entry, options, &ts) {
            record_action(
                &mut self.recent_actions,
                &mut self.current_tool,
                action,
                options.max_actions,
            );
        }

        record_actions(
            &mut self.recent_actions,
            &mut self.current_tool,
            reasoning_summary_actions(entry, options, &ts),
            options.max_actions,
        );
    }

    fn into_snapshot(mut self, stats: JsonlStats) -> ParseSnapshot {
        self.commit_signal.finalize();

        ParseSnapshot {
            user_task: self.user_task,
            recent_actions: self.recent_actions,
            current_tool: self.current_tool,
            token_count: self.token_count,
            awaiting_user_input: self.awaiting_user_input,
            awaiting_user_text: self.awaiting_user_text,
            commit_signal: Some(self.commit_signal),
            events_seen: stats.events_seen,
            malformed_lines_skipped: stats.malformed_lines_skipped,
            bytes_read: stats.bytes_read,
            raw_events: stats.raw_events,
        }
    }
}

fn entry_type(entry: &Value) -> &str {
    entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn payload(entry: &Value) -> &Value {
    entry.get("payload").unwrap_or(&Value::Null)
}

fn update_user_task(entry: &Value, options: &ExtractOptions, user_task: &mut Option<String>) {
    if let Some(text) = user_task_text(entry)
        .filter(|text| !internal_warning_text(text))
        .map(|text| truncate(&text, options.max_task_chars))
    {
        *user_task = Some(text);
    }
}

fn user_task_text(entry: &Value) -> Option<String> {
    match entry_type(entry) {
        "response_item" => user_response_item_text(payload(entry)),
        "event_msg" => user_event_message_text(payload(entry)),
        _ => None,
    }
}

fn is_user_turn_entry(entry: &Value) -> bool {
    match entry_type(entry) {
        "response_item" => payload(entry).get("role").and_then(Value::as_str) == Some("user"),
        "event_msg" => payload(entry).get("type").and_then(Value::as_str) == Some("user_message"),
        _ => false,
    }
}

fn user_response_item_text(payload: &Value) -> Option<String> {
    payload
        .get("role")
        .and_then(Value::as_str)
        .filter(|role| *role == "user")
        .and_then(|_| extract_user_input_text(payload))
        .filter(|text| !is_harness_markup(text))
}

fn user_event_message_text(payload: &Value) -> Option<String> {
    payload
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| *value == "user_message")
        .and_then(|_| payload.get("message").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        // Apply the same harness-markup filter as the `response_item` user path
        // (`user_response_item_text`). Without this, a harness wrapper delivered
        // as a `user_message` event (`<system-reminder>...`, slash-command or
        // hook output) would surface as the public `user_task`. The shared
        // length cap is applied downstream in `update_user_task`.
        .filter(|value| !is_harness_markup(value))
        .map(ToString::to_string)
}

fn internal_warning_text(text: &str) -> bool {
    const APPLY_PATCH_EXEC_WARNING: &str =
        "Warning: apply_patch was requested via exec_command. Use the apply_patch tool instead of exec_command.";
    text.trim_start().starts_with(APPLY_PATCH_EXEC_WARNING)
}

fn update_awaiting_user_state(
    entry: &Value,
    options: &ExtractOptions,
    awaiting_user_input: &mut bool,
    awaiting_user_text: &mut Option<String>,
) {
    // Use a structural check rather than `user_task_text(...).is_some()`. The
    // task-text path filters out user turns whose content is markup-prefixed
    // (e.g. `<system-reminder>...`) or whitespace-only — but those are still
    // valid user turns that should clear the awaiting flag. Without this, an
    // agent that finished a final-answer and then received a system-reminder-
    // prefixed user reply would stay flagged as
    // `awaiting_user_input=true` and the engine would keep the session
    // Sleeping while it is actually back at work.
    if is_user_turn_entry(entry) || event_payload(entry, "turn_aborted").is_some() {
        *awaiting_user_input = false;
        *awaiting_user_text = None;
        return;
    }

    if let Some((awaiting, text)) = assistant_turn_state(entry) {
        *awaiting_user_input = awaiting;
        // `awaiting_user_text` is a `short text` field per the clawgs.v2 schema.
        // Apply the same redaction the `user_task` path uses so a final answer
        // that opens with a harness wrapper (`<system-reminder>...`) is dropped
        // rather than surfaced, and bound the remaining text to the same budget
        // as `user_task` so an oversized final answer (which may contain pasted
        // secrets or other private content) cannot flow verbatim into the
        // public snapshot.
        *awaiting_user_text = text
            .filter(|text| !is_harness_markup(text))
            .map(|text| truncate(&text, options.max_task_chars));
        return;
    }

    if entry_signals_agent_progress(entry) {
        *awaiting_user_input = false;
        *awaiting_user_text = None;
    }
}

/// Tool calls, custom tool calls, function-call outputs, reasoning items, and
/// agent_reasoning events all imply the agent is still mid-turn — any one of
/// these clears the awaiting-user flag.
fn entry_signals_agent_progress(entry: &Value) -> bool {
    response_item_payload(entry, "function_call").is_some()
        || response_item_payload(entry, "custom_tool_call").is_some()
        || response_item_payload(entry, "function_call_output").is_some()
        || response_item_payload(entry, "reasoning").is_some()
        || event_payload(entry, "agent_reasoning").is_some()
}

fn assistant_turn_state(entry: &Value) -> Option<(bool, Option<String>)> {
    match entry_type(entry) {
        "response_item" => response_item_message_state(payload(entry)),
        "event_msg" => event_message_state(payload(entry)),
        _ => None,
    }
}

fn response_item_message_state(payload: &Value) -> Option<(bool, Option<String>)> {
    (payload.get("type").and_then(Value::as_str) == Some("message")
        && payload.get("role").and_then(Value::as_str) == Some("assistant"))
    .then(|| {
        let awaiting = payload.get("phase").and_then(Value::as_str) == Some("final_answer");
        (
            awaiting,
            awaiting.then(|| assistant_output_text(payload)).flatten(),
        )
    })
}

fn event_message_state(payload: &Value) -> Option<(bool, Option<String>)> {
    (payload.get("type").and_then(Value::as_str) == Some("agent_message")).then(|| {
        let awaiting = payload.get("phase").and_then(Value::as_str) == Some("final_answer");
        (
            awaiting,
            awaiting.then(|| assistant_output_text(payload)).flatten(),
        )
    })
}

fn assistant_output_text(payload: &Value) -> Option<String> {
    payload
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    matches!(
                        block.get("type").and_then(Value::as_str),
                        Some("output_text") | Some("text")
                    )
                    .then(|| block.get("text").and_then(Value::as_str))
                    .flatten()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn update_token_count(entry: &Value, token_count: &mut u64) {
    if let Some(value) = token_count_from_entry(entry) {
        *token_count = value;
    }
}

fn token_count_from_entry(entry: &Value) -> Option<u64> {
    response_token_count(entry).or_else(|| event_token_count(entry))
}

fn response_token_count(entry: &Value) -> Option<u64> {
    (entry_type(entry) == "response")
        .then_some(payload(entry))
        .and_then(|payload| payload.get("usage"))
        .and_then(|usage| usage.get("input_tokens"))
        .and_then(Value::as_u64)
}

fn event_token_count(entry: &Value) -> Option<u64> {
    event_payload(entry, "token_count")
        .and_then(|payload| payload.get("info"))
        .and_then(|info| {
            info.get("last_token_usage")
                .and_then(|usage| usage.get("input_tokens"))
                .and_then(Value::as_u64)
                .or_else(|| {
                    info.get("total_token_usage")
                        .and_then(|usage| usage.get("input_tokens"))
                        .and_then(Value::as_u64)
                })
        })
}

fn function_call_observation(
    entry: &Value,
    options: &ExtractOptions,
    ts: &Option<String>,
) -> Option<ToolCallObservation> {
    response_item_payload(entry, "function_call").map(|payload| {
        let parsed_arguments = payload.get("arguments").and_then(parse_tool_arguments);
        ToolCallObservation {
            action: Action {
                tool: payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                detail: parsed_arguments
                    .as_ref()
                    .and_then(|arguments| argument_detail(arguments, options.max_detail_chars)),
                kind: "function_call".to_string(),
                ts: ts.clone(),
            },
            call_id: payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            command: parsed_arguments.as_ref().and_then(command_from_arguments),
            session_id: parsed_arguments
                .as_ref()
                .and_then(session_id_from_arguments),
            marks_edit: false,
        }
    })
}

fn custom_tool_call_observation(
    entry: &Value,
    options: &ExtractOptions,
    ts: &Option<String>,
) -> Option<ToolCallObservation> {
    response_item_payload(entry, "custom_tool_call").map(|payload| {
        let tool_name = payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let detail = custom_tool_call_detail(
            tool_name.as_str(),
            payload.get("input"),
            options.max_detail_chars,
        );

        ToolCallObservation {
            action: Action {
                tool: tool_name.clone(),
                detail,
                kind: "function_call".to_string(),
                ts: ts.clone(),
            },
            call_id: payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            command: None,
            session_id: None,
            marks_edit: tool_name == "apply_patch"
                && payload.get("status").and_then(Value::as_str) == Some("completed"),
        }
    })
}

fn reasoning_event_action(
    entry: &Value,
    options: &ExtractOptions,
    ts: &Option<String>,
) -> Option<Action> {
    event_payload(entry, "agent_reasoning")
        .and_then(|payload| payload.get("text").and_then(Value::as_str))
        .map(|text| thinking_action(text, options.max_detail_chars, ts))
}

fn reasoning_summary_actions(
    entry: &Value,
    options: &ExtractOptions,
    ts: &Option<String>,
) -> Vec<Action> {
    response_item_payload(entry, "reasoning")
        .and_then(|payload| payload.get("summary").and_then(Value::as_array))
        .map(|items| {
            items
                .iter()
                .filter_map(|summary| summary_text_action(summary, options.max_detail_chars, ts))
                .collect()
        })
        .unwrap_or_default()
}

fn response_item_payload<'a>(entry: &'a Value, expected_type: &str) -> Option<&'a Value> {
    (entry_type(entry) == "response_item")
        .then_some(payload(entry))
        .filter(|payload| payload.get("type").and_then(Value::as_str) == Some(expected_type))
}

fn event_payload<'a>(entry: &'a Value, expected_type: &str) -> Option<&'a Value> {
    (entry_type(entry) == "event_msg")
        .then_some(payload(entry))
        .filter(|payload| payload.get("type").and_then(Value::as_str) == Some(expected_type))
}

fn summary_text_action(
    summary: &Value,
    max_detail_chars: usize,
    ts: &Option<String>,
) -> Option<Action> {
    summary
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| *value == "summary_text")
        .and_then(|_| summary.get("text").and_then(Value::as_str))
        .map(|text| thinking_action(text, max_detail_chars, ts))
}

fn thinking_action(text: &str, max_detail_chars: usize, ts: &Option<String>) -> Action {
    Action {
        tool: "thinking".to_string(),
        detail: Some(truncate(text, max_detail_chars)),
        kind: "thinking".to_string(),
        ts: ts.clone(),
    }
}

fn record_action(
    recent_actions: &mut Vec<Action>,
    current_tool: &mut Option<Action>,
    action: Action,
    max_actions: usize,
) {
    push_action(recent_actions, action.clone(), max_actions);
    *current_tool = Some(action);
}

fn record_actions(
    recent_actions: &mut Vec<Action>,
    current_tool: &mut Option<Action>,
    actions: Vec<Action>,
    max_actions: usize,
) {
    for action in actions {
        record_action(recent_actions, current_tool, action, max_actions);
    }
}

fn extract_user_input_text(payload: &Value) -> Option<String> {
    let blocks = payload.get("content")?.as_array()?;
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("input_text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn observe_and_record(
    observation: Option<ToolCallObservation>,
    pending_validation_commands: &mut HashMap<String, String>,
    pending_validation_sessions: &mut HashMap<String, String>,
    pending_dirty_check_commands: &mut HashSet<String>,
    pending_commit_commands: &mut HashSet<String>,
    commit_signal: &mut CommitSignal,
    recent_actions: &mut Vec<Action>,
    current_tool: &mut Option<Action>,
    max_actions: usize,
) {
    let Some(observation) = observation else {
        return;
    };
    observe_tool_call(
        &observation,
        pending_validation_commands,
        pending_validation_sessions,
        pending_dirty_check_commands,
        pending_commit_commands,
        commit_signal,
    );
    record_action(
        recent_actions,
        current_tool,
        observation.action,
        max_actions,
    );
}

fn observe_tool_call(
    observation: &ToolCallObservation,
    pending_validation_commands: &mut HashMap<String, String>,
    pending_validation_sessions: &mut HashMap<String, String>,
    pending_dirty_check_commands: &mut HashSet<String>,
    pending_commit_commands: &mut HashSet<String>,
    commit_signal: &mut CommitSignal,
) {
    if observation.marks_edit {
        commit_signal.edited = true;
        // A new edit invalidates prior validation, dirty-tree, commit, and
        // pending validation evidence: the new bytes need fresh observations.
        commit_signal.validated = false;
        commit_signal.dirty_checked = false;
        commit_signal.commit_seen = false;
        pending_validation_commands.clear();
        pending_validation_sessions.clear();
        pending_dirty_check_commands.clear();
        pending_commit_commands.clear();
    }

    match observation.action.tool.as_str() {
        "write_stdin" => carry_pending_validation_to_call(
            observation,
            pending_validation_commands,
            pending_validation_sessions,
        ),
        "exec_command" => observe_exec_command(
            observation,
            pending_validation_commands,
            pending_dirty_check_commands,
            pending_commit_commands,
        ),
        _ => {}
    }
}

fn carry_pending_validation_to_call(
    observation: &ToolCallObservation,
    pending_validation_commands: &mut HashMap<String, String>,
    pending_validation_sessions: &mut HashMap<String, String>,
) {
    let Some(call_id) = observation.call_id.as_deref() else {
        return;
    };
    let Some(session_id) = observation.session_id.as_deref() else {
        return;
    };
    let Some(command) = pending_validation_sessions.get(session_id) else {
        return;
    };
    pending_validation_commands.insert(call_id.to_string(), command.clone());
}

fn observe_exec_command(
    observation: &ToolCallObservation,
    pending_validation_commands: &mut HashMap<String, String>,
    pending_dirty_check_commands: &mut HashSet<String>,
    pending_commit_commands: &mut HashSet<String>,
) {
    let Some(command) = observation.command.as_deref() else {
        return;
    };

    let call_id = observation.call_id.as_deref();
    if dirty_check_command(command) {
        if let Some(call_id) = call_id {
            pending_dirty_check_commands.insert(call_id.to_string());
        }
    }
    if commit_command(command) {
        if let Some(call_id) = call_id {
            pending_commit_commands.insert(call_id.to_string());
        }
    }
    if validation_command(command) {
        if let Some(call_id) = call_id {
            pending_validation_commands.insert(call_id.to_string(), command.to_string());
        }
    }
}

fn update_command_output_signals(
    entry: &Value,
    pending_validation_commands: &mut HashMap<String, String>,
    pending_validation_sessions: &mut HashMap<String, String>,
    pending_dirty_check_commands: &mut HashSet<String>,
    pending_commit_commands: &mut HashSet<String>,
    commit_signal: &mut CommitSignal,
) {
    let Some(payload) = response_item_payload(entry, "function_call_output") else {
        return;
    };
    let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
        return;
    };
    let output = payload
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();

    if pending_dirty_check_commands.remove(call_id) && successful_command_output(output) {
        commit_signal.dirty_checked = true;
    }
    if pending_commit_commands.remove(call_id) && successful_command_output(output) {
        commit_signal.commit_seen = true;
    }

    let Some(command) = pending_validation_commands.remove(call_id) else {
        return;
    };
    if validation_command(&command) && successful_command_output(output) {
        commit_signal.validated = true;
    } else if let Some(session_id) = running_session_id_from_output(output) {
        pending_validation_sessions.insert(session_id, command);
    }
}

fn parse_tool_arguments(arguments: &Value) -> Option<Value> {
    match arguments {
        Value::String(value) => serde_json::from_str(value).ok(),
        Value::Object(_) => Some(arguments.clone()),
        _ => None,
    }
}

fn argument_detail(arguments: &Value, max_chars: usize) -> Option<String> {
    if let Some(command) = command_from_arguments(arguments) {
        return Some(truncate(&command, max_chars));
    }
    if let Some(file_path) = arguments.get("file_path").and_then(Value::as_str) {
        return Some(path_basename(file_path));
    }
    if let Some(pattern) = arguments.get("pattern").and_then(Value::as_str) {
        return Some(truncate(pattern, max_chars));
    }
    None
}

fn command_from_arguments(arguments: &Value) -> Option<String> {
    arguments
        .get("cmd")
        .or_else(|| arguments.get("command"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn session_id_from_arguments(arguments: &Value) -> Option<String> {
    arguments.get("session_id").and_then(|value| match value {
        Value::String(session_id) => Some(session_id.clone()),
        Value::Number(session_id) => Some(session_id.to_string()),
        _ => None,
    })
}

fn custom_tool_call_detail(
    tool_name: &str,
    input: Option<&Value>,
    max_chars: usize,
) -> Option<String> {
    let input = input?;
    if tool_name == "apply_patch" {
        return patch_target_detail(input);
    }

    input.as_str().map(|value| truncate(value, max_chars))
}

fn patch_target_detail(input: &Value) -> Option<String> {
    let patch = input.as_str()?;
    patch.lines().find_map(|line| {
        line.strip_prefix("*** Update File: ")
            .or_else(|| line.strip_prefix("*** Add File: "))
            .or_else(|| line.strip_prefix("*** Delete File: "))
            .or_else(|| line.strip_prefix("*** Move to: "))
            .map(path_basename)
    })
}

fn path_basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn dirty_check_command(command: &str) -> bool {
    let trimmed = command.trim();
    trimmed == "git status"
        || trimmed.starts_with("git status ")
        || trimmed == "git diff"
        || (trimmed.starts_with("git diff ")
            && !command_has_arg(trimmed, "--check")
            && !git_diff_uses_historical_revision(trimmed))
}

fn commit_command(command: &str) -> bool {
    let trimmed = command.trim();
    (trimmed == "git commit" || trimmed.starts_with("git commit "))
        && !command_has_arg(trimmed, "--dry-run")
}

fn command_has_arg(command: &str, arg: &str) -> bool {
    command.split_whitespace().any(|token| token == arg)
}

fn git_diff_uses_historical_revision(command: &str) -> bool {
    command
        .split_whitespace()
        .skip(2)
        .any(|token| token.contains("..") || token.contains('~') || token.contains('^'))
}

fn validation_command(command: &str) -> bool {
    let normalized = command.trim().to_lowercase();
    [
        "cargo test",
        "cargo build",
        "cargo nextest",
        "cargo clippy",
        "pytest",
        "vitest",
        "jest",
        "go test",
        "npm test",
        "npm run build",
        "npm run test",
        "npm run lint",
        "npm run type-check",
        "npm run typecheck",
        "pnpm build",
        "pnpm test",
        "pnpm lint",
        "pnpm type-check",
        "pnpm typecheck",
        "yarn build",
        "yarn test",
        "yarn lint",
        "yarn type-check",
        "yarn typecheck",
        "swift test",
        "swift build",
        "tsc --noemit",
        "eslint",
        "biome check",
        "ruff check",
        "mypy",
        "pyright",
    ]
    .iter()
    .any(|prefix| command_starts_with_word(&normalized, prefix))
        || make_validation_command(&normalized)
        || xcodebuild_validation_command(&normalized)
}

fn command_starts_with_word(command: &str, prefix: &str) -> bool {
    command == prefix
        || command
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

fn make_validation_command(normalized: &str) -> bool {
    normalized
        .strip_prefix("make ")
        .map(|target| {
            [
                "test",
                "check",
                "build",
                "lint",
                "verify",
                "typecheck",
                "type-check",
            ]
            .iter()
            .any(|target_name| make_target_matches(target, target_name))
        })
        .unwrap_or(false)
}

fn make_target_matches(target: &str, target_name: &str) -> bool {
    target == target_name
        || target
            .strip_prefix(target_name)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace) || rest.starts_with('-'))
}

fn xcodebuild_validation_command(normalized: &str) -> bool {
    command_starts_with_word(normalized, "xcodebuild")
        && normalized
            .split_whitespace()
            .skip(1)
            .any(|token| matches!(token, "test" | "build"))
}

fn successful_command_output(output: &str) -> bool {
    // Codex tool outputs are framed:
    //   <metadata lines incl. "Process exited with code N">
    //   Output:
    //   <stdout from the inner command>
    // Match the exit line only inside the metadata header so command stdout
    // that happens to print "Process exited with code 0" cannot forge a
    // validation pass when the command actually failed.
    let Some(header) = command_output_header(output) else {
        return false;
    };
    header
        .lines()
        .any(|line| line.trim() == "Process exited with code 0")
}

fn command_output_header(output: &str) -> Option<&str> {
    output.split_once("\nOutput:\n").map(|(header, _)| header)
}

fn running_session_id_from_output(output: &str) -> Option<String> {
    command_output_header(output)?.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Process running with session ID ")
            .map(ToString::to_string)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn parse_codex_extracts_task_function_call_and_tokens() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Build parser\"}}\n",
                "{\"type\":\"response\",\"payload\":{\"usage\":{\"input_tokens\":456}}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"command\\\":\\\"ls -la\\\"}\"}}\n"
            ),
        )
        .expect("write fixture");

        let options = ExtractOptions::default();
        let snapshot = parse(file.path(), &options).expect("parse");

        assert_eq!(snapshot.user_task.as_deref(), Some("Build parser"));
        assert_eq!(snapshot.token_count, 456);
        assert_eq!(snapshot.recent_actions.len(), 1);
        assert_eq!(snapshot.recent_actions[0].tool, "exec_command");
        assert_eq!(snapshot.recent_actions[0].kind, "function_call");
        assert_eq!(
            snapshot.commit_signal,
            Some(CommitSignal {
                candidate: false,
                edited: false,
                validated: false,
                dirty_checked: false,
                commit_seen: false,
            })
        );
        assert_eq!(
            snapshot.current_tool.as_ref().map(|a| a.tool.as_str()),
            Some("exec_command")
        );
        assert!(!snapshot.awaiting_user_input);
    }

    #[test]
    fn parse_codex_marks_final_answer_as_awaiting_user() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"checking\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Need your approval to continue.\"}]}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        assert!(snapshot.awaiting_user_input);
        assert_eq!(
            snapshot.awaiting_user_text.as_deref(),
            Some("Need your approval to continue.")
        );
    }

    #[test]
    fn parse_codex_clears_awaiting_user_when_work_resumes() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Which option do you want?\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"pwd\\\"}\"}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        assert!(!snapshot.awaiting_user_input);
        assert!(snapshot.awaiting_user_text.is_none());
    }

    #[test]
    fn parse_codex_clears_awaiting_user_on_markup_prefixed_user_reply() {
        // Regression: a final_answer flips awaiting=true. The user then replies
        // with a system-reminder-prefixed input (common harness shape — the
        // user_task_text filter rejects strings starting with `<`). The
        // structural user-turn check must still clear awaiting state because
        // the user has, in fact, taken a turn and the agent is no longer idle.
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Need approval to continue.\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"<system-reminder>continue working</system-reminder>\"}]}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        assert!(
            !snapshot.awaiting_user_input,
            "user reply must clear awaiting state even when content is markup-prefixed"
        );
        assert!(snapshot.awaiting_user_text.is_none());
    }

    #[test]
    fn extract_user_input_text_returns_first_nonempty_input_block() {
        let payload = serde_json::json!({
            "content": [
                {"type": "text", "text": "ignore"},
                {"type": "input_text", "text": "  inspect parser branches  "},
                {"type": "input_text", "text": "later"}
            ]
        });

        assert_eq!(
            extract_user_input_text(&payload).as_deref(),
            Some("inspect parser branches")
        );
    }

    #[test]
    fn extract_user_input_text_skips_empty_or_missing_blocks() {
        let empty_payload = serde_json::json!({
            "content": [
                {"type": "input_text", "text": "   "},
                {"type": "text", "text": "ignore"}
            ]
        });
        let missing_payload = serde_json::json!({"content": "not-an-array"});

        assert_eq!(extract_user_input_text(&empty_payload), None);
        assert_eq!(extract_user_input_text(&missing_payload), None);
    }

    #[test]
    fn user_response_item_text_rejects_short_markup_input() {
        let payload = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "input_text", "text": "<system>"}
            ]
        });

        assert_eq!(user_response_item_text(&payload), None);
    }

    #[test]
    fn user_response_item_text_preserves_user_html_or_xml_content() {
        for raw in [
            "Here's my HTML: <template>...</template>",
            "<html>my snippet</html>",
            "<div>why is this not centered?</div>",
            "<query> what does this do?",
        ] {
            let payload = serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "input_text", "text": raw}
                ]
            });
            assert_eq!(
                user_response_item_text(&payload).as_deref(),
                Some(raw),
                "filter should not drop legitimate user input: {raw:?}"
            );
        }
    }

    #[test]
    fn user_response_item_text_rejects_known_harness_wrappers() {
        for raw in [
            "<system-reminder>be quiet</system-reminder>",
            "<command-name>codebase-audit</command-name>",
            "<command-message>codebase-audit</command-message>",
            "<bash-input>ls -la</bash-input>",
            "<local-command-stdout>output</local-command-stdout>",
        ] {
            let payload = serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "input_text", "text": raw}
                ]
            });
            assert_eq!(
                user_response_item_text(&payload),
                None,
                "filter should drop harness markup: {raw:?}"
            );
        }
    }

    #[test]
    fn user_response_item_text_accepts_exactly_1000_chars() {
        let payload = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "input_text", "text": "a".repeat(1000)}
            ]
        });

        assert_eq!(user_response_item_text(&payload), Some("a".repeat(1000)));
    }

    #[test]
    fn user_event_message_text_rejects_harness_wrappers() {
        // Asymmetry fix: the `event_msg`/`user_message` task path must apply the
        // same `is_harness_markup` filter as the `response_item` path. Without
        // the filter these harness wrappers would surface as the public
        // `user_task`.
        for raw in [
            "<system-reminder>be quiet</system-reminder>",
            "<command-name>codebase-audit</command-name>",
            "<bash-input>ls -la</bash-input>",
            "  <local-command-stdout>output</local-command-stdout>",
        ] {
            let payload = serde_json::json!({"type": "user_message", "message": raw});
            assert_eq!(
                user_event_message_text(&payload),
                None,
                "user_message harness markup must be dropped: {raw:?}"
            );
        }
    }

    #[test]
    fn user_event_message_text_preserves_legitimate_content() {
        // The harness-markup filter must not drop genuine user prompts, including
        // ones that merely contain or begin with XML-ish syntax.
        for raw in [
            "Build a parser",
            "<div>why is this not centered?</div>",
            "Here is my HTML: <template>...</template>",
        ] {
            let payload = serde_json::json!({"type": "user_message", "message": raw});
            assert_eq!(
                user_event_message_text(&payload).as_deref(),
                Some(raw),
                "legitimate user_message content must be preserved: {raw:?}"
            );
        }
    }

    #[test]
    fn parse_codex_filters_harness_markup_from_event_message_user_task() {
        // End-to-end: a genuine task arrives via `user_message`, then a later
        // harness-injected `user_message` (system-reminder) must NOT overwrite
        // the real task — matching the `response_item` user path.
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Summarize the failing tests\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"<system-reminder>keep going</system-reminder>\"}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        assert_eq!(
            snapshot.user_task.as_deref(),
            Some("Summarize the failing tests"),
            "harness markup via user_message must not overwrite the real user task"
        );
    }

    #[test]
    fn parse_codex_drops_harness_markup_awaiting_user_text() {
        // Asymmetry fix: `awaiting_user_text` must run through `is_harness_markup`
        // like `user_task`. A final answer that opens with a harness wrapper must
        // not surface (even truncated) in the public snapshot.
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"<system-reminder>internal wrapper leaked into final answer</system-reminder>\"}]}}\n",
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        assert!(
            snapshot.awaiting_user_input,
            "structural final-answer signal still flips awaiting"
        );
        assert!(
            snapshot.awaiting_user_text.is_none(),
            "harness-markup final answer must not surface in awaiting_user_text"
        );
    }

    #[test]
    fn parse_codex_preserves_legitimate_awaiting_user_text() {
        // The harness filter must not drop a genuine final answer that merely
        // contains XML-ish content.
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Should I wrap it in a <div>?\"}]}}\n",
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        assert!(snapshot.awaiting_user_input);
        assert_eq!(
            snapshot.awaiting_user_text.as_deref(),
            Some("Should I wrap it in a <div>?"),
            "legitimate final answer must still surface"
        );
    }

    #[test]
    fn parse_codex_collects_reasoning_actions_and_file_details() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Review parser output\"}]}}\n",
                "{\"type\":\"response\",\"payload\":{\"usage\":{\"input_tokens\":77}}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"read_file\",\"arguments\":\"{\\\"file_path\\\":\\\"/tmp/demo.txt\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_reasoning\",\"text\":\"checking transcript details\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"looking at fallback handling\"},{\"type\":\"other\",\"text\":\"ignored\"}]}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        assert_eq!(snapshot.user_task.as_deref(), Some("Review parser output"));
        assert_eq!(snapshot.token_count, 77);
        assert_eq!(snapshot.recent_actions.len(), 3);
        assert_eq!(snapshot.recent_actions[0].tool, "read_file");
        assert_eq!(
            snapshot.recent_actions[0].detail.as_deref(),
            Some("demo.txt")
        );
        assert_eq!(snapshot.recent_actions[1].tool, "thinking");
        assert_eq!(
            snapshot.recent_actions[1].detail.as_deref(),
            Some("checking transcript details")
        );
        assert_eq!(snapshot.recent_actions[2].tool, "thinking");
        assert_eq!(
            snapshot.recent_actions[2].detail.as_deref(),
            Some("looking at fallback handling")
        );
        assert_eq!(
            snapshot
                .current_tool
                .as_ref()
                .map(|action| action.tool.as_str()),
            Some("thinking")
        );
    }

    #[test]
    fn parse_codex_ignores_markup_and_blank_user_inputs() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"<system>\"}]}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"   \"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Use the fallback task\"}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        assert_eq!(snapshot.user_task.as_deref(), Some("Use the fallback task"));
    }

    #[test]
    fn parse_codex_truncates_oversized_response_item_user_task() {
        let file = NamedTempFile::new().expect("temp file");
        let oversized = "a".repeat(1500);
        fs::write(
            file.path(),
            format!(
                "{{\"type\":\"response_item\",\"payload\":{{\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{oversized}\"}}]}}}}\n",
                oversized = oversized
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        let expected = "a".repeat(ExtractOptions::default().max_task_chars);
        assert_eq!(snapshot.user_task.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn parse_codex_preserves_user_warning_prompts_except_known_harness_warning() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Initial task\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Warning: do not run destructive commands.\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Warning: apply_patch was requested via exec_command. Use the apply_patch tool instead of exec_command.\"}]}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        assert_eq!(
            snapshot.user_task.as_deref(),
            Some("Warning: do not run destructive commands.")
        );
    }

    #[test]
    fn parse_codex_ignores_empty_reasoning_entries() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\",\"summary\":[{\"type\":\"other\",\"text\":\"ignored\"}]}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_reasoning\"}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        assert!(snapshot.recent_actions.is_empty());
        assert!(snapshot.current_tool.is_none());
    }

    #[test]
    fn parse_codex_current_shapes_preserve_task_and_commit_signal() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Ship preview-first widget fix\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test --manifest-path clawgs/Cargo.toml codex -- --nocapture\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: abc123\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nvalidation passed\\n\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":144379}}}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Warning: apply_patch was requested via exec_command. Use the apply_patch tool instead of exec_command.\"}]}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        assert_eq!(
            snapshot.user_task.as_deref(),
            Some("Ship preview-first widget fix")
        );
        assert_eq!(snapshot.token_count, 144379);
        assert_eq!(
            snapshot.recent_actions[0].detail.as_deref(),
            Some("git status --short")
        );
        assert_eq!(snapshot.recent_actions[1].tool, "apply_patch");
        assert_eq!(
            snapshot.recent_actions[1].detail.as_deref(),
            Some("widget.tsx")
        );
        assert_eq!(
            snapshot.commit_signal,
            Some(CommitSignal {
                candidate: false,
                edited: true,
                validated: true,
                dirty_checked: false,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_second_edit_after_validation_clears_commit_candidate() {
        // Realistic flow: dirty-check -> patch1 -> validate (passes) -> patch2
        // (no re-validation). Without resetting `validated` on the second
        // patch, candidate would still report true and falsely tell downstream
        // tools the agent is ready to commit even though the new bytes are
        // unvalidated.
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Iterate on widget\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch1\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: abc\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch2\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-new\\n+newer\\n*** End Patch\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        let signal = snapshot.commit_signal.expect("commit_signal");

        assert!(signal.edited, "second edit should still mark edited");
        assert!(
            !signal.dirty_checked,
            "dirty checks before the latest edit must not count"
        );
        assert!(
            !signal.validated,
            "second edit must invalidate the prior validation"
        );
        assert!(
            !signal.candidate,
            "candidate must be false until the new edit is re-validated"
        );
    }

    #[test]
    fn user_response_item_text_accepts_short_non_ascii_message_over_1000_bytes() {
        // 400 multi-byte CJK chars = ~1200 bytes UTF-8 but < 1000 chars.
        // The cap is meant to be character-based, so this should be accepted.
        let text: String = "漢字".repeat(200); // 400 chars, ~1200 bytes
        assert!(text.chars().count() < 1000);
        assert!(text.len() > 1000);

        let payload = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "input_text", "text": text}
            ]
        });

        assert!(
            user_response_item_text(&payload).is_some(),
            "non-ASCII task text under the 1000-char cap must not be silently dropped"
        );
    }

    #[test]
    fn dirty_check_command_recognizes_bare_git_diff_and_status_variants() {
        assert!(dirty_check_command("git status"));
        assert!(dirty_check_command("git status --short"));
        assert!(dirty_check_command("git diff"));
        assert!(dirty_check_command("git diff --cached"));
        assert!(dirty_check_command("git diff HEAD"));
        assert!(dirty_check_command("  git diff  "));
        assert!(!dirty_check_command("git diff --check"));
        assert!(!dirty_check_command("git diff --cached --check"));
        assert!(!dirty_check_command("git diff origin/main..HEAD"));
        assert!(!dirty_check_command("git diff main...feature"));
        assert!(!dirty_check_command("git diff HEAD~1"));
        assert!(!dirty_check_command("git diffstat"));
        assert!(!dirty_check_command("git log"));
    }

    #[test]
    fn commit_command_ignores_dry_run() {
        assert!(commit_command("git commit"));
        assert!(commit_command("git commit -m ready"));
        assert!(commit_command("git commit --amend --no-edit"));
        assert!(!commit_command("git commit --dry-run"));
        assert!(!commit_command("git commit --dry-run -m ready"));
        assert!(!commit_command("git status"));
    }

    #[test]
    fn successful_command_output_only_trusts_metadata_header() {
        // Real success: metadata header has the exit line.
        let real_success = "Chunk ID: abc\nWall time: 0.01s\nProcess exited with code 0\nOriginal token count: 12\nOutput:\n\nok\n";
        assert!(successful_command_output(real_success));

        // False positive guard: command stdout contains the literal exit line
        // but the header reports a non-zero exit. Must not be treated as success.
        let stdout_forgery = "Chunk ID: abc\nWall time: 0.01s\nProcess exited with code 1\nOriginal token count: 12\nOutput:\n\nlog: Process exited with code 0\n";
        assert!(!successful_command_output(stdout_forgery));

        // Header missing entirely (no Output: boundary): unchanged conservative
        // behavior — stdout that mentions the line is not enough on its own
        // outside the framed envelope.
        let unframed = "Process exited with code 0 (in some random log)";
        assert!(!successful_command_output(unframed));
        let exact_unframed = "Process exited with code 0";
        assert!(!successful_command_output(exact_unframed));
    }

    #[test]
    fn running_session_id_only_trusts_metadata_header() {
        let header_session = "Chunk ID: abc\nWall time: 0.01s\nProcess running with session ID 42\nOriginal token count: 12\nOutput:\n\n";
        assert_eq!(
            running_session_id_from_output(header_session).as_deref(),
            Some("42")
        );

        let stdout_forgery = "Chunk ID: abc\nWall time: 0.01s\nProcess exited with code 1\nOriginal token count: 12\nOutput:\n\nProcess running with session ID 42\n";
        assert_eq!(running_session_id_from_output(stdout_forgery), None);
    }

    #[test]
    fn validation_command_recognizes_make_swift_and_xcodebuild_flows() {
        assert!(validation_command("make test"));
        assert!(validation_command("make build-agent"));
        assert!(validation_command("swift test"));
        assert!(validation_command("swift build"));
        assert!(validation_command("xcodebuild -scheme Etcha test"));
        assert!(validation_command("xcodebuild -scheme Etcha build"));
        assert!(!validation_command("make docs"));
        assert!(!validation_command("make testdata"));
        assert!(!validation_command("xcodebuildish -scheme Etcha test"));
        assert!(!validation_command("xcodebuild-wrapper -scheme Etcha test"));
        assert!(!validation_command(
            "xcodebuild -list -project TestApp.xcodeproj"
        ));
        assert!(!validation_command("xcodebuild -showBuildSettings"));
        assert!(!validation_command("xcodebuild -list"));
        assert!(!validation_command("cargo testish"));
        assert!(!validation_command("pytestfoo"));
    }

    #[test]
    fn parse_codex_failed_validation_stdout_cannot_seed_running_session() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Finish contract\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/lib.rs\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: test\\nWall time: 0.01s\\nProcess exited with code 1\\nOriginal token count: 12\\nOutput:\\n\\nProcess running with session ID 23259\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"write_stdin\",\"arguments\":\"{\\\"session_id\\\":23259,\\\"chars\\\":\\\"\\\",\\\"yield_time_ms\\\":1000}\",\"call_id\":\"call_poll\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_poll\",\"output\":\"Chunk ID: poll\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_status\",\"output\":\"Chunk ID: status\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M src/lib.rs\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        assert_eq!(
            parse(file.path(), &ExtractOptions::default())
                .expect("parse")
                .commit_signal,
            Some(CommitSignal {
                candidate: false,
                edited: true,
                validated: false,
                dirty_checked: true,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_make_test_marks_commit_candidate_after_success() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Finish the tape save flow\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/Sources/App.swift\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"make test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: abc123\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nvalidation passed\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_fresh_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_fresh_status\",\"output\":\"Chunk ID: status\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M src/lib.rs\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        assert_eq!(
            snapshot.commit_signal,
            Some(CommitSignal {
                candidate: true,
                edited: true,
                validated: true,
                dirty_checked: true,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_failed_dirty_check_does_not_mark_commit_ready() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Finish contract\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/lib.rs\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: test\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_status\",\"output\":\"Chunk ID: status\\nWall time: 0.01s\\nProcess exited with code 128\\nOriginal token count: 12\\nOutput:\\n\\nfatal: not a git repository\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        assert_eq!(
            parse(file.path(), &ExtractOptions::default())
                .expect("parse")
                .commit_signal,
            Some(CommitSignal {
                candidate: false,
                edited: true,
                validated: true,
                dirty_checked: false,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_diff_check_does_not_prove_dirty_tree_checked() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Finish contract\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/lib.rs\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: test\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git diff --check\\\"}\",\"call_id\":\"call_diff_check\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_diff_check\",\"output\":\"Chunk ID: diff\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        assert_eq!(
            parse(file.path(), &ExtractOptions::default())
                .expect("parse")
                .commit_signal,
            Some(CommitSignal {
                candidate: false,
                edited: true,
                validated: true,
                dirty_checked: false,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_failed_commit_does_not_suppress_commit_candidate() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Commit if ready\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/lib.rs\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: test\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_status\",\"output\":\"Chunk ID: status\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M src/lib.rs\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git commit -m ready\\\"}\",\"call_id\":\"call_commit\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_commit\",\"output\":\"Chunk ID: commit\\nWall time: 0.01s\\nProcess exited with code 1\\nOriginal token count: 12\\nOutput:\\n\\nmissing user.name\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        assert_eq!(
            parse(file.path(), &ExtractOptions::default())
                .expect("parse")
                .commit_signal,
            Some(CommitSignal {
                candidate: true,
                edited: true,
                validated: true,
                dirty_checked: true,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_commit_dry_run_does_not_suppress_commit_candidate() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Commit if ready\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/lib.rs\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: test\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_status\",\"output\":\"Chunk ID: status\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M src/lib.rs\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git commit --dry-run -m ready\\\"}\",\"call_id\":\"call_commit\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_commit\",\"output\":\"Chunk ID: commit\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nOn branch main\\nChanges to be committed\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        assert_eq!(
            parse(file.path(), &ExtractOptions::default())
                .expect("parse")
                .commit_signal,
            Some(CommitSignal {
                candidate: true,
                edited: true,
                validated: true,
                dirty_checked: true,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_write_stdin_validation_marks_commit_candidate() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Finish the tape save flow\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/Sources/App.swift\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"make test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: abc123\\nWall time: 0.0100 seconds\\nProcess running with session ID 23259\\nOriginal token count: 12\\nOutput:\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"write_stdin\",\"arguments\":\"{\\\"session_id\\\":23259,\\\"chars\\\":\\\"\\\",\\\"yield_time_ms\\\":1000}\",\"call_id\":\"call_poll\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_poll\",\"output\":\"Chunk ID: def456\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nvalidation passed\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git diff --stat\\\"}\",\"call_id\":\"call_fresh_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_fresh_status\",\"output\":\"Chunk ID: status\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n src/lib.rs | 2 +-\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");

        assert_eq!(
            snapshot.commit_signal,
            Some(CommitSignal {
                candidate: true,
                edited: true,
                validated: true,
                dirty_checked: true,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_commit_before_later_edit_does_not_suppress_new_candidate() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Continue after commit\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch1\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate1\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate1\",\"output\":\"Chunk ID: abc\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status1\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_status1\",\"output\":\"Chunk ID: status1\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M src/widget.tsx\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git commit -m first\\\"}\",\"call_id\":\"call_commit\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_commit\",\"output\":\"Chunk ID: commit\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n[main abc123] first\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch2\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/src/widget.tsx\\n@@\\n-new\\n+newer\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cargo test\\\"}\",\"call_id\":\"call_validate2\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate2\",\"output\":\"Chunk ID: def\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nok\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_status2\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_status2\",\"output\":\"Chunk ID: status2\\nWall time: 0.01s\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M src/widget.tsx\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        assert_eq!(
            parse(file.path(), &ExtractOptions::default())
                .expect("parse")
                .commit_signal,
            Some(CommitSignal {
                candidate: true,
                edited: true,
                validated: true,
                dirty_checked: true,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn parse_codex_later_edit_ignores_stale_running_validation_completion() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Keep validation fresh\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch1\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/Sources/App.swift\\n@@\\n-old\\n+new\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"make test\\\"}\",\"call_id\":\"call_validate\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_validate\",\"output\":\"Chunk ID: abc123\\nWall time: 0.0100 seconds\\nProcess running with session ID 23259\\nOriginal token count: 12\\nOutput:\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"custom_tool_call\",\"status\":\"completed\",\"name\":\"apply_patch\",\"call_id\":\"call_patch2\",\"input\":\"*** Begin Patch\\n*** Update File: /tmp/project/Sources/App.swift\\n@@\\n-new\\n+newer\\n*** End Patch\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"write_stdin\",\"arguments\":\"{\\\"session_id\\\":23259,\\\"chars\\\":\\\"\\\",\\\"yield_time_ms\\\":1000}\",\"call_id\":\"call_poll\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_poll\",\"output\":\"Chunk ID: def456\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\nvalidation passed\\n\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",\"call_id\":\"call_fresh_status\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_fresh_status\",\"output\":\"Chunk ID: status\\nWall time: 0.0100 seconds\\nProcess exited with code 0\\nOriginal token count: 12\\nOutput:\\n\\n M Sources/App.swift\\n\"}}\n"
            ),
        )
        .expect("write fixture");

        assert_eq!(
            parse(file.path(), &ExtractOptions::default())
                .expect("parse")
                .commit_signal,
            Some(CommitSignal {
                candidate: false,
                edited: true,
                validated: false,
                dirty_checked: true,
                commit_seen: false,
            })
        );
    }

    #[test]
    fn event_message_state_marks_final_answer_agent_message_as_awaiting() {
        // `event_msg` final answers arrive as `agent_message` payloads (distinct
        // from the `response_item` `message` shape). This branch of
        // `event_message_state` was previously the least-covered awaiting path.
        let payload = serde_json::json!({
            "type": "agent_message",
            "phase": "final_answer",
            "content": [{"type": "output_text", "text": "  Should I proceed?  "}]
        });

        assert_eq!(
            event_message_state(&payload),
            Some((true, Some("Should I proceed?".to_string())))
        );
    }

    #[test]
    fn event_message_state_non_final_phase_is_not_awaiting() {
        // An `agent_message` that is not a final answer is mid-turn: awaiting
        // must stay false and no text is surfaced.
        let payload = serde_json::json!({
            "type": "agent_message",
            "phase": "commentary",
            "content": [{"type": "output_text", "text": "still working"}]
        });

        assert_eq!(event_message_state(&payload), Some((false, None)));
    }

    #[test]
    fn event_message_state_final_answer_with_empty_content_yields_no_text() {
        // Final answer flag set, but the content is whitespace-only: awaiting is
        // still true (the structural signal holds) while the text is dropped.
        let payload = serde_json::json!({
            "type": "agent_message",
            "phase": "final_answer",
            "content": [{"type": "output_text", "text": "   "}]
        });

        assert_eq!(event_message_state(&payload), Some((true, None)));
    }

    #[test]
    fn event_message_state_ignores_non_agent_message_events() {
        let payload = serde_json::json!({
            "type": "token_count",
            "phase": "final_answer"
        });

        assert_eq!(event_message_state(&payload), None);
    }

    #[test]
    fn parse_codex_marks_agent_message_event_final_answer_as_awaiting() {
        // End-to-end: an `event_msg` `agent_message` final answer drives the
        // snapshot awaiting fields, exercising the `event_msg` arm of
        // `assistant_turn_state` that the response_item tests do not.
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Need a decision from you.\"}]}}\n",
        )
        .expect("write fixture");

        let snapshot = parse(file.path(), &ExtractOptions::default()).expect("parse");
        assert!(snapshot.awaiting_user_input);
        assert_eq!(
            snapshot.awaiting_user_text.as_deref(),
            Some("Need a decision from you.")
        );
    }

    #[test]
    fn parse_codex_bounds_awaiting_user_text_to_max_task_chars() {
        // Regression: `awaiting_user_text` is a `short text` field in the
        // clawgs.v2 schema. An oversized assistant final answer (which can carry
        // pasted secrets or other private content) must NOT flow verbatim into
        // the public snapshot; it is capped to `max_task_chars` exactly like
        // `user_task`.
        let big = "S".repeat(5000);
        let line = format!(
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{big}\"}}]}}}}\n"
        );
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), line).expect("write fixture");

        let options = ExtractOptions::default();
        let snapshot = parse(file.path(), &options).expect("parse");

        assert!(snapshot.awaiting_user_input);
        let text = snapshot
            .awaiting_user_text
            .expect("final answer should still surface (bounded) text");
        assert_eq!(
            text.chars().count(),
            options.max_task_chars,
            "awaiting_user_text must be capped at max_task_chars, not surfaced verbatim"
        );
        assert!(
            text.chars().count() < big.chars().count(),
            "oversized final answer must be truncated"
        );
        assert!(
            text.chars().all(|c| c == 'S'),
            "truncation should preserve a prefix of the original answer"
        );
    }

    #[test]
    fn parse_codex_respects_custom_max_task_chars_for_awaiting_text() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"abcdefghij\"}]}}\n",
        )
        .expect("write fixture");

        let options = ExtractOptions {
            max_task_chars: 4,
            ..ExtractOptions::default()
        };
        let snapshot = parse(file.path(), &options).expect("parse");
        assert_eq!(snapshot.awaiting_user_text.as_deref(), Some("abcd"));
    }
}
