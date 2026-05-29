pub mod claude;
pub mod codex;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::DateTime;
use serde_json::Value;

use crate::{Action, CommitSignal, ExtractOptions};

pub(crate) struct ParseSnapshot {
    pub user_task: Option<String>,
    pub recent_actions: Vec<Action>,
    pub current_tool: Option<Action>,
    pub token_count: u64,
    pub awaiting_user_input: bool,
    pub awaiting_user_text: Option<String>,
    pub commit_signal: Option<CommitSignal>,
    pub events_seen: u64,
    pub malformed_lines_skipped: u64,
    pub bytes_read: u64,
    pub raw_events: Option<Vec<Value>>,
}

pub(crate) struct ParsedLines {
    pub entries: Vec<Value>,
    pub malformed_lines_skipped: u64,
    pub bytes_read: u64,
    pub raw_events: Option<Vec<Value>>,
}

const RAW_EVENT_RING_CAP: usize = 20;

pub(crate) fn read_jsonl(path: &Path, include_raw: bool) -> Result<ParsedLines> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let bytes_read = bytes.len() as u64;

    let mut acc = JsonlAccumulator::new(include_raw);
    for line in String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        acc.ingest(line);
    }

    Ok(acc.into_parsed_lines(bytes_read))
}

struct JsonlAccumulator {
    entries: Vec<Value>,
    malformed_lines_skipped: u64,
    raw_events: Vec<Value>,
    include_raw: bool,
}

impl JsonlAccumulator {
    fn new(include_raw: bool) -> Self {
        Self {
            entries: Vec::new(),
            malformed_lines_skipped: 0,
            raw_events: Vec::new(),
            include_raw,
        }
    }

    fn ingest(&mut self, line: &str) {
        match serde_json::from_str::<Value>(line) {
            Ok(value) => {
                if self.include_raw {
                    push_raw_event(&mut self.raw_events, value.clone());
                }
                self.entries.push(value);
            }
            Err(_) => self.malformed_lines_skipped += 1,
        }
    }

    fn into_parsed_lines(self, bytes_read: u64) -> ParsedLines {
        let raw_events = self.include_raw.then_some(self.raw_events);
        ParsedLines {
            entries: self.entries,
            malformed_lines_skipped: self.malformed_lines_skipped,
            bytes_read,
            raw_events,
        }
    }
}

/// Append `value` and drop the oldest entries past the ring cap. Always drains
/// (no-op when at or below cap) so the caller doesn't need a separate length
/// branch.
fn push_raw_event(raw_events: &mut Vec<Value>, value: Value) {
    raw_events.push(value);
    let drop_count = raw_events.len().saturating_sub(RAW_EVENT_RING_CAP);
    raw_events.drain(0..drop_count);
}

pub(crate) fn truncate(value: &str, max_chars: usize) -> String {
    // Single pass: walk char_indices up to max_chars + 1 to learn both whether
    // truncation is needed and where the cut byte-offset is. Avoids the
    // previous chars().count() + chars().take() double-walk on every call.
    let mut iter = value.char_indices();
    for _ in 0..max_chars {
        if iter.next().is_none() {
            return value.to_string();
        }
    }
    match iter.next() {
        None => value.to_string(),
        Some((cut, _)) => value[..cut].to_string(),
    }
}

pub(crate) fn push_action(actions: &mut Vec<Action>, action: Action, max_actions: usize) {
    actions.push(action);
    if actions.len() > max_actions {
        let to_remove = actions.len() - max_actions;
        actions.drain(0..to_remove);
    }
}

pub(crate) fn extract_tool_detail(input: &Value, options: &ExtractOptions) -> Option<String> {
    string_field(input, "file_path")
        .map(|file_path| basename(file_path).to_string())
        .or_else(|| {
            string_field(input, "command")
                .map(|command| truncate(command, options.max_detail_chars))
        })
        .or_else(|| {
            string_field(input, "pattern")
                .map(|pattern| truncate(pattern, options.max_detail_chars))
        })
}

pub(crate) fn extract_timestamp(entry: &Value) -> Option<String> {
    timestamp_from_value(entry).or_else(|| entry.get("payload").and_then(timestamp_from_value))
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn timestamp_from_value(value: &Value) -> Option<String> {
    ["timestamp", "created_at", "time", "ts"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(date_time_string))
}

fn date_time_string(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    DateTime::parse_from_rfc3339(value)
        .is_ok()
        .then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn read_jsonl_skips_bad_lines() {
        let file = NamedTempFile::new().expect("temp file");
        fs::write(
            file.path(),
            "{\"type\":\"a\"}\nnot-json\n{\"type\":\"b\"}\n",
        )
        .expect("write");

        let parsed = read_jsonl(file.path(), false).expect("read jsonl");
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.malformed_lines_skipped, 1);
    }

    #[test]
    fn truncate_limits_chars() {
        assert_eq!(truncate("hello", 3), "hel");
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn extract_tool_detail_prefers_file_path_then_command_then_pattern() {
        let options = ExtractOptions::default();
        let file_input = serde_json::json!({"file_path": "/tmp/demo.txt", "command": "ignored"});
        let command_input = serde_json::json!({"command": "cargo test --all"});
        let pattern_input = serde_json::json!({"pattern": "extract timestamp"});

        assert_eq!(
            extract_tool_detail(&file_input, &options).as_deref(),
            Some("demo.txt")
        );
        assert_eq!(
            extract_tool_detail(&command_input, &options).as_deref(),
            Some("cargo test --all")
        );
        assert_eq!(
            extract_tool_detail(&pattern_input, &options).as_deref(),
            Some("extract timestamp")
        );
    }

    #[test]
    fn extract_timestamp_uses_entry_then_payload_date_time_strings() {
        let top_level = serde_json::json!({"timestamp": "2026-02-26T20:11:56Z"});
        let payload = serde_json::json!({"payload": {"created_at": "2026-02-26T20:12:56Z"}});

        assert_eq!(
            extract_timestamp(&top_level).as_deref(),
            Some("2026-02-26T20:11:56Z")
        );
        assert_eq!(
            extract_timestamp(&payload).as_deref(),
            Some("2026-02-26T20:12:56Z")
        );
    }

    #[test]
    fn extract_timestamp_rejects_non_date_time_values() {
        let numeric = serde_json::json!({"timestamp": 12345});
        let boolean = serde_json::json!({"payload": {"created_at": true}});
        let plain_text = serde_json::json!({"ts": "yesterday"});

        assert_eq!(extract_timestamp(&numeric), None);
        assert_eq!(extract_timestamp(&boolean), None);
        assert_eq!(extract_timestamp(&plain_text), None);
    }
}
