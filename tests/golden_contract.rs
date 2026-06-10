use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn run_clawgs(args: &[&str]) -> Value {
    let home = TempDir::new().expect("temp home");
    let output = Command::new(env!("CARGO_BIN_EXE_clawgs"))
        .args(args)
        .env("HOME", home.path())
        .output()
        .expect("run clawgs");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut value: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    normalize_contract(&mut value);
    value
}

fn expected(name: &str) -> Value {
    let raw = match name {
        "demo_emit" => include_str!("goldens/demo_emit.normalized.json"),
        "demo_extract_codex" => include_str!("goldens/demo_extract_codex.normalized.json"),
        "demo_extract_claude" => include_str!("goldens/demo_extract_claude.normalized.json"),
        "extract_codex_fixture" => include_str!("goldens/extract_codex_fixture.normalized.json"),
        "extract_claude_fixture" => include_str!("goldens/extract_claude_fixture.normalized.json"),
        "extract_codex_current_fixture" => {
            include_str!("goldens/extract_codex_current_fixture.normalized.json")
        }
        _ => panic!("unknown golden {name}"),
    };
    serde_json::from_str(raw).expect("golden json")
}

fn normalize_contract(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("generated_at") {
                map.insert(
                    "generated_at".to_string(),
                    Value::String("<generated_at>".to_string()),
                );
            }
            for child in map.values_mut() {
                normalize_contract(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_contract(item);
            }
        }
        _ => {}
    }
}

#[test]
fn demo_emit_matches_golden_contract() {
    assert_eq!(
        run_clawgs(&["demo", "emit", "--pretty"]),
        expected("demo_emit")
    );
}

#[test]
fn demo_extract_codex_matches_golden_contract() {
    assert_eq!(
        run_clawgs(&["demo", "extract", "--tool", "codex", "--pretty"]),
        expected("demo_extract_codex")
    );
}

#[test]
fn demo_extract_claude_matches_golden_contract() {
    assert_eq!(
        run_clawgs(&["demo", "extract", "--tool", "claude", "--pretty"]),
        expected("demo_extract_claude")
    );
}

#[test]
fn fixture_extract_codex_matches_golden_contract() {
    assert_eq!(
        run_clawgs(&[
            "extract",
            "--tool",
            "codex",
            "--cwd",
            "/schema-sync/project",
            "--input",
            "tests/fixtures/codex-sample.jsonl",
            "--pretty",
        ]),
        expected("extract_codex_fixture")
    );
}

#[test]
fn fixture_extract_claude_matches_golden_contract() {
    assert_eq!(
        run_clawgs(&[
            "extract",
            "--tool",
            "claude",
            "--cwd",
            "/schema-sync/project",
            "--input",
            "tests/fixtures/claude-sample.jsonl",
            "--pretty",
        ]),
        expected("extract_claude_fixture")
    );
}

#[test]
fn fixture_extract_codex_current_matches_golden_contract() {
    assert_eq!(
        run_clawgs(&[
            "extract",
            "--tool",
            "codex",
            "--cwd",
            "/tmp/project",
            "--input",
            "tests/fixtures/codex-current.jsonl",
            "--pretty",
        ]),
        expected("extract_codex_current_fixture")
    );
}
