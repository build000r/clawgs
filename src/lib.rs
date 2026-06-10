//! Normalize Claude Code and Codex JSONL transcripts into stable
//! [`clawgs.v2`](https://github.com/build000r/clawgs/blob/main/references/schema-v2.md)
//! JSON snapshots.
//!
//! # Library usage
//!
//! Parse a JSONL string into a structured [`ExtractOutput`]:
//!
//! ```
//! use std::path::Path;
//! use clawgs::{AgentTool, ExtractOptions, extract_jsonl_str};
//!
//! let jsonl = r#"{"type":"session_meta","payload":{"cwd":"/tmp/project"}}
//! {"type":"event_msg","payload":{"type":"user_message","message":"Build a parser"}}
//! {"type":"response","payload":{"usage":{"input_tokens":500}}}
//! "#;
//!
//! let output = extract_jsonl_str(
//!     AgentTool::Codex,
//!     "inline",
//!     jsonl,
//!     Path::new("/tmp/project"),
//!     false,
//!     &ExtractOptions::default(),
//! ).unwrap();
//!
//! assert_eq!(output.schema_version, "clawgs.v2");
//! assert_eq!(output.snapshot.user_task.as_deref(), Some("Build a parser"));
//! assert!(output.stats.events_seen > 0);
//! ```
//!
//! # CLI
//!
//! The `clawgs` binary exposes the same functionality via subcommands. See the
//! [README](https://github.com/build000r/clawgs) for CLI usage.

#![warn(missing_docs)]

pub mod emit;
pub mod parsers;
pub mod tmux;

use std::collections::HashSet;
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use parsers::ParseSnapshot;

const SCHEMA_VERSION: &str = "clawgs.v2";

/// Supported agent transcript sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTool {
    /// Claude Code JSONL transcripts.
    Claude,
    /// OpenAI Codex JSONL transcripts.
    Codex,
}

impl AgentTool {
    /// Returns the lowercase tool name (`"claude"` or `"codex"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// CLI-level tool selection before resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSelection {
    /// Infer from the newest available transcript.
    Auto,
    /// Force Claude Code parser.
    Claude,
    /// Force Codex parser.
    Codex,
}

/// Output-shaping limits for [`extract`] and [`extract_jsonl_str`].
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    /// Maximum recent actions retained in the snapshot.
    pub max_actions: usize,
    /// Character budget for the user task field.
    pub max_task_chars: usize,
    /// Character budget for per-action detail strings.
    pub max_detail_chars: usize,
    /// When `true`, include the last 20 raw transcript events.
    pub include_raw: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            max_actions: 10,
            max_task_chars: 300,
            max_detail_chars: 100,
            include_raw: false,
        }
    }
}

/// A single agent action observed in the transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Action {
    /// The tool or function name invoked (e.g. `"exec_command"`).
    pub tool: String,
    /// Optional human-readable detail (command text, file path, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Action category (`"tool_call"`, `"edit"`, etc.).
    pub kind: String,
    /// ISO 8601 timestamp when the action was observed, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
}

/// Commit-readiness signals derived from transcript events.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitSignal {
    /// `true` when all four observations indicate a commit is ready.
    pub candidate: bool,
    /// An edit (file write, patch) was observed.
    pub edited: bool,
    /// A validation step (test, lint, type-check) succeeded after the edit.
    pub validated: bool,
    /// A dirty-tree check ran after the latest edit.
    pub dirty_checked: bool,
    /// A commit was observed after the latest edit (clears candidate).
    pub commit_seen: bool,
}

impl CommitSignal {
    /// `candidate` is derived from the four observations rather than set by
    /// callers — keep the predicate in one place so the parser doesn't drift
    /// from the schema.
    pub fn finalize(&mut self) {
        self.candidate = self.edited && self.validated && self.dirty_checked && !self.commit_seen;
    }
}

/// A transcript-derived attention fact for downstream consumers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionCue {
    /// What the cue signals (e.g. commit readiness, awaiting user).
    pub kind: ActionCueKind,
    /// Current cue state (currently always `Active`).
    pub status: ActionCueStatus,
    /// Where the cue was derived from (currently always `Transcript`).
    pub source: ActionCueSource,
    /// Confidence level of the derivation.
    pub confidence: ActionCueConfidence,
    /// Evidence tags supporting this cue.
    pub evidence: Vec<String>,
}

impl ActionCue {
    pub(crate) fn active(kind: ActionCueKind) -> Self {
        Self {
            kind,
            status: ActionCueStatus::Active,
            source: ActionCueSource::Transcript,
            confidence: ActionCueConfidence::Deterministic,
            evidence: Self::expected_evidence(kind)
                .iter()
                .map(|item| item.to_string())
                .collect(),
        }
    }

    /// Returns the canonical evidence tags for the given cue kind.
    pub fn expected_evidence(kind: ActionCueKind) -> &'static [&'static str] {
        match kind {
            ActionCueKind::AwaitingUser => &["awaiting_user_input"],
            ActionCueKind::CommitReady => &[
                "edit_seen",
                "validation_succeeded",
                "dirty_tree_checked_after_latest_edit",
                "commit_not_seen_after_latest_edit",
            ],
            ActionCueKind::ValidationMissingAfterEdit => &[
                "edit_seen",
                "fresh_validation_not_seen",
                "commit_not_seen_after_latest_edit",
            ],
            ActionCueKind::DirtyCheckMissing => &[
                "edit_seen",
                "validation_succeeded",
                "dirty_tree_check_not_seen_after_latest_edit",
                "commit_not_seen_after_latest_edit",
            ],
        }
    }

    /// `true` when the evidence tags exactly match the expected set for this cue's kind.
    pub fn has_expected_evidence(&self) -> bool {
        let expected = Self::expected_evidence(self.kind);
        self.evidence.len() == expected.len()
            && self
                .evidence
                .iter()
                .zip(expected.iter())
                .all(|(actual, expected)| actual == expected)
    }

    /// `true` when this cue carries complete, matching evidence.
    pub fn is_valid(&self) -> bool {
        self.has_expected_evidence()
    }

    pub(crate) fn valid_from(action_cues: &[Self]) -> Vec<Self> {
        action_cues
            .iter()
            .filter(|cue| cue.is_valid())
            .cloned()
            .collect()
    }

    pub(crate) fn contains_valid_kind(action_cues: &[Self], kind: ActionCueKind) -> bool {
        action_cues
            .iter()
            .any(|cue| cue.kind == kind && cue.is_valid())
    }
}

/// The kind of attention fact an [`ActionCue`] represents.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueKind {
    /// The agent is waiting for user input.
    AwaitingUser,
    /// An edit was validated and the tree is dirty — ready to commit.
    CommitReady,
    /// An edit occurred but no validation step has been observed yet.
    ValidationMissingAfterEdit,
    /// An edit was validated but no dirty-tree check has been observed.
    DirtyCheckMissing,
}

impl ActionCueKind {
    /// Returns the snake_case wire name for this cue kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingUser => "awaiting_user",
            Self::CommitReady => "commit_ready",
            Self::ValidationMissingAfterEdit => "validation_missing_after_edit",
            Self::DirtyCheckMissing => "dirty_check_missing",
        }
    }
}

/// Whether an [`ActionCue`] is currently active.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueStatus {
    /// The cue is currently active.
    Active,
}

/// Where an [`ActionCue`] was derived from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueSource {
    /// Derived from transcript event analysis.
    Transcript,
}

/// Confidence level of an [`ActionCue`] derivation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueConfidence {
    /// All evidence is deterministic (no heuristic or probabilistic component).
    Deterministic,
}

/// Normalized session state extracted from a transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The user's original task or prompt, truncated to [`ExtractOptions::max_task_chars`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_task: Option<String>,
    /// The most recent tool invocation still in progress, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<Action>,
    /// Cumulative input token count observed in the transcript.
    pub token_count: u64,
    /// `true` when the agent is waiting for user input.
    #[serde(default, skip_serializing_if = "is_false")]
    pub awaiting_user_input: bool,
    /// The text the agent displayed when requesting user input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awaiting_user_text: Option<String>,
    /// The most recent actions, newest last, capped at [`ExtractOptions::max_actions`].
    #[serde(default)]
    pub recent_actions: Vec<Action>,
    /// Commit-readiness signals, present when any edit was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_signal: Option<CommitSignal>,
    /// Transcript-derived attention facts for downstream consumers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_cues: Vec<ActionCue>,
}

/// Metadata about the transcript file that was parsed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Which parser was used (`"claude"` or `"codex"`).
    pub tool: String,
    /// File path or `"embedded:..."` for demo transcripts.
    pub path: String,
    /// `true` when the transcript was found by discovery rather than `--input`.
    pub discovered: bool,
    /// Working directory the transcript was associated with.
    pub cwd: String,
}

/// Parse statistics from transcript extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    /// Total JSONL events processed.
    pub events_seen: u64,
    /// Lines skipped due to parse errors.
    pub malformed_lines_skipped: u64,
    /// Total bytes read from the transcript file.
    pub bytes_read: u64,
}

/// The full `clawgs.v2` extraction result: source metadata, normalized snapshot, and parse stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractOutput {
    /// Always `"clawgs.v2"`.
    pub schema_version: String,
    /// Metadata about the parsed transcript file.
    pub source: Source,
    /// The normalized session state.
    pub snapshot: Snapshot,
    /// Parse statistics.
    pub stats: Stats,
    /// ISO 8601 timestamp when extraction ran.
    pub generated_at: String,
    /// Raw transcript events, present only when `include_raw` was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_events: Option<Vec<Value>>,
}

/// A transcript file resolved by discovery or explicit `--input` path.
#[derive(Debug, Clone)]
pub struct ResolvedInput {
    /// Which parser applies to this transcript.
    pub tool: AgentTool,
    /// Filesystem path to the JSONL transcript.
    pub path: PathBuf,
    /// `true` when found by discovery, `false` when passed via `--input`.
    pub discovered: bool,
}

/// Resolve a transcript file from an explicit path or by discovery under `cwd`.
pub fn resolve_input(
    selection: ToolSelection,
    cwd: &Path,
    input: Option<&Path>,
) -> Result<ResolvedInput> {
    if let Some(path) = input {
        let tool = match selection {
            ToolSelection::Auto => infer_tool_from_file(path)?,
            ToolSelection::Claude => AgentTool::Claude,
            ToolSelection::Codex => AgentTool::Codex,
        };

        return Ok(ResolvedInput {
            tool,
            path: path.to_path_buf(),
            discovered: false,
        });
    }

    let resolved = match selection {
        ToolSelection::Auto => discover_auto(cwd),
        ToolSelection::Claude => discover_for_tool(cwd, AgentTool::Claude),
        ToolSelection::Codex => discover_for_tool(cwd, AgentTool::Codex),
    }?;

    Ok(resolved)
}

/// Parse a JSONL transcript file into a `clawgs.v2` [`ExtractOutput`].
pub fn extract(
    tool: AgentTool,
    path: &Path,
    cwd: &Path,
    discovered: bool,
    options: &ExtractOptions,
) -> Result<ExtractOutput> {
    let parsed: ParseSnapshot = match tool {
        AgentTool::Claude => parsers::claude::parse(path, options)?,
        AgentTool::Codex => parsers::codex::parse(path, options)?,
    };
    Ok(extract_output(
        tool,
        path.display().to_string(),
        cwd,
        discovered,
        parsed,
    ))
}

/// Parse a JSONL string in memory into a `clawgs.v2` [`ExtractOutput`].
pub fn extract_jsonl_str(
    tool: AgentTool,
    source_path: &str,
    input: &str,
    cwd: &Path,
    discovered: bool,
    options: &ExtractOptions,
) -> Result<ExtractOutput> {
    let parsed: ParseSnapshot = match tool {
        AgentTool::Claude => parsers::claude::parse_str(input, options)?,
        AgentTool::Codex => parsers::codex::parse_str(input, options)?,
    };

    Ok(extract_output(
        tool,
        source_path.to_string(),
        cwd,
        discovered,
        parsed,
    ))
}

fn extract_output(
    tool: AgentTool,
    source_path: String,
    cwd: &Path,
    discovered: bool,
    parsed: ParseSnapshot,
) -> ExtractOutput {
    let action_cues =
        action_cues_for_snapshot(parsed.commit_signal.as_ref(), parsed.awaiting_user_input);

    ExtractOutput {
        schema_version: SCHEMA_VERSION.to_string(),
        source: Source {
            tool: tool.as_str().to_string(),
            path: source_path,
            discovered,
            cwd: cwd.display().to_string(),
        },
        snapshot: Snapshot {
            user_task: parsed.user_task,
            current_tool: parsed.current_tool,
            token_count: parsed.token_count,
            awaiting_user_input: parsed.awaiting_user_input,
            awaiting_user_text: parsed.awaiting_user_text,
            recent_actions: parsed.recent_actions,
            commit_signal: parsed.commit_signal,
            action_cues,
        },
        stats: Stats {
            events_seen: parsed.events_seen,
            malformed_lines_skipped: parsed.malformed_lines_skipped,
            bytes_read: parsed.bytes_read,
        },
        generated_at: Utc::now().to_rfc3339(),
        raw_events: parsed.raw_events,
    }
}

pub(crate) fn action_cues_for_snapshot(
    commit_signal: Option<&CommitSignal>,
    awaiting_user_input: bool,
) -> Vec<ActionCue> {
    let mut cues = Vec::new();

    if awaiting_user_input {
        cues.push(ActionCue::active(ActionCueKind::AwaitingUser));
    }

    if let Some(kind) = commit_signal.and_then(commit_signal_action_cue_kind) {
        cues.push(ActionCue::active(kind));
    }

    cues
}

fn commit_signal_action_cue_kind(signal: &CommitSignal) -> Option<ActionCueKind> {
    if signal.candidate {
        return Some(ActionCueKind::CommitReady);
    }

    if !signal.edited || signal.commit_seen {
        return None;
    }

    if !signal.validated {
        return Some(ActionCueKind::ValidationMissingAfterEdit);
    }

    (!signal.dirty_checked).then_some(ActionCueKind::DirtyCheckMissing)
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Read the first 40 lines of a JSONL file and infer whether it is a Claude or Codex transcript.
pub fn infer_tool_from_file(path: &Path) -> Result<AgentTool> {
    let file = fs::File::open(path).with_context(|| {
        format!(
            "failed to open input file for tool inference: {}",
            path.display()
        )
    })?;
    let reader = std::io::BufReader::new(file);

    for line in reader.lines().take(40) {
        // Skip unreadable (e.g. non-UTF-8) lines rather than aborting the whole
        // inference: a single bad line should not hide a valid tool marker on a
        // later line, matching the tolerance of the discovery scan
        // (`reader_matches_or_lacks_cwd`) and `parsed_line_value`.
        let Ok(line) = line else {
            continue;
        };
        if let Some(tool) = parsed_line_value(&line).and_then(|value| infer_tool_from_entry(&value))
        {
            return Ok(tool);
        }
    }

    Err(anyhow!(
        "could not infer tool format from {}. Pass --tool claude or --tool codex",
        path.display()
    ))
}

/// Discover the newest transcript for `tool` under the default log directory for `cwd`.
pub fn discover_for_tool(cwd: &Path, tool: AgentTool) -> Result<ResolvedInput> {
    discovered_path_for_tool(cwd, tool)
        .map(|path| discovered_input(tool, path))
        .ok_or_else(|| {
            anyhow!(
                "no {tool_name} transcript JSONL found for cwd {cwd}.\n  \
                 Try: clawgs extract --tool {tool_name} --input <path/to/session.jsonl>",
                tool_name = tool.as_str(),
                cwd = cwd.display()
            )
        })
}

/// Auto-discover the newest Claude or Codex transcript matching `cwd`.
pub fn discover_auto(cwd: &Path) -> Result<ResolvedInput> {
    match (discover_claude_path(cwd), discover_codex_path(cwd)) {
        (Some(path), None) => Ok(discovered_input(AgentTool::Claude, path)),
        (None, Some(path)) => Ok(discovered_input(AgentTool::Codex, path)),
        (Some(claude), Some(codex)) => Ok(newer_discovered_input(claude, codex)),
        (None, None) => Err(anyhow!(
            "no Claude or Codex transcript JSONL found for cwd {}.\n  \
             Try: clawgs extract --input <path/to/session.jsonl>, or run \
             `clawgs demo extract --tool codex` to see the snapshot format.",
            cwd.display()
        )),
    }
}

/// Return the newest Claude transcript path for `cwd`, if any.
pub fn discover_claude_path(cwd: &Path) -> Option<PathBuf> {
    discover_claude_paths(cwd).into_iter().next()
}

/// Return all Claude transcript paths for `cwd`, newest first.
pub fn discover_claude_paths(cwd: &Path) -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let cwd_slug = cwd.display().to_string().replace('/', "-");
    let project_dir = home.join(".claude").join("projects").join(cwd_slug);

    let mut files: Vec<(PathBuf, SystemTime)> = match fs::read_dir(project_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .filter(|path| claude_file_matches_cwd(path, cwd))
            .filter_map(|path| {
                let modified = fs::metadata(&path).ok()?.modified().ok()?;
                Some((path, modified))
            })
            .collect(),
        Err(_) => return Vec::new(),
    };

    // Newest mtime first; break mtime ties by path (descending) so same-second
    // sessions resolve deterministically instead of in arbitrary readdir order,
    // matching the path-based tiebreak Codex discovery already uses.
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    files.into_iter().map(|(path, _)| path).collect()
}

/// Return the newest Codex transcript path for `cwd`, if any.
pub fn discover_codex_path(cwd: &Path) -> Option<PathBuf> {
    discover_codex_paths(cwd).into_iter().next()
}

/// Return all Codex transcript paths for `cwd`, newest first.
pub fn discover_codex_paths(cwd: &Path) -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let sessions_dir = home.join(".codex").join("sessions");
    sorted_numeric_subdirs_reverse(&sessions_dir, 4)
        .into_iter()
        .flat_map(|year| codex_paths_in_year(&year, cwd))
        .collect()
}

/// Like [`discover_claude_path`] but skips paths in `excluded` (used by tmux-emit to avoid reusing claimed transcripts).
pub fn discover_claude_path_excluding(cwd: &Path, excluded: &HashSet<PathBuf>) -> Option<PathBuf> {
    discover_claude_paths(cwd)
        .into_iter()
        .find(|path| !excluded.contains(path))
}

fn claude_file_matches_cwd(path: &Path, cwd: &Path) -> bool {
    fs::File::open(path)
        .ok()
        .map(std::io::BufReader::new)
        .is_some_and(|reader| reader_matches_or_lacks_cwd(reader, &cwd.display().to_string()))
}

/// Like [`discover_codex_path`] but skips paths in `excluded`.
pub fn discover_codex_path_excluding(cwd: &Path, excluded: &HashSet<PathBuf>) -> Option<PathBuf> {
    discover_codex_paths(cwd)
        .into_iter()
        .find(|path| !excluded.contains(path))
}

fn codex_file_matches_cwd(path: &Path, cwd: &Path) -> bool {
    let cwd_str = cwd.display().to_string();
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut lines = std::io::BufReader::new(file).lines();
    let first_line = match lines.next() {
        Some(Ok(line)) => line,
        _ => return false,
    };

    let value: Value = match serde_json::from_str(&first_line) {
        Ok(value) => value,
        Err(_) => return false,
    };

    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return false;
    }

    value
        .get("payload")
        .and_then(|payload| payload.get("cwd"))
        .and_then(Value::as_str)
        .map(|entry_cwd| entry_cwd == cwd_str)
        .unwrap_or(false)
}

fn sorted_numeric_subdirs_reverse(dir: &Path, width: usize) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_type()
                    .ok()
                    .map(|ft| ft.is_dir())
                    .unwrap_or(false)
            })
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.len() == width && name.chars().all(|ch| ch.is_ascii_digit()))
                    .unwrap_or(false)
            })
            .map(|entry| entry.path())
            .collect(),
        Err(_) => Vec::new(),
    };

    dirs.sort();
    dirs.reverse();
    dirs
}

fn modified_or_epoch(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn home_dir() -> Option<PathBuf> {
    // An empty HOME ("") must fail discovery rather than resolve to a relative
    // `.claude/projects/...` path under the process cwd, which would silently
    // scan an unrelated location. Treat empty like unset.
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn parsed_line_value(line: &str) -> Option<Value> {
    let trimmed = line.trim();
    (!trimmed.is_empty())
        .then_some(trimmed)
        .and_then(|trimmed| serde_json::from_str(trimmed).ok())
}

fn infer_tool_from_entry(value: &Value) -> Option<AgentTool> {
    let entry_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    codex_entry_tool(entry_type)
        .or_else(|| claude_entry_tool(value, entry_type))
        .or_else(|| value.get("payload").map(|_| AgentTool::Codex))
}

fn codex_entry_tool(entry_type: &str) -> Option<AgentTool> {
    matches!(
        entry_type,
        "session_meta" | "response" | "response_item" | "event_msg"
    )
    .then_some(AgentTool::Codex)
}

fn claude_entry_tool(value: &Value, entry_type: &str) -> Option<AgentTool> {
    matches!(entry_type, "assistant" | "user")
        .then_some(AgentTool::Claude)
        .or_else(|| value.get("message").map(|_| AgentTool::Claude))
}

fn discovered_path_for_tool(cwd: &Path, tool: AgentTool) -> Option<PathBuf> {
    match tool {
        AgentTool::Claude => discover_claude_path(cwd),
        AgentTool::Codex => discover_codex_path(cwd),
    }
}

fn discovered_input(tool: AgentTool, path: PathBuf) -> ResolvedInput {
    ResolvedInput {
        tool,
        path,
        discovered: true,
    }
}

fn newer_discovered_input(claude: PathBuf, codex: PathBuf) -> ResolvedInput {
    if modified_or_epoch(&codex) > modified_or_epoch(&claude) {
        discovered_input(AgentTool::Codex, codex)
    } else {
        discovered_input(AgentTool::Claude, claude)
    }
}

fn codex_paths_in_year(year: &Path, cwd: &Path) -> Vec<PathBuf> {
    sorted_numeric_subdirs_reverse(year, 2)
        .into_iter()
        .flat_map(|month| codex_paths_in_month(&month, cwd))
        .collect()
}

fn codex_paths_in_month(month: &Path, cwd: &Path) -> Vec<PathBuf> {
    sorted_numeric_subdirs_reverse(month, 2)
        .into_iter()
        .flat_map(|day| matching_codex_rollouts(&day, cwd))
        .collect()
}

fn matching_codex_rollouts(day: &Path, cwd: &Path) -> Vec<PathBuf> {
    codex_rollout_files(day)
        .into_iter()
        .filter(|path| codex_file_matches_cwd(path, cwd))
        .collect()
}

fn codex_rollout_files(day: &Path) -> Vec<PathBuf> {
    let mut rollout_files: Vec<PathBuf> = match fs::read_dir(day) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                name.starts_with("rollout-") && name.ends_with(".jsonl")
            })
            .collect(),
        Err(_) => return Vec::new(),
    };

    rollout_files.sort();
    rollout_files.reverse();
    rollout_files
}

const DISCOVERY_VALID_JSON_SCAN_LIMIT: usize = 64;
const DISCOVERY_PHYSICAL_LINE_SCAN_LIMIT: usize = 4096;

fn reader_matches_or_lacks_cwd<R: BufRead>(reader: R, cwd_str: &str) -> bool {
    let mut saw_valid_json = false;
    let mut saw_cwd = false;
    let mut valid_json_lines = 0usize;
    let mut hit_physical_line_limit = false;

    for (physical_line_index, line) in reader.lines().enumerate() {
        if physical_line_index >= DISCOVERY_PHYSICAL_LINE_SCAN_LIMIT {
            hit_physical_line_limit = true;
            break;
        }

        let Ok(line) = line else {
            break;
        };
        let Some(value) = parsed_line_value(&line) else {
            continue;
        };

        saw_valid_json = true;
        valid_json_lines += 1;
        if let Some(entry_cwd) = value.get("cwd").and_then(Value::as_str) {
            saw_cwd = true;
            if entry_cwd == cwd_str {
                return true;
            }
        }
        if valid_json_lines >= DISCOVERY_VALID_JSON_SCAN_LIMIT {
            break;
        }
    }

    saw_valid_json && !saw_cwd && !hit_physical_line_limit
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Mutex, OnceLock};

    pub(crate) fn home_env_lock() -> &'static Mutex<()> {
        static HOME_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        HOME_ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Single mutex shared across every test that mutates process-wide env
    /// vars consumed by the model client and engine
    /// (`OPENROUTER_API_KEY`, `CLAWGS_GROK_BIN`, `SWIMMERS_THOUGHT_MODEL*`, etc.).
    /// Without a single shared lock, model_client tests racing engine tests
    /// can leave the wrong env state visible to whichever test reads first.
    pub(crate) fn process_env_lock() -> &'static Mutex<()> {
        static PROCESS_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        PROCESS_ENV_LOCK.get_or_init(|| Mutex::new(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    #[test]
    fn infer_codex_tool_from_response_item() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\"}}\n",
        )
        .expect("write file");

        let tool = infer_tool_from_file(file.path()).expect("infer tool");
        assert_eq!(tool, AgentTool::Codex);
    }

    #[test]
    fn infer_claude_tool_from_assistant_message() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}\n",
        )
        .expect("write file");

        let tool = infer_tool_from_file(file.path()).expect("infer tool");
        assert_eq!(tool, AgentTool::Claude);
    }

    #[test]
    fn extract_jsonl_str_uses_virtual_source_and_preserves_stats() {
        let input = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/demo\"}}\n",
            "not-json\n",
            "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Build a parser\"}]}}\n"
        );

        let output = extract_jsonl_str(
            AgentTool::Codex,
            "embedded:test.jsonl",
            input,
            std::path::Path::new("/demo"),
            false,
            &ExtractOptions::default(),
        )
        .expect("extract from string");

        assert_eq!(output.source.tool, "codex");
        assert_eq!(output.source.path, "embedded:test.jsonl");
        assert!(!output.source.discovered);
        assert_eq!(output.source.cwd, "/demo");
        assert_eq!(output.snapshot.user_task.as_deref(), Some("Build a parser"));
        assert_eq!(output.stats.events_seen, 2);
        assert_eq!(output.stats.malformed_lines_skipped, 1);
        assert_eq!(output.stats.bytes_read, input.len() as u64);
    }

    #[test]
    fn codex_file_matches_cwd_returns_false_for_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist.jsonl");
        assert!(!codex_file_matches_cwd(&missing, dir.path()));
    }

    #[test]
    fn codex_file_matches_cwd_returns_false_for_empty_file() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), b"").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!codex_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn codex_file_matches_cwd_returns_false_for_malformed_first_line() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), b"this is not json\n").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!codex_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn codex_file_matches_cwd_returns_false_when_first_line_is_not_session_meta() {
        let file = NamedTempFile::new().expect("temp file");
        // Valid JSON but not a session_meta row, so no cwd inference applies.
        fs::write(file.path(), b"{\"type\":\"response_item\"}\n").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!codex_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn codex_file_matches_cwd_returns_false_when_payload_cwd_is_missing() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), b"{\"type\":\"session_meta\",\"payload\":{}}\n").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!codex_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn codex_file_matches_cwd_returns_false_when_payload_cwd_does_not_match() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            b"{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/some/other\"}}\n",
        )
        .expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!codex_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn codex_file_matches_cwd_returns_true_when_payload_cwd_matches_exactly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd_str = dir.path().display().to_string();
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{cwd_str}\"}}}}\n"),
        )
        .expect("write");
        assert!(codex_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn claude_file_matches_cwd_returns_false_for_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist.jsonl");
        assert!(!claude_file_matches_cwd(&missing, dir.path()));
    }

    #[test]
    fn claude_file_matches_cwd_returns_false_when_no_parseable_jsonl_lines() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), b"not-json\nalso-not-json\n").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!claude_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn claude_file_matches_cwd_returns_true_when_valid_json_lacks_cwd() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            b"{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}\n",
        )
        .expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(claude_file_matches_cwd(file.path(), dir.path()));
    }

    #[test]
    fn claude_discovery_counts_parseable_jsonl_lines_not_physical_lines() {
        let cwd = PathBuf::from("/tmp/target-project");
        let other_cwd = PathBuf::from("/tmp/other-project");
        let file = NamedTempFile::new().expect("temp file");
        let mut lines =
            vec!["{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}".to_string()];
        lines.extend((0..70).map(|_| "not-json".to_string()));
        lines.push(format!(
            "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"wrong\"}}}}",
            other_cwd.display()
        ));
        fs::write(file.path(), format!("{}\n", lines.join("\n"))).expect("write");

        assert!(
            !claude_file_matches_cwd(file.path(), &cwd),
            "mismatched cwd evidence must count even when malformed lines precede it"
        );
    }

    #[test]
    fn claude_discovery_does_not_fallback_after_physical_line_cap() {
        let cwd = PathBuf::from("/tmp/target-project");
        let file = NamedTempFile::new().expect("temp file");
        let mut lines =
            vec!["{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}".to_string()];
        lines.extend((0..DISCOVERY_PHYSICAL_LINE_SCAN_LIMIT).map(|_| "not-json".to_string()));
        fs::write(file.path(), format!("{}\n", lines.join("\n"))).expect("write");

        assert!(
            !claude_file_matches_cwd(file.path(), &cwd),
            "legacy no-cwd fallback needs bounded evidence before the physical line cap"
        );
    }

    #[test]
    fn discover_auto_errors_when_home_is_missing() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let original_home = std::env::var_os("HOME");
        std::env::remove_var("HOME");
        let cwd = PathBuf::from("/tmp/no-home-project");

        let err = discover_auto(&cwd).expect_err("missing HOME should not discover transcripts");
        let message = err.to_string();
        assert!(
            message.contains("no Claude or Codex transcript JSONL found"),
            "got: {message}"
        );
        assert!(message.contains("--input <path/to/session.jsonl>"));

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        }
    }

    #[test]
    fn discover_auto_errors_when_home_is_empty() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let original_home = std::env::var_os("HOME");
        // An empty HOME must fail discovery, not resolve to a relative
        // `.claude/projects/...` path scanned under the process cwd.
        std::env::set_var("HOME", "");
        let cwd = PathBuf::from("/tmp/empty-home-project");

        let err = discover_auto(&cwd).expect_err("empty HOME should not discover transcripts");
        assert!(
            err.to_string()
                .contains("no Claude or Codex transcript JSONL found"),
            "got: {err}"
        );

        match original_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn infer_tool_skips_unreadable_line_before_a_valid_marker() {
        // Line 1 is valid JSON without a tool marker, line 2 is invalid UTF-8,
        // and the identifying marker is on line 3. Inference must skip the bad
        // line rather than abort with an io error.
        let file = NamedTempFile::new().expect("temp file");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"{\"unrelated\":1}\n");
        bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
        bytes.extend_from_slice(b"{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}\n");
        fs::write(file.path(), bytes).expect("write file");

        let tool = infer_tool_from_file(file.path()).expect("infer tool past the bad line");
        assert_eq!(tool, AgentTool::Claude);
    }

    #[test]
    fn claude_discovery_breaks_mtime_ties_deterministically_by_path() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = PathBuf::from("/tmp/mtime-tie-project");
        let cwd_slug = cwd.display().to_string().replace('/', "-");
        let project_dir = tmp.path().join(".claude").join("projects").join(cwd_slug);
        fs::create_dir_all(&project_dir).expect("mkdir");

        let line = format!(
            "{{\"type\":\"assistant\",\"cwd\":\"{}\",\"message\":{{\"role\":\"assistant\"}}}}\n",
            cwd.display()
        );
        let a = project_dir.join("session-a.jsonl");
        let z = project_dir.join("session-z.jsonl");
        fs::write(&a, &line).expect("write a");
        fs::write(&z, &line).expect("write z");
        // Force identical mtimes so only the path tiebreak distinguishes them.
        let shared = fs::metadata(&a).expect("meta").modified().expect("mtime");
        for path in [&a, &z] {
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .and_then(|file| file.set_modified(shared))
                .expect("pin mtime");
        }
        std::env::set_var("HOME", tmp.path());

        // Descending path order wins ties, so "session-z" sorts ahead of
        // "session-a" regardless of readdir order.
        assert_eq!(discover_claude_path(&cwd), Some(z));
    }

    #[test]
    fn infer_tool_errors_when_no_line_carries_a_tool_marker() {
        let file = NamedTempFile::new().expect("temp file");
        // Two lines with no tool-shaped fields and one malformed JSON line —
        // none should yield an inference, so the for-loop must exhaust and
        // the function must surface the help-text error.
        fs::write(
            file.path(),
            "{\"unrelated\":1}\n{\"another\":\"thing\"}\nnot-json\n",
        )
        .expect("write file");

        let err = infer_tool_from_file(file.path())
            .expect_err("inference should fail on unrecognized content");
        let message = err.to_string();
        assert!(
            message.contains("could not infer tool format"),
            "got: {message}"
        );
        assert!(message.contains("--tool claude or --tool codex"));
    }

    #[test]
    fn infer_tool_errors_when_input_path_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("not-here.jsonl");
        let err = infer_tool_from_file(&missing).expect_err("must fail without file");
        assert!(err.to_string().contains("failed to open input file"));
    }

    #[test]
    fn infer_codex_tool_from_payload_marker() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), "{\"payload\":{\"cwd\":\"/tmp/project\"}}\n").expect("write file");

        let tool = infer_tool_from_file(file.path()).expect("infer tool");
        assert_eq!(tool, AgentTool::Codex);
    }

    #[test]
    fn resolve_input_explicit_path_auto_infers_tool_from_contents() {
        // Auto + explicit `--input`: tool is inferred from the file, and the
        // result is marked as not discovered (the user supplied the path).
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\"}}\n",
        )
        .expect("write file");
        let cwd = PathBuf::from("/tmp/ignored-when-input-given");

        let resolved = resolve_input(ToolSelection::Auto, &cwd, Some(file.path()))
            .expect("resolve explicit codex input");

        assert_eq!(resolved.tool, AgentTool::Codex);
        assert_eq!(resolved.path, file.path());
        assert!(
            !resolved.discovered,
            "an explicitly supplied path is never reported as discovered"
        );
    }

    #[test]
    fn resolve_input_explicit_tool_overrides_file_contents() {
        // Claude/Codex + explicit `--input`: the requested tool wins outright
        // and the file is NOT sniffed (so a Claude-shaped file can be force-read
        // as Codex without an inference round-trip).
        let claude_file = NamedTempFile::new().expect("temp file");
        fs::write(
            claude_file.path(),
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}\n",
        )
        .expect("write file");
        let cwd = PathBuf::from("/tmp/ignored");

        let forced_codex = resolve_input(ToolSelection::Codex, &cwd, Some(claude_file.path()))
            .expect("force codex");
        assert_eq!(
            forced_codex.tool,
            AgentTool::Codex,
            "explicit --tool codex must override Claude-shaped file contents"
        );
        assert!(!forced_codex.discovered);

        let forced_claude = resolve_input(ToolSelection::Claude, &cwd, Some(claude_file.path()))
            .expect("force claude");
        assert_eq!(forced_claude.tool, AgentTool::Claude);
    }

    #[test]
    fn resolve_input_explicit_auto_path_errors_when_tool_unidentifiable() {
        // Auto + explicit path whose contents carry no tool marker must surface
        // the inference error (propagated from `infer_tool_from_file`) rather
        // than silently defaulting to a tool.
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), "{\"unrelated\":1}\n").expect("write file");
        let cwd = PathBuf::from("/tmp/ignored");

        let err = resolve_input(ToolSelection::Auto, &cwd, Some(file.path()))
            .expect_err("unidentifiable explicit input should error");
        assert!(
            err.to_string().contains("could not infer tool format"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_input_without_path_falls_through_to_discovery() {
        // No `--input`: with an empty HOME, discovery finds nothing and the
        // error path (the discovery arm of resolve_input) is exercised for each
        // selection. Guarded by the HOME mutex like other discovery tests.
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let original_home = std::env::var_os("HOME");
        std::env::set_var("HOME", "");
        let cwd = PathBuf::from("/tmp/resolve-input-no-discovery");

        let auto_err = resolve_input(ToolSelection::Auto, &cwd, None)
            .expect_err("auto discovery should find nothing");
        assert!(auto_err
            .to_string()
            .contains("no Claude or Codex transcript JSONL found"));

        let claude_err = resolve_input(ToolSelection::Claude, &cwd, None)
            .expect_err("claude discovery should find nothing");
        assert!(claude_err
            .to_string()
            .contains("no claude transcript JSONL"));

        let codex_err = resolve_input(ToolSelection::Codex, &cwd, None)
            .expect_err("codex discovery should find nothing");
        assert!(codex_err.to_string().contains("no codex transcript JSONL"));

        match original_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    /// Helper: create a fake Claude project dir under a temp HOME with JSONL files.
    /// Returns (temp_dir, cwd, vec of created file paths sorted oldest-first).
    fn setup_claude_project_dir(
        cwd_path: &str,
        file_count: usize,
    ) -> (tempfile::TempDir, PathBuf, Vec<PathBuf>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = PathBuf::from(cwd_path);
        let cwd_slug = cwd.display().to_string().replace('/', "-");
        let project_dir = tmp.path().join(".claude").join("projects").join(cwd_slug);
        fs::create_dir_all(&project_dir).expect("mkdir");

        let mut paths = Vec::new();
        for i in 0..file_count {
            let file_path = project_dir.join(format!("session-{i}.jsonl"));
            let line = format!(
                "{{\"type\":\"assistant\",\"cwd\":\"{}\",\"message\":{{\"role\":\"assistant\"}}}}\n",
                cwd.display()
            );
            fs::write(&file_path, line).expect("write");
            paths.push(file_path);
            // Ensure distinct mtime ordering
            thread::sleep(Duration::from_millis(50));
        }
        (tmp, cwd, paths)
    }

    #[test]
    fn excluding_empty_set_returns_newest() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let (tmp, cwd, paths) = setup_claude_project_dir("/tmp/project", 2);
        std::env::set_var("HOME", tmp.path());
        let result = discover_claude_path_excluding(&cwd, &HashSet::new());
        assert_eq!(result, Some(paths[1].clone()), "should return newest file");
    }

    #[test]
    fn excluding_newest_returns_second() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let (tmp, cwd, paths) = setup_claude_project_dir("/tmp/project-a", 2);
        std::env::set_var("HOME", tmp.path());
        let mut excluded = HashSet::new();
        excluded.insert(paths[1].clone());
        let result = discover_claude_path_excluding(&cwd, &excluded);
        assert_eq!(
            result,
            Some(paths[0].clone()),
            "should return second-newest when newest excluded"
        );
    }

    #[test]
    fn excluding_all_returns_none() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let (tmp, cwd, paths) = setup_claude_project_dir("/tmp/project-b", 1);
        std::env::set_var("HOME", tmp.path());
        let mut excluded = HashSet::new();
        excluded.insert(paths[0].clone());
        let result = discover_claude_path_excluding(&cwd, &excluded);
        assert_eq!(result, None, "should return None when all files excluded");
    }

    #[test]
    fn exclusion_does_not_cross_cwd_boundaries() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let (tmp, _cwd_a, paths_a) = setup_claude_project_dir("/tmp/project-c", 1);
        // Create a second project dir under the same HOME
        let cwd_b = PathBuf::from("/tmp/project-d");
        let slug_b = cwd_b.display().to_string().replace('/', "-");
        let dir_b = tmp.path().join(".claude").join("projects").join(slug_b);
        fs::create_dir_all(&dir_b).expect("mkdir");
        let file_b = dir_b.join("session-0.jsonl");
        fs::write(
            &file_b,
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}\n",
        )
        .expect("write");

        std::env::set_var("HOME", tmp.path());

        // Exclude a path from project A
        let mut excluded = HashSet::new();
        excluded.insert(paths_a[0].clone());

        // Project B should still find its file unaffected
        let result = discover_claude_path_excluding(&cwd_b, &excluded);
        assert_eq!(
            result,
            Some(file_b),
            "exclusion from different CWD should not affect discovery"
        );
    }

    #[test]
    fn claude_discovery_filters_colliding_slug_by_exact_cwd() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().expect("tempdir");

        let cwd_a = PathBuf::from("/tmp/a-b/c");
        let cwd_b = PathBuf::from("/tmp/a/b-c");
        let slug_a = cwd_a.display().to_string().replace('/', "-");
        let slug_b = cwd_b.display().to_string().replace('/', "-");
        assert_eq!(slug_a, slug_b, "test requires slug collision");

        let project_dir = tmp.path().join(".claude").join("projects").join(&slug_a);
        fs::create_dir_all(&project_dir).expect("mkdir");

        // Older file for cwd_a
        let file_a = project_dir.join("session-a.jsonl");
        fs::write(
            &file_a,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"TASK_A\"}}}}\n",
                cwd_a.display()
            ),
        )
        .expect("write");
        thread::sleep(Duration::from_millis(50));

        // Newer file for cwd_b (same slug dir due collision)
        let file_b = project_dir.join("session-b.jsonl");
        fs::write(
            &file_b,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"TASK_B\"}}}}\n",
                cwd_b.display()
            ),
        )
        .expect("write");

        std::env::set_var("HOME", tmp.path());

        let found_plain = discover_claude_path(&cwd_a);
        assert_eq!(
            found_plain,
            Some(file_a.clone()),
            "plain discovery should ignore newer mismatched-cwd file"
        );

        let found_excluding = discover_claude_path_excluding(&cwd_a, &HashSet::new());
        assert_eq!(
            found_excluding,
            Some(file_a),
            "excluding discovery should ignore newer mismatched-cwd file"
        );
    }

    #[test]
    fn claude_discovery_exclusion_isolates_same_cwd_sessions() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let (tmp, cwd, paths) = setup_claude_project_dir("/tmp/shared-cwd", 2);
        std::env::set_var("HOME", tmp.path());

        let first = discover_claude_path_excluding(&cwd, &HashSet::new())
            .expect("first discovery should find newest file");
        let mut excluded = HashSet::new();
        excluded.insert(first.clone());
        let second = discover_claude_path_excluding(&cwd, &excluded)
            .expect("second discovery should find non-excluded file");

        assert_ne!(first, second, "same-cwd sessions must not claim same file");
        assert_eq!(first, paths[1], "first claim should be newest file");
        assert_eq!(second, paths[0], "second claim should be next newest file");
    }

    #[test]
    fn discover_for_tool_finds_codex_rollout() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = PathBuf::from("/tmp/codex-project");
        let codex_day = tmp
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("03")
            .join("16");
        fs::create_dir_all(&codex_day).expect("mkdir");
        let rollout = codex_day.join("rollout-a.jsonl");
        fs::write(
            &rollout,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                cwd.display()
            ),
        )
        .expect("write");
        std::env::set_var("HOME", tmp.path());

        let resolved = discover_for_tool(&cwd, AgentTool::Codex).expect("discover codex");

        assert_eq!(resolved.tool, AgentTool::Codex);
        assert_eq!(resolved.path, rollout);
        assert!(resolved.discovered);
    }

    #[test]
    fn codex_discovery_skips_non_numeric_dirs_non_rollouts_and_malformed_headers() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = PathBuf::from("/tmp/codex-filter-project");
        std::env::set_var("HOME", tmp.path());

        let sessions = tmp.path().join(".codex").join("sessions");
        let ignored_non_numeric = sessions.join("latest").join("05").join("16");
        fs::create_dir_all(&ignored_non_numeric).expect("mkdir");
        fs::write(
            ignored_non_numeric.join("rollout-newer.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                cwd.display()
            ),
        )
        .expect("write non-numeric candidate");

        let ignored_non_rollout = sessions.join("2027").join("05").join("16");
        fs::create_dir_all(&ignored_non_rollout).expect("mkdir");
        fs::write(
            ignored_non_rollout.join("session-newer.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                cwd.display()
            ),
        )
        .expect("write non-rollout candidate");

        let malformed_day = sessions.join("2026").join("03").join("17");
        fs::create_dir_all(&malformed_day).expect("mkdir");
        fs::write(malformed_day.join("rollout-z.jsonl"), b"not-json\n")
            .expect("write malformed candidate");

        let valid_day = sessions.join("2026").join("03").join("16");
        fs::create_dir_all(&valid_day).expect("mkdir");
        let valid = valid_day.join("rollout-a.jsonl");
        fs::write(
            &valid,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                cwd.display()
            ),
        )
        .expect("write valid candidate");

        assert_eq!(discover_codex_path(&cwd), Some(valid));
    }

    #[test]
    fn codex_excluding_newest_returns_next_matching_rollout() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = PathBuf::from("/tmp/codex-shared-cwd");
        std::env::set_var("HOME", tmp.path());

        let codex_day = tmp
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("03")
            .join("16");
        fs::create_dir_all(&codex_day).expect("mkdir");
        let older = codex_day.join("rollout-a.jsonl");
        let newer = codex_day.join("rollout-z.jsonl");
        let line = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
            cwd.display()
        );
        fs::write(&older, &line).expect("write older");
        fs::write(&newer, &line).expect("write newer");

        let mut excluded = HashSet::new();
        excluded.insert(newer);

        assert_eq!(discover_codex_path_excluding(&cwd, &excluded), Some(older));
    }

    #[test]
    fn discover_auto_prefers_newer_codex_rollout() {
        let _lock = crate::test_support::home_env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = PathBuf::from("/tmp/mixed-project");
        std::env::set_var("HOME", tmp.path());

        let cwd_slug = cwd.display().to_string().replace('/', "-");
        let claude_dir = tmp.path().join(".claude").join("projects").join(cwd_slug);
        fs::create_dir_all(&claude_dir).expect("mkdir");
        let claude_file = claude_dir.join("session-a.jsonl");
        fs::write(
            &claude_file,
            format!(
                "{{\"type\":\"assistant\",\"cwd\":\"{}\",\"message\":{{\"role\":\"assistant\"}}}}\n",
                cwd.display()
            ),
        )
        .expect("write");
        thread::sleep(Duration::from_millis(50));

        let codex_day = tmp
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("03")
            .join("16");
        fs::create_dir_all(&codex_day).expect("mkdir");
        let codex_file = codex_day.join("rollout-z.jsonl");
        fs::write(
            &codex_file,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}\"}}}}\n",
                cwd.display()
            ),
        )
        .expect("write");

        let resolved = discover_auto(&cwd).expect("discover newest");

        assert_eq!(resolved.tool, AgentTool::Codex);
        assert_eq!(resolved.path, codex_file);
    }

    #[test]
    fn extract_output_roundtrips_through_json() {
        let jsonl = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/test\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello\"}}\n",
            "{\"type\":\"response\",\"payload\":{\"usage\":{\"input_tokens\":42}}}\n",
        );
        let output = extract_jsonl_str(
            AgentTool::Codex,
            "roundtrip-test",
            jsonl,
            std::path::Path::new("/tmp/test"),
            false,
            &ExtractOptions::default(),
        )
        .expect("extract");
        let json = serde_json::to_string(&output).expect("serialize");
        let back: ExtractOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.schema_version, output.schema_version);
        assert_eq!(back.snapshot.user_task, output.snapshot.user_task);
        assert_eq!(back.stats.events_seen, output.stats.events_seen);
    }
}
