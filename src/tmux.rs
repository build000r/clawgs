use std::collections::{HashMap, HashSet};
use std::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::emit::protocol::{
    RestState, SessionSnapshot, SessionState, ThoughtSource, ThoughtState,
};

const FIELD_SEP: char = '\u{1f}';

pub fn tmux_bin() -> String {
    std::env::var("CLAWGS_TMUX_BIN").unwrap_or_else(|_| "tmux".to_string())
}

/// One-shot tmux scan.
///
/// This is intentionally stateless: callers that want activity aging across
/// repeated scans must keep a `TmuxScanTracker` and reuse it.
pub fn scan_sessions(now: DateTime<Utc>, max_capture_lines: usize) -> Result<Vec<SessionSnapshot>> {
    let mut tracker = TmuxScanTracker::new();
    tracker.scan_with_bin(now, max_capture_lines, &tmux_bin())
}

/// One-shot tmux scan against an explicit tmux binary.
///
/// This is intentionally stateless: callers that want activity aging across
/// repeated scans must keep a `TmuxScanTracker` and reuse it.
pub fn scan_sessions_with_bin(
    now: DateTime<Utc>,
    max_capture_lines: usize,
    tmux_bin: &str,
) -> Result<Vec<SessionSnapshot>> {
    let mut tracker = TmuxScanTracker::new();
    tracker.scan_with_bin(now, max_capture_lines, tmux_bin)
}

pub struct TmuxScanTracker {
    sessions: HashMap<String, TrackedSession>,
}

impl TmuxScanTracker {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn scan(
        &mut self,
        now: DateTime<Utc>,
        max_capture_lines: usize,
    ) -> Result<Vec<SessionSnapshot>> {
        self.scan_with_bin(now, max_capture_lines, &tmux_bin())
    }

    pub fn scan_with_bin(
        &mut self,
        now: DateTime<Utc>,
        max_capture_lines: usize,
        tmux_bin: &str,
    ) -> Result<Vec<SessionSnapshot>> {
        let stdout = list_tmux_panes(tmux_bin)?;
        let observations: Vec<_> = stdout
            .lines()
            .filter_map(parse_pane_meta_line)
            .filter_map(|meta| pane_meta_to_observation(max_capture_lines, tmux_bin, meta))
            .collect();

        let live_ids: HashSet<_> = observations
            .iter()
            .map(|observation| observation.session_id.clone())
            .collect();
        self.sessions
            .retain(|session_id, _| live_ids.contains(session_id));

        Ok(observations
            .into_iter()
            .map(|observation| self.apply_observation(now, observation))
            .collect())
    }

    fn apply_observation(
        &mut self,
        now: DateTime<Utc>,
        observation: SessionObservation,
    ) -> SessionSnapshot {
        // tmux can tell us which pane is selected, but that is focus, not work.
        // Treat visible pane changes as activity and preserve the prior
        // timestamp across identical scans so idle/sleeping can emerge.
        let previous = self.sessions.get(&observation.session_id);
        let observed_activity = previous
            .map(|state| state.changed(&observation))
            .unwrap_or(false);
        let last_activity_at = match previous {
            Some(state) if !observed_activity => state.last_activity_at,
            _ => now,
        };
        let sticky_busy =
            sticky_busy_state(&observation.current_command, observation.tool.as_deref());
        let bootstrap_busy = previous.is_none()
            && bootstrap_busy(&observation.current_command, observation.tool.as_deref());
        let state = if observed_activity || bootstrap_busy || sticky_busy {
            SessionState::Busy
        } else {
            SessionState::Idle
        };

        let session_id = observation.session_id.clone();
        let tool = observation.tool.clone();
        let cwd = observation.cwd.clone();
        let replay_text = observation.replay_text.clone();

        self.sessions.insert(
            session_id.clone(),
            TrackedSession::from_observation(observation, last_activity_at),
        );

        SessionSnapshot {
            session_id,
            state,
            exited: false,
            tool,
            cwd,
            replay_text,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            objective_fingerprint: None,
            thought_updated_at: None,
            token_count: 0,
            context_limit: 0,
            last_activity_at,
            rest_state: RestState::Active,
            commit_candidate: false,
        }
    }
}

impl Default for TmuxScanTracker {
    fn default() -> Self {
        Self::new()
    }
}

fn list_tmux_panes(tmux_bin: &str) -> Result<String> {
    let format = format!(
        "#{{session_name}}{sep}#{{window_index}}{sep}#{{pane_index}}{sep}#{{pane_id}}{sep}#{{pane_current_path}}{sep}#{{pane_current_command}}{sep}#{{?pane_dead,1,0}}",
        sep = FIELD_SEP
    );

    let output = Command::new(tmux_bin)
        .args(["list-panes", "-a", "-F", &format])
        .output()
        .with_context(|| format!("failed to run {tmux_bin} list-panes"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if tmux_server_missing(&stderr) {
            return Ok(String::new());
        }

        anyhow::bail!(
            "{tmux_bin} list-panes failed: {}",
            stderr.trim().replace('\n', " ")
        );
    }

    String::from_utf8(output.stdout).context("tmux list-panes output was not UTF-8")
}

#[derive(Debug, PartialEq, Eq)]
struct PaneMeta {
    session_name: String,
    window_index: String,
    pane_index: String,
    pane_id: String,
    current_path: String,
    current_command: String,
    dead: bool,
}

fn parse_pane_line(line: &str) -> Option<PaneMeta> {
    let mut parts = line.split(FIELD_SEP);

    Some(PaneMeta {
        session_name: parts.next()?.to_string(),
        window_index: parts.next()?.to_string(),
        pane_index: parts.next()?.to_string(),
        pane_id: parts.next()?.to_string(),
        current_path: parts.next()?.to_string(),
        current_command: parts.next()?.to_string(),
        dead: parts.next()? == "1",
    })
}

fn parse_pane_meta_line(line: &str) -> Option<PaneMeta> {
    let trimmed = line.trim_end();
    (!trimmed.is_empty())
        .then(|| parse_pane_line(trimmed))
        .flatten()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionObservation {
    session_id: String,
    tool: Option<String>,
    cwd: String,
    replay_text: String,
    current_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedSession {
    cwd: String,
    replay_text: String,
    current_command: String,
    last_activity_at: DateTime<Utc>,
}

impl TrackedSession {
    fn from_observation(observation: SessionObservation, last_activity_at: DateTime<Utc>) -> Self {
        Self {
            cwd: observation.cwd,
            replay_text: observation.replay_text,
            current_command: observation.current_command,
            last_activity_at,
        }
    }

    fn changed(&self, observation: &SessionObservation) -> bool {
        self.cwd != observation.cwd
            || self.replay_text != observation.replay_text
            || self.current_command != observation.current_command
    }
}

fn pane_meta_to_observation(
    max_capture_lines: usize,
    tmux_bin: &str,
    meta: PaneMeta,
) -> Option<SessionObservation> {
    (!meta.dead).then(|| build_session_observation(max_capture_lines, tmux_bin, meta))
}

fn build_session_observation(
    max_capture_lines: usize,
    tmux_bin: &str,
    meta: PaneMeta,
) -> SessionObservation {
    let replay_text =
        capture_pane_text(tmux_bin, &meta.pane_id, max_capture_lines).unwrap_or_default();

    SessionObservation {
        session_id: format!(
            "tmux:{}:{}.{}:{}",
            meta.session_name, meta.window_index, meta.pane_index, meta.pane_id
        ),
        tool: infer_tool(&meta.current_command),
        cwd: meta.current_path,
        replay_text,
        current_command: meta.current_command,
    }
}

fn capture_pane_text(tmux_bin: &str, pane_id: &str, max_capture_lines: usize) -> Result<String> {
    let start = capture_start(max_capture_lines);
    let output = Command::new(tmux_bin)
        .args(["capture-pane", "-p", "-t", pane_id, "-S", &start])
        .output()
        .with_context(|| format!("failed to run {tmux_bin} capture-pane for {pane_id}"))?;

    if !output.status.success() {
        return Ok(String::new());
    }

    let stdout =
        String::from_utf8(output.stdout).context("tmux capture-pane output was not UTF-8")?;
    Ok(stdout.trim().to_string())
}

fn capture_start(max_capture_lines: usize) -> String {
    let lines = max_capture_lines.max(1);
    format!("-{}", lines.saturating_sub(1))
}

fn infer_tool(current_command: &str) -> Option<String> {
    let normalized = current_command.trim().to_lowercase();
    ["claude", "codex"]
        .into_iter()
        .find(|tool| normalized.contains(tool))
        .map(|tool| tool.to_string())
}

fn bootstrap_busy(current_command: &str, tool: Option<&str>) -> bool {
    tool.is_some() || sticky_busy_state(current_command, tool)
}

fn sticky_busy_state(current_command: &str, tool: Option<&str>) -> bool {
    tool.is_none()
        && !normalized_command(current_command).is_empty()
        && !is_shell_command(current_command)
}

fn normalized_command(current_command: &str) -> String {
    current_command.trim().to_ascii_lowercase()
}

fn is_shell_command(current_command: &str) -> bool {
    let current_command = normalized_command(current_command);
    matches!(
        current_command.as_str(),
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "tcsh" | "csh"
    )
}

fn tmux_server_missing(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    [
        "no server running",
        "failed to connect to server",
        "no sessions",
    ]
    .iter()
    .any(|fragment| lower.contains(fragment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pane_line_decodes_tmux_fields() {
        let line = "work\u{1f}1\u{1f}0\u{1f}%3\u{1f}/tmp/project\u{1f}codex\u{1f}0";
        let parsed = parse_pane_line(line).expect("pane meta");

        assert_eq!(
            parsed,
            PaneMeta {
                session_name: "work".to_string(),
                window_index: "1".to_string(),
                pane_index: "0".to_string(),
                pane_id: "%3".to_string(),
                current_path: "/tmp/project".to_string(),
                current_command: "codex".to_string(),
                dead: false,
            }
        );
    }

    #[test]
    fn capture_start_keeps_one_line_minimum() {
        assert_eq!(capture_start(0), "-0");
        assert_eq!(capture_start(1), "-0");
        assert_eq!(capture_start(200), "-199");
    }

    #[test]
    fn tmux_server_missing_recognizes_expected_errors() {
        assert!(tmux_server_missing("No server running on /tmp/tmux"));
        assert!(tmux_server_missing("failed to connect to server"));
        assert!(tmux_server_missing("no sessions"));
        assert!(!tmux_server_missing("permission denied"));
    }

    #[test]
    fn infer_tool_matches_supported_agents() {
        assert_eq!(infer_tool("  Claude  ").as_deref(), Some("claude"));
        assert_eq!(
            infer_tool("/usr/bin/codex --json").as_deref(),
            Some("codex")
        );
        assert_eq!(infer_tool("vim"), None);
    }

    #[test]
    fn bootstrap_busy_ignores_shells() {
        assert!(!bootstrap_busy("zsh", None));
        assert!(!bootstrap_busy(" fish ", None));
        assert!(bootstrap_busy("codex", Some("codex")));
        assert!(bootstrap_busy("cargo", None));
    }

    #[test]
    fn tracker_preserves_last_activity_when_observation_is_unchanged() {
        let now = Utc::now();
        let mut tracker = TmuxScanTracker::new();

        let first = tracker.apply_observation(
            now,
            SessionObservation {
                session_id: "tmux:work:1.0:%1".to_string(),
                tool: Some("codex".to_string()),
                cwd: "/tmp/project".to_string(),
                replay_text: "Need approval to continue".to_string(),
                current_command: "codex".to_string(),
            },
        );
        assert_eq!(first.state, SessionState::Busy);
        assert_eq!(first.last_activity_at, now);

        let later = now + chrono::Duration::seconds(45);
        let second = tracker.apply_observation(
            later,
            SessionObservation {
                session_id: "tmux:work:1.0:%1".to_string(),
                tool: Some("codex".to_string()),
                cwd: "/tmp/project".to_string(),
                replay_text: "Need approval to continue".to_string(),
                current_command: "codex".to_string(),
            },
        );
        assert_eq!(second.state, SessionState::Idle);
        assert_eq!(second.last_activity_at, now);
    }

    #[test]
    fn tracker_keeps_non_agent_foreground_command_busy_when_observation_is_unchanged() {
        let now = Utc::now();
        let mut tracker = TmuxScanTracker::new();

        let first = tracker.apply_observation(
            now,
            SessionObservation {
                session_id: "tmux:work:1.0:%1".to_string(),
                tool: None,
                cwd: "/tmp/project".to_string(),
                replay_text: String::new(),
                current_command: "cargo".to_string(),
            },
        );
        assert_eq!(first.state, SessionState::Busy);

        let second = tracker.apply_observation(
            now + chrono::Duration::seconds(45),
            SessionObservation {
                session_id: "tmux:work:1.0:%1".to_string(),
                tool: None,
                cwd: "/tmp/project".to_string(),
                replay_text: String::new(),
                current_command: "cargo".to_string(),
            },
        );
        assert_eq!(second.state, SessionState::Busy);
    }

    #[test]
    fn tracker_refreshes_last_activity_when_replay_text_changes() {
        let now = Utc::now();
        let mut tracker = TmuxScanTracker::new();

        let _ = tracker.apply_observation(
            now,
            SessionObservation {
                session_id: "tmux:work:1.0:%1".to_string(),
                tool: Some("codex".to_string()),
                cwd: "/tmp/project".to_string(),
                replay_text: "Thinking".to_string(),
                current_command: "codex".to_string(),
            },
        );

        let later = now + chrono::Duration::seconds(45);
        let changed = tracker.apply_observation(
            later,
            SessionObservation {
                session_id: "tmux:work:1.0:%1".to_string(),
                tool: Some("codex".to_string()),
                cwd: "/tmp/project".to_string(),
                replay_text: "Need approval to continue".to_string(),
                current_command: "codex".to_string(),
            },
        );
        assert_eq!(changed.state, SessionState::Busy);
        assert_eq!(changed.last_activity_at, later);
    }
}
