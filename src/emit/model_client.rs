use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const OPENROUTER_TIMEOUT: Duration = Duration::from_secs(15);
const OPENROUTER_CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const CODEX_EXEC_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_CLI_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
const DEFAULT_CODEX_CLI_MODEL: &str = "gpt-5.1-codex-mini";
const DEFAULT_CODEX_CLI_REASONING: &str = "low";
const DEFAULT_CODEX_CLI_VERBOSITY: &str = "low";
const DEFAULT_CLAUDE_CLI_MODEL: &str = "haiku";
const DEFAULT_CLAUDE_CLI_MAX_BUDGET: &str = "0.02";
const MODEL_BACKEND_ENV: &str = "CLAWGS_MODEL_BACKEND";
const CODEX_BIN_ENV: &str = "CLAWGS_CODEX_BIN";
const CODEX_REASONING_ENV: &str = "CLAWGS_CODEX_REASONING_EFFORT";
const CODEX_VERBOSITY_ENV: &str = "CLAWGS_CODEX_VERBOSITY";
const CODEX_WORKDIR_ENV: &str = "CLAWGS_CODEX_WORKDIR";
const CODEX_RUNTIME_DIR: &str = "clawgs-codex-exec";
const CLAUDE_BIN_ENV: &str = "CLAWGS_CLAUDE_BIN";
const CLAUDE_MAX_BUDGET_ENV: &str = "CLAWGS_CLAUDE_MAX_BUDGET";
const CLAUDE_RUNTIME_DIR: &str = "clawgs-claude-exec";
const MODEL_ENV_KEYS: [&str; 3] = [
    "SWIMMERS_THOUGHT_MODEL",
    "SWIMMERS_THOUGHT_MODEL_2",
    "SWIMMERS_THOUGHT_MODEL_3",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelBackend {
    OpenRouter,
    CodexCli,
    ClaudeCli,
}

impl ModelBackend {
    pub fn from_env_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some(Self::OpenRouter),
            "codex" | "codex_cli" | "codex-cli" => Some(Self::CodexCli),
            "claude" | "claude_cli" | "claude-cli" => Some(Self::ClaudeCli),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::CodexCli => "codex",
            Self::ClaudeCli => "claude",
        }
    }
}

pub trait ModelClient: Send + Sync {
    fn complete(&self, prompt: &str, model_override: Option<&str>) -> Result<String, String>;
}

pub fn build_model_client() -> Result<Box<dyn ModelClient>, String> {
    build_model_client_for(resolve_model_backend())
}

pub fn build_model_client_for(backend: ModelBackend) -> Result<Box<dyn ModelClient>, String> {
    match backend {
        ModelBackend::OpenRouter => {
            OpenRouterModelClient::new().map(|client| Box::new(client) as Box<dyn ModelClient>)
        }
        ModelBackend::CodexCli => Ok(Box::new(CodexCliModelClient::new())),
        ModelBackend::ClaudeCli => Ok(Box::new(ClaudeCliModelClient::new())),
    }
}

pub fn validate_backend_credentials(backend: ModelBackend) -> Result<(), String> {
    match backend {
        ModelBackend::OpenRouter => {
            if nonempty_env_var("OPENROUTER_API_KEY").is_none() {
                return Err(format!("{}: OPENROUTER_API_KEY not set", backend.as_str()));
            }
            Ok(())
        }
        ModelBackend::CodexCli => {
            if !codex_command_available() {
                return Err(format!("{}: codex binary not found", backend.as_str()));
            }
            Ok(())
        }
        ModelBackend::ClaudeCli => {
            if !claude_command_available() {
                return Err(format!("{}: claude binary not found", backend.as_str()));
            }
            Ok(())
        }
    }
}

pub fn resolve_model_backend() -> ModelBackend {
    nonempty_env_var(MODEL_BACKEND_ENV)
        .and_then(|value| ModelBackend::from_env_value(&value))
        .unwrap_or_else(auto_detect_model_backend)
}

pub fn default_model_for_backend(backend: ModelBackend) -> String {
    thought_models(None, backend)
        .into_iter()
        .next()
        .unwrap_or_default()
}

pub fn thought_models(model_override: Option<&str>, backend: ModelBackend) -> Vec<String> {
    candidate_models(model_override, backend)
}

pub struct OpenRouterModelClient {
    client: reqwest::blocking::Client,
    chat_url: String,
}

impl OpenRouterModelClient {
    pub fn new() -> Result<Self, String> {
        Self::with_chat_url(OPENROUTER_CHAT_URL.to_string())
    }

    pub(crate) fn with_chat_url(chat_url: String) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(OPENROUTER_TIMEOUT)
            .build()
            .map_err(|error| format!("failed to build HTTP client: {error}"))?;
        Ok(Self { client, chat_url })
    }
}

impl ModelClient for OpenRouterModelClient {
    fn complete(&self, prompt: &str, model_override: Option<&str>) -> Result<String, String> {
        let api_key = std::env::var("OPENROUTER_API_KEY") // ubs:ignore - environment variable name, not a secret value
            .map_err(|_| "OPENROUTER_API_KEY not set".to_string())?;
        complete_with_models(
            &candidate_models(model_override, ModelBackend::OpenRouter),
            |model| {
                nonempty_openrouter_response(&self.client, &self.chat_url, prompt, model, &api_key)
            },
        )
    }
}

pub struct CodexCliModelClient {
    bin: String,
    runtime_dir: PathBuf,
    workdir: PathBuf,
    reasoning_effort: String,
    verbosity: String,
}

struct TempFileGuard {
    paths: Vec<PathBuf>,
}

impl TempFileGuard {
    fn new(paths: &[&Path]) -> Self {
        Self {
            paths: paths.iter().map(|p| p.to_path_buf()).collect(),
        }
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// Codex `model_reasoning_effort` accepts a fixed set of values; any other
/// value would be rejected by the upstream `codex exec` invocation later, so
/// we fall back to the default at startup with a warning instead of failing
/// the daemon mid-loop.
const REASONING_EFFORT_ALLOWED: &[&str] = &["minimal", "low", "medium", "high"];
/// Codex `model_verbosity` allowlist (same rationale as reasoning_effort).
const VERBOSITY_ALLOWED: &[&str] = &["low", "medium", "high"];

fn validated_codex_setting(env_key: &str, default: &str, allowed: &[&str]) -> String {
    match nonempty_env_var(env_key) {
        Some(value) if allowed.iter().any(|candidate| *candidate == value) => value,
        Some(value) => {
            eprintln!(
                "clawgs: ignoring {env_key}={value:?}; expected one of {allowed:?}. \
                 Falling back to {default:?}."
            );
            default.to_string()
        }
        None => default.to_string(),
    }
}

impl CodexCliModelClient {
    pub fn new() -> Self {
        Self {
            bin: codex_bin(),
            runtime_dir: std::env::temp_dir().join(CODEX_RUNTIME_DIR),
            workdir: nonempty_env_var(CODEX_WORKDIR_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            reasoning_effort: validated_codex_setting(
                CODEX_REASONING_ENV,
                DEFAULT_CODEX_CLI_REASONING,
                REASONING_EFFORT_ALLOWED,
            ),
            verbosity: validated_codex_setting(
                CODEX_VERBOSITY_ENV,
                DEFAULT_CODEX_CLI_VERBOSITY,
                VERBOSITY_ALLOWED,
            ),
        }
    }
}

impl Default for CodexCliModelClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelClient for CodexCliModelClient {
    fn complete(&self, prompt: &str, model_override: Option<&str>) -> Result<String, String> {
        let model = candidate_models(model_override, ModelBackend::CodexCli)
            .into_iter()
            .next()
            .ok_or_else(|| "no models configured".to_string())?;

        fs::create_dir_all(&self.runtime_dir)
            .map_err(|error| format!("failed to create {}: {error}", self.runtime_dir.display()))?;

        let stamp = unique_stamp();
        let output_path = self.runtime_dir.join(format!("{stamp}.last.txt"));
        let stdout_path = self.runtime_dir.join(format!("{stamp}.stdout.log"));
        let stderr_path = self.runtime_dir.join(format!("{stamp}.stderr.log"));

        // RAII guard: each invocation creates three files in `runtime_dir`.
        // Without explicit cleanup they would accumulate forever in a
        // long-lived `tmux-emit` daemon. Drop runs on every exit path,
        // including early `?` returns, panics, and timeouts.
        let _cleanup = TempFileGuard::new(&[
            output_path.as_path(),
            stdout_path.as_path(),
            stderr_path.as_path(),
        ]);

        let output = run_subprocess_capturing(SubprocessSpec {
            bin: &self.bin,
            args: build_codex_exec_args(
                &model,
                &output_path,
                &self.workdir,
                &self.reasoning_effort,
                &self.verbosity,
            ),
            stdin_payload: prompt.as_bytes(),
            stdout_path: &stdout_path,
            stderr_path: &stderr_path,
            timeout: CODEX_EXEC_TIMEOUT,
            label: "codex exec",
        })?;

        if !output.success {
            return Err(format!(
                "codex exec failed: {}",
                failure_preview(&output.stderr, &output.stdout)
            ));
        }

        let content = fs::read_to_string(&output_path).map_err(|error| {
            format!(
                "codex exec succeeded but {} was missing: {error}",
                output_path.display()
            )
        })?;
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err("codex exec returned an empty final message".to_string());
        }
        Ok(trimmed.to_string())
    }
}

pub struct ClaudeCliModelClient {
    bin: String,
    runtime_dir: PathBuf,
    max_budget: String,
}

impl ClaudeCliModelClient {
    pub fn new() -> Self {
        Self {
            bin: claude_bin(),
            runtime_dir: std::env::temp_dir().join(CLAUDE_RUNTIME_DIR),
            max_budget: nonempty_env_var(CLAUDE_MAX_BUDGET_ENV)
                .unwrap_or_else(|| DEFAULT_CLAUDE_CLI_MAX_BUDGET.to_string()),
        }
    }
}

impl Default for ClaudeCliModelClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelClient for ClaudeCliModelClient {
    fn complete(&self, prompt: &str, model_override: Option<&str>) -> Result<String, String> {
        let model = model_override
            .map(|m| m.to_string())
            .or_else(|| {
                candidate_models(None, ModelBackend::ClaudeCli)
                    .into_iter()
                    .next()
            })
            .unwrap_or_else(|| DEFAULT_CLAUDE_CLI_MODEL.to_string());

        fs::create_dir_all(&self.runtime_dir)
            .map_err(|error| format!("failed to create {}: {error}", self.runtime_dir.display()))?;

        let stamp = unique_stamp();
        let stdout_path = self.runtime_dir.join(format!("{stamp}.stdout.log"));
        let stderr_path = self.runtime_dir.join(format!("{stamp}.stderr.log"));
        let _cleanup = TempFileGuard::new(&[stdout_path.as_path(), stderr_path.as_path()]);

        let args = [
            "--print",
            "--model",
            &model,
            "--bare",
            "--output-format",
            "text",
            "--no-session-persistence",
            "--max-budget-usd",
            &self.max_budget,
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();

        let output = run_subprocess_capturing(SubprocessSpec {
            bin: &self.bin,
            args,
            stdin_payload: prompt.as_bytes(),
            stdout_path: &stdout_path,
            stderr_path: &stderr_path,
            timeout: CLAUDE_CLI_TIMEOUT,
            label: "claude exec",
        })?;

        if !output.success {
            return Err(format!(
                "claude exec failed: {}",
                failure_preview(&output.stderr, &output.stdout)
            ));
        }

        let trimmed = output.stdout.trim();
        if trimmed.is_empty() {
            return Err("claude exec returned empty output".to_string());
        }
        Ok(trimmed.to_string())
    }
}

fn auto_detect_model_backend() -> ModelBackend {
    if nonempty_env_var("OPENROUTER_API_KEY").is_some() {
        ModelBackend::OpenRouter
    } else if claude_command_available() {
        ModelBackend::ClaudeCli
    } else if codex_command_available() {
        ModelBackend::CodexCli
    } else {
        ModelBackend::OpenRouter
    }
}

fn codex_command_available() -> bool {
    Command::new(codex_bin())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn claude_command_available() -> bool {
    Command::new(claude_bin())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn codex_bin() -> String {
    nonempty_env_var(CODEX_BIN_ENV).unwrap_or_else(|| "codex".to_string())
}

fn claude_bin() -> String {
    nonempty_env_var(CLAUDE_BIN_ENV).unwrap_or_else(|| "claude".to_string())
}

fn candidate_models(model_override: Option<&str>, backend: ModelBackend) -> Vec<String> {
    model_override
        .map(|model| vec![model.to_string()])
        .unwrap_or_else(|| configured_models(backend))
}

fn configured_models(backend: ModelBackend) -> Vec<String> {
    let configured: Vec<String> = MODEL_ENV_KEYS
        .iter()
        .filter_map(|key| nonempty_env_var(key))
        .collect();
    if !configured.is_empty() {
        configured
    } else {
        backend_default_model(backend)
            .map(|model| vec![model.to_string()])
            .unwrap_or_default()
    }
}

fn backend_default_model(backend: ModelBackend) -> Option<&'static str> {
    match backend {
        ModelBackend::OpenRouter => Some(DEFAULT_OPENROUTER_MODEL),
        ModelBackend::CodexCli => Some(DEFAULT_CODEX_CLI_MODEL),
        ModelBackend::ClaudeCli => Some(DEFAULT_CLAUDE_CLI_MODEL),
    }
}

fn build_codex_exec_args(
    model: &str,
    output_path: &Path,
    workdir: &Path,
    reasoning_effort: &str,
    verbosity: &str,
) -> Vec<OsString> {
    vec![
        OsString::from("exec"),
        OsString::from("-m"),
        OsString::from(model),
        OsString::from("-C"),
        workdir.as_os_str().to_os_string(),
        OsString::from("--skip-git-repo-check"),
        OsString::from("--ephemeral"),
        OsString::from("--output-last-message"),
        output_path.as_os_str().to_os_string(),
        OsString::from("-c"),
        OsString::from(format!("model_reasoning_effort={reasoning_effort:?}")),
        OsString::from("-c"),
        OsString::from(format!("model_verbosity={verbosity:?}")),
        OsString::from("-c"),
        OsString::from("model_reasoning_summary=\"none\""),
        OsString::from("-"),
    ]
}

struct SubprocessSpec<'a> {
    bin: &'a str,
    args: Vec<OsString>,
    stdin_payload: &'a [u8],
    stdout_path: &'a Path,
    stderr_path: &'a Path,
    timeout: Duration,
    label: &'a str,
}

struct SubprocessOutput {
    stdout: String,
    stderr: String,
    success: bool,
}

fn unique_stamp() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn run_subprocess_capturing(spec: SubprocessSpec<'_>) -> Result<SubprocessOutput, String> {
    let stdout_file = File::create(spec.stdout_path)
        .map_err(|error| format!("failed to create {}: {error}", spec.stdout_path.display()))?;
    let stderr_file = File::create(spec.stderr_path)
        .map_err(|error| format!("failed to create {}: {error}", spec.stderr_path.display()))?;

    let mut command = Command::new(spec.bin);
    command.args(&spec.args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::from(stdout_file));
    command.stderr(Stdio::from(stderr_file));

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn {}: {error}", spec.label))?;

    {
        let Some(stdin) = child.stdin.as_mut() else {
            return Err(format!("{} missing stdin pipe", spec.label));
        };
        stdin
            .write_all(spec.stdin_payload)
            .map_err(|error| format!("failed to write {} prompt: {error}", spec.label))?;
    }
    drop(child.stdin.take());

    let status = wait_with_timeout(&mut child, spec.timeout, spec.label)?;
    let stdout = fs::read_to_string(spec.stdout_path).unwrap_or_default();
    let stderr = fs::read_to_string(spec.stderr_path).unwrap_or_default();
    Ok(SubprocessOutput {
        stdout,
        stderr,
        success: status.success(),
    })
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
    label: &str,
) -> Result<std::process::ExitStatus, String> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} timed out after {}s", timeout.as_secs()));
            }
            Err(error) => return Err(format!("failed to wait for {label}: {error}")),
        }
    }
}

fn failure_preview(stderr: &str, stdout: &str) -> String {
    let merged = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let trimmed = merged.trim();
    if trimmed.is_empty() {
        "process exited without output".to_string()
    } else {
        trimmed.chars().take(500).collect()
    }
}

fn call_openrouter(
    client: &reqwest::blocking::Client,
    url: &str,
    prompt: &str,
    model: &str,
    api_key: &str,
) -> Result<Option<String>, String> {
    call_openrouter_with_reasoning_mode(client, url, prompt, model, api_key, false)
}

fn build_openrouter_request_body(
    prompt: &str,
    model: &str,
    suppress_reasoning: bool,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": 80,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    });
    if suppress_reasoning {
        body["reasoning"] = serde_json::json!({
            "effort": "none",
            "exclude": true
        });
    }
    body
}

fn call_openrouter_with_reasoning_mode(
    client: &reqwest::blocking::Client,
    url: &str,
    prompt: &str,
    model: &str,
    api_key: &str,
    suppress_reasoning: bool,
) -> Result<Option<String>, String> {
    let body = build_openrouter_request_body(prompt, model, suppress_reasoning);
    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let status = response.status();
    let body_text = response.text().unwrap_or_default();
    interpret_openrouter_response(status, body_text)
}

fn interpret_openrouter_response(
    status: reqwest::StatusCode,
    body_text: String,
) -> Result<Option<String>, String> {
    if !status.is_success() {
        let preview: String = body_text.chars().take(500).collect();
        return Err(format!("{status}: {preview}"));
    }
    let body: serde_json::Value =
        serde_json::from_str(&body_text).map_err(|error| format!("json parse failed: {error}"))?;
    Ok(extract_openrouter_content(&body))
}

fn extract_openrouter_content(body: &serde_json::Value) -> Option<String> {
    let content = &body["choices"][0]["message"]["content"];
    if let Some(text) = content.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| text.to_string());
    }

    let parts = content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn nonempty_env_var(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn complete_with_models<F>(models: &[String], mut attempt: F) -> Result<String, String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    let mut last_error = "no models configured".to_string();
    models
        .iter()
        .find_map(|model| match attempt(model) {
            Ok(content) => Some(Ok(content)),
            Err(error) => {
                last_error = format!("{model}: {error}");
                None
            }
        })
        .map_or(
            Err(format!("all models failed, last: {last_error}")),
            |result| result,
        )
}

fn pick_nonempty_or_fallback<F>(primary: Option<String>, fallback: F) -> Result<String, String>
where
    F: FnOnce() -> Option<String>,
{
    primary
        .or_else(fallback)
        .ok_or_else(|| "returned empty".to_string())
}

fn nonempty_openrouter_response(
    client: &reqwest::blocking::Client,
    url: &str,
    prompt: &str,
    model: &str,
    api_key: &str,
) -> Result<String, String> {
    let primary = call_openrouter(client, url, prompt, model, api_key)?;
    pick_nonempty_or_fallback(primary, || {
        call_openrouter_with_reasoning_mode(client, url, prompt, model, api_key, true)
            .ok()
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::MutexGuard;

    fn lock_env() -> MutexGuard<'static, ()> {
        crate::test_support::process_env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    use super::{
        build_codex_exec_args, build_openrouter_request_body, default_model_for_backend,
        extract_openrouter_content, failure_preview, interpret_openrouter_response,
        pick_nonempty_or_fallback, thought_models, validate_backend_credentials,
        validated_codex_setting, ClaudeCliModelClient, CodexCliModelClient, ModelBackend,
        ModelClient, OpenRouterModelClient, REASONING_EFFORT_ALLOWED, VERBOSITY_ALLOWED,
    };

    #[test]
    fn thought_models_prefers_override() {
        let models = thought_models(Some("custom/model"), ModelBackend::CodexCli);
        assert_eq!(models, vec!["custom/model".to_string()]);
    }

    #[test]
    fn thought_models_collects_nonempty_env_overrides_in_order() {
        let _lock = lock_env();
        std::env::set_var("SWIMMERS_THOUGHT_MODEL", "openrouter/one");
        std::env::set_var("SWIMMERS_THOUGHT_MODEL_2", "   ");
        std::env::set_var("SWIMMERS_THOUGHT_MODEL_3", "openrouter/three");

        let models = thought_models(None, ModelBackend::OpenRouter);

        assert_eq!(
            models,
            vec!["openrouter/one".to_string(), "openrouter/three".to_string()]
        );

        std::env::remove_var("SWIMMERS_THOUGHT_MODEL");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_2");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_3");
    }

    #[test]
    fn validated_codex_setting_accepts_allowlisted_values_and_rejects_others() {
        let _lock = lock_env();
        std::env::set_var("CLAWGS_TEST_REASONING", "high");
        assert_eq!(
            validated_codex_setting("CLAWGS_TEST_REASONING", "low", REASONING_EFFORT_ALLOWED),
            "high"
        );

        // Empty / whitespace-only values fall through to default.
        std::env::set_var("CLAWGS_TEST_REASONING", "   ");
        assert_eq!(
            validated_codex_setting("CLAWGS_TEST_REASONING", "low", REASONING_EFFORT_ALLOWED),
            "low"
        );

        // Bogus values fall back to default (with a stderr warning).
        std::env::set_var("CLAWGS_TEST_REASONING", "evil\"; rm -rf /");
        assert_eq!(
            validated_codex_setting("CLAWGS_TEST_REASONING", "low", REASONING_EFFORT_ALLOWED),
            "low"
        );

        // Verbosity allowlist excludes "minimal".
        std::env::set_var("CLAWGS_TEST_VERBOSITY", "minimal");
        assert_eq!(
            validated_codex_setting("CLAWGS_TEST_VERBOSITY", "low", VERBOSITY_ALLOWED),
            "low"
        );

        std::env::remove_var("CLAWGS_TEST_REASONING");
        std::env::remove_var("CLAWGS_TEST_VERBOSITY");
    }

    #[test]
    fn codex_backend_falls_back_to_headless_codex_default_model() {
        let _lock = lock_env();
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_2");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_3");

        let model = default_model_for_backend(ModelBackend::CodexCli);

        assert_eq!(model, "gpt-5.1-codex-mini");
    }

    #[test]
    fn openrouter_backend_falls_back_to_router_default_model() {
        let _lock = lock_env();
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_2");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_3");

        let model = default_model_for_backend(ModelBackend::OpenRouter);

        assert_eq!(model, "openrouter/free");
    }

    #[test]
    fn complete_with_models_returns_first_successful_result() {
        let models = vec!["first".to_string(), "second".to_string()];
        let result = super::complete_with_models(&models, |model| {
            if model == "first" {
                Err("boom".to_string())
            } else {
                Ok("done".to_string())
            }
        });

        assert_eq!(result.expect("successful fallback"), "done");
    }

    #[test]
    fn complete_with_models_reports_last_error() {
        let models = vec!["alpha".to_string(), "beta".to_string()];
        let error = super::complete_with_models(&models, |model| Err(format!("{model} failed")))
            .expect_err("expected failure");

        assert!(error.contains("beta: beta failed"));
    }

    #[test]
    fn build_codex_exec_args_pins_low_reasoning_and_low_verbosity() {
        let args = build_codex_exec_args(
            "gpt-5.1-codex-mini",
            Path::new("/tmp/last.txt"),
            Path::new("/tmp"),
            "low",
            "low",
        );
        let args: Vec<String> = args
            .into_iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();

        assert!(args.contains(&"exec".to_string()));
        assert!(args.contains(&"gpt-5.1-codex-mini".to_string()));
        assert!(args.contains(&"model_reasoning_effort=\"low\"".to_string()));
        assert!(args.contains(&"model_verbosity=\"low\"".to_string()));
        assert!(args.contains(&"model_reasoning_summary=\"none\"".to_string()));
        assert!(args.contains(&"--ephemeral".to_string()));
    }

    #[test]
    fn claude_backend_falls_back_to_haiku_default_model() {
        let _lock = lock_env();
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_2");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_3");

        let model = default_model_for_backend(ModelBackend::ClaudeCli);

        assert_eq!(model, "haiku");
    }

    #[test]
    fn model_backend_from_env_value_parses_claude_variants() {
        assert_eq!(
            ModelBackend::from_env_value("claude"),
            Some(ModelBackend::ClaudeCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("claude_cli"),
            Some(ModelBackend::ClaudeCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("claude-cli"),
            Some(ModelBackend::ClaudeCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("CLAUDE"),
            Some(ModelBackend::ClaudeCli)
        );
    }

    #[test]
    fn model_backend_from_env_value_rejects_unknown() {
        assert_eq!(ModelBackend::from_env_value("gemini"), None);
        assert_eq!(ModelBackend::from_env_value(""), None);
    }

    #[test]
    fn model_backend_as_str_roundtrips() {
        assert_eq!(ModelBackend::OpenRouter.as_str(), "openrouter");
        assert_eq!(ModelBackend::CodexCli.as_str(), "codex");
        assert_eq!(ModelBackend::ClaudeCli.as_str(), "claude");
    }

    #[test]
    fn validate_backend_credentials_rejects_missing_openrouter_key() {
        let _lock = lock_env();
        std::env::remove_var("OPENROUTER_API_KEY");

        let err = validate_backend_credentials(ModelBackend::OpenRouter)
            .expect_err("should fail without API key");
        assert!(err.starts_with("openrouter:"));
        assert!(err.contains("OPENROUTER_API_KEY"));
    }

    #[test]
    fn claude_client_handles_large_stdout_without_pipe_deadlock() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-claude");
        fs::write(
            &script_path,
            concat!(
                "#!/bin/sh\n",
                "cat >/dev/null\n",
                "i=0\n",
                "while [ \"$i\" -lt 2000 ]; do\n",
                "  printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n'\n",
                "  i=$((i + 1))\n",
                "done\n"
            ),
        )
        .expect("write fake claude");
        let mut permissions = fs::metadata(&script_path)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("chmod");

        let client = ClaudeCliModelClient {
            bin: script_path.to_string_lossy().into_owned(),
            runtime_dir: dir.path().join("runtime"),
            max_budget: "0.01".to_string(),
        };

        let output = client
            .complete("status prompt", Some("fake-model"))
            .expect("large stdout should complete");

        assert!(
            output.len() > 100_000,
            "test fixture must exceed a typical pipe buffer"
        );
    }

    #[test]
    fn temp_file_guard_removes_files_on_drop_and_tolerates_missing() {
        use std::fs;
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        let present = dir.path().join("present.tmp");
        let missing = dir.path().join("never-created.tmp");
        fs::write(&present, b"data").expect("write fixture");
        assert!(present.exists());

        {
            let _guard = super::TempFileGuard::new(&[present.as_path(), missing.as_path()]);
        }

        assert!(!present.exists(), "guard must delete the temp file on drop");
        // Dropping with a missing path must not panic — long-lived daemons
        // depend on this so a partially-created exec leaves no residue.
        assert!(!missing.exists());
    }

    #[test]
    fn extract_openrouter_content_reads_string_message() {
        let body = serde_json::json!({
            "choices": [{"message": {"content": "  hello world  "}}]
        });
        assert_eq!(
            extract_openrouter_content(&body),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn extract_openrouter_content_joins_array_parts() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": [
                        {"type": "text", "text": " first "},
                        {"type": "text", "text": "second"},
                        {"type": "text", "text": "   "}
                    ]
                }
            }]
        });
        assert_eq!(
            extract_openrouter_content(&body),
            Some("first second".to_string())
        );
    }

    #[test]
    fn extract_openrouter_content_returns_none_when_blank() {
        let blank_string = serde_json::json!({
            "choices": [{"message": {"content": "   "}}]
        });
        assert_eq!(extract_openrouter_content(&blank_string), None);

        let empty_array = serde_json::json!({
            "choices": [{"message": {"content": []}}]
        });
        assert_eq!(extract_openrouter_content(&empty_array), None);

        let missing = serde_json::json!({"choices": []});
        assert_eq!(extract_openrouter_content(&missing), None);
    }

    #[test]
    fn failure_preview_prefers_stderr_when_present() {
        let preview = failure_preview("real error\n", "ignored stdout");
        assert_eq!(preview, "real error");
    }

    #[test]
    fn failure_preview_falls_back_to_stdout_when_stderr_blank() {
        let preview = failure_preview("   \n", "stdout content");
        assert_eq!(preview, "stdout content");
    }

    #[test]
    fn failure_preview_reports_when_both_blank() {
        let preview = failure_preview("", "");
        assert_eq!(preview, "process exited without output");
    }

    #[test]
    fn failure_preview_truncates_long_output_to_500_chars() {
        let long_stderr = "x".repeat(2_000);
        let preview = failure_preview(&long_stderr, "");
        assert_eq!(preview.chars().count(), 500);
    }

    #[test]
    fn validate_backend_credentials_rejects_missing_codex_binary() {
        let _lock = lock_env();
        std::env::set_var("CLAWGS_CODEX_BIN", "/nonexistent/clawgs-codex-zzz");
        let err = validate_backend_credentials(ModelBackend::CodexCli)
            .expect_err("must fail when codex bin missing");
        assert!(err.starts_with("codex:"));
        assert!(err.contains("not found"));
        std::env::remove_var("CLAWGS_CODEX_BIN");
    }

    #[test]
    fn validate_backend_credentials_rejects_missing_claude_binary() {
        let _lock = lock_env();
        std::env::set_var("CLAWGS_CLAUDE_BIN", "/nonexistent/clawgs-claude-zzz");
        let err = validate_backend_credentials(ModelBackend::ClaudeCli)
            .expect_err("must fail when claude bin missing");
        assert!(err.starts_with("claude:"));
        assert!(err.contains("not found"));
        std::env::remove_var("CLAWGS_CLAUDE_BIN");
    }

    fn write_fake_codex(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-codex");
        fs::write(&script, body).expect("write fake codex");
        let mut permissions = fs::metadata(&script)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("chmod");
        script
    }

    fn build_codex_test_client(
        bin: std::path::PathBuf,
        runtime_dir: std::path::PathBuf,
        workdir: std::path::PathBuf,
    ) -> CodexCliModelClient {
        CodexCliModelClient {
            bin: bin.to_string_lossy().into_owned(),
            runtime_dir,
            workdir,
            reasoning_effort: "low".to_string(),
            verbosity: "low".to_string(),
        }
    }

    #[test]
    fn codex_client_complete_returns_trimmed_last_message() {
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        // Fake codex finds --output-last-message in argv, writes a payload there,
        // then exits 0. Real codex contract: write the model's last message to
        // the path passed via --output-last-message.
        let script = write_fake_codex(
            dir.path(),
            concat!(
                "#!/bin/sh\n",
                "cat >/dev/null\n",
                "out=\"\"\n",
                "while [ \"$#\" -gt 0 ]; do\n",
                "  if [ \"$1\" = \"--output-last-message\" ]; then\n",
                "    out=\"$2\"; shift 2; continue\n",
                "  fi\n",
                "  shift\n",
                "done\n",
                "printf '   hello world   \\n' > \"$out\"\n",
            ),
        );
        let client =
            build_codex_test_client(script, dir.path().join("runtime"), dir.path().to_path_buf());
        let out = client
            .complete("ignored", Some("fake-model"))
            .expect("complete should succeed");
        assert_eq!(out, "hello world");
    }

    #[test]
    fn codex_client_complete_surfaces_subprocess_failure() {
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        let script = write_fake_codex(
            dir.path(),
            concat!(
                "#!/bin/sh\n",
                "cat >/dev/null\n",
                "echo 'codex blew up' >&2\n",
                "exit 7\n",
            ),
        );
        let client =
            build_codex_test_client(script, dir.path().join("runtime"), dir.path().to_path_buf());
        let err = client
            .complete("prompt", Some("fake-model"))
            .expect_err("should fail when codex exits nonzero");
        assert!(err.contains("codex exec failed"));
        assert!(err.contains("codex blew up"));
    }

    #[test]
    fn codex_client_complete_errors_on_empty_last_message() {
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        let script = write_fake_codex(
            dir.path(),
            concat!(
                "#!/bin/sh\n",
                "cat >/dev/null\n",
                "out=\"\"\n",
                "while [ \"$#\" -gt 0 ]; do\n",
                "  if [ \"$1\" = \"--output-last-message\" ]; then\n",
                "    out=\"$2\"; shift 2; continue\n",
                "  fi\n",
                "  shift\n",
                "done\n",
                "printf '   \\n' > \"$out\"\n",
            ),
        );
        let client =
            build_codex_test_client(script, dir.path().join("runtime"), dir.path().to_path_buf());
        let err = client
            .complete("prompt", Some("fake-model"))
            .expect_err("blank message must fail");
        assert!(err.contains("empty final message"));
    }

    /// `/usr/bin/true` exits 0 regardless of args, so it stands in for any
    /// "command is available" probe (`<bin> --version` returning success).
    const ALWAYS_OK_BIN: &str = "/usr/bin/true";

    /// Apply `value` to env var `key`, restoring nothing — the caller does
    /// that. `Some("")` is treated like any other value; only `None` removes.
    fn override_env(key: &str, value: Option<&str>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    fn auto_detect_with_isolated_env(
        api_key: Option<&str>,
        claude_bin: &str,
        codex_bin: &str,
    ) -> ModelBackend {
        let _lock = lock_env();
        let priors: [(&str, Option<String>); 3] = [
            (
                "OPENROUTER_API_KEY",
                std::env::var("OPENROUTER_API_KEY").ok(),
            ),
            ("CLAWGS_CLAUDE_BIN", std::env::var("CLAWGS_CLAUDE_BIN").ok()),
            ("CLAWGS_CODEX_BIN", std::env::var("CLAWGS_CODEX_BIN").ok()),
        ];
        override_env("OPENROUTER_API_KEY", api_key);
        std::env::set_var("CLAWGS_CLAUDE_BIN", claude_bin);
        std::env::set_var("CLAWGS_CODEX_BIN", codex_bin);
        let backend = super::auto_detect_model_backend();
        for (key, prior) in &priors {
            override_env(key, prior.as_deref());
        }
        backend
    }

    #[test]
    fn auto_detect_prefers_openrouter_when_api_key_set() {
        let backend = auto_detect_with_isolated_env(
            Some("any-test-key"),
            "/nonexistent/claude-zzz",
            "/nonexistent/codex-zzz",
        );
        assert_eq!(backend, ModelBackend::OpenRouter);
    }

    #[test]
    fn auto_detect_chooses_claude_when_only_claude_runnable() {
        let backend = auto_detect_with_isolated_env(None, ALWAYS_OK_BIN, "/nonexistent/codex-zzz");
        assert_eq!(backend, ModelBackend::ClaudeCli);
    }

    #[test]
    fn auto_detect_chooses_codex_when_only_codex_runnable() {
        let backend = auto_detect_with_isolated_env(None, "/nonexistent/claude-zzz", ALWAYS_OK_BIN);
        assert_eq!(backend, ModelBackend::CodexCli);
    }

    #[test]
    fn auto_detect_falls_back_to_openrouter_when_nothing_available() {
        let backend = auto_detect_with_isolated_env(
            None,
            "/nonexistent/claude-zzz",
            "/nonexistent/codex-zzz",
        );
        assert_eq!(backend, ModelBackend::OpenRouter);
    }

    #[test]
    fn validate_backend_credentials_accepts_present_openrouter_key() {
        let _lock = lock_env();
        let prior = std::env::var("OPENROUTER_API_KEY").ok();
        std::env::set_var("OPENROUTER_API_KEY", "sk-test-not-real");
        validate_backend_credentials(ModelBackend::OpenRouter)
            .expect("present API key should validate");
        match prior {
            Some(value) => std::env::set_var("OPENROUTER_API_KEY", value),
            None => std::env::remove_var("OPENROUTER_API_KEY"),
        }
    }

    #[test]
    fn validate_backend_credentials_accepts_runnable_claude_binary() {
        let _lock = lock_env();
        let prior = std::env::var("CLAWGS_CLAUDE_BIN").ok();
        std::env::set_var("CLAWGS_CLAUDE_BIN", ALWAYS_OK_BIN);
        validate_backend_credentials(ModelBackend::ClaudeCli)
            .expect("runnable claude bin should validate");
        match prior {
            Some(value) => std::env::set_var("CLAWGS_CLAUDE_BIN", value),
            None => std::env::remove_var("CLAWGS_CLAUDE_BIN"),
        }
    }

    #[test]
    fn validate_backend_credentials_accepts_runnable_codex_binary() {
        let _lock = lock_env();
        let prior = std::env::var("CLAWGS_CODEX_BIN").ok();
        std::env::set_var("CLAWGS_CODEX_BIN", ALWAYS_OK_BIN);
        validate_backend_credentials(ModelBackend::CodexCli)
            .expect("runnable codex bin should validate");
        match prior {
            Some(value) => std::env::set_var("CLAWGS_CODEX_BIN", value),
            None => std::env::remove_var("CLAWGS_CODEX_BIN"),
        }
    }

    #[test]
    fn codex_cli_client_new_uses_defaults_when_env_unset() {
        let _lock = lock_env();
        std::env::remove_var("CLAWGS_CODEX_BIN");
        std::env::remove_var("CLAWGS_CODEX_REASONING_EFFORT");
        std::env::remove_var("CLAWGS_CODEX_VERBOSITY");
        std::env::remove_var("CLAWGS_CODEX_WORKDIR");
        let client = CodexCliModelClient::new();
        assert_eq!(client.bin, "codex");
        assert_eq!(client.reasoning_effort, "low");
        assert_eq!(client.verbosity, "low");
        assert!(client.runtime_dir.ends_with("clawgs-codex-exec"));
    }

    #[test]
    fn claude_cli_client_new_uses_defaults_when_env_unset() {
        let _lock = lock_env();
        std::env::remove_var("CLAWGS_CLAUDE_BIN");
        std::env::remove_var("CLAWGS_CLAUDE_MAX_BUDGET");
        let client = ClaudeCliModelClient::new();
        assert_eq!(client.bin, "claude");
        assert_eq!(client.max_budget, "0.02");
        assert!(client.runtime_dir.ends_with("clawgs-claude-exec"));
    }

    #[test]
    fn openrouter_request_body_omits_reasoning_block_by_default() {
        let body = build_openrouter_request_body("hello", "openrouter/free", false);
        assert_eq!(body["model"], "openrouter/free");
        assert_eq!(body["max_tokens"], 80);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hello");
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn openrouter_request_body_suppresses_reasoning_when_requested() {
        let body = build_openrouter_request_body("hi", "x/y", true);
        assert_eq!(body["reasoning"]["effort"], "none");
        assert_eq!(body["reasoning"]["exclude"], true);
    }

    #[test]
    fn pick_nonempty_or_fallback_returns_primary_when_present() {
        let result =
            pick_nonempty_or_fallback(Some("primary".to_string()), || panic!("must not run"));
        assert_eq!(result.expect("ok"), "primary");
    }

    #[test]
    fn pick_nonempty_or_fallback_uses_fallback_when_primary_blank() {
        let result = pick_nonempty_or_fallback(None, || Some("fallback".to_string()));
        assert_eq!(result.expect("ok"), "fallback");
    }

    #[test]
    fn pick_nonempty_or_fallback_errors_when_both_empty() {
        let err = pick_nonempty_or_fallback(None, || None).expect_err("must error");
        assert_eq!(err, "returned empty");
    }

    #[test]
    fn interpret_openrouter_response_returns_content_on_2xx_with_string_body() {
        let body = r#"{"choices":[{"message":{"content":"hi there"}}]}"#;
        let result =
            interpret_openrouter_response(reqwest::StatusCode::OK, body.to_string()).unwrap();
        assert_eq!(result, Some("hi there".to_string()));
    }

    #[test]
    fn interpret_openrouter_response_returns_none_when_content_is_blank() {
        let body = r#"{"choices":[{"message":{"content":"   "}}]}"#;
        let result =
            interpret_openrouter_response(reqwest::StatusCode::OK, body.to_string()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn interpret_openrouter_response_surfaces_status_with_body_preview_on_error() {
        let err = interpret_openrouter_response(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "rate limited".to_string(),
        )
        .expect_err("non-2xx must error");
        assert!(err.contains("429"));
        assert!(err.contains("rate limited"));
    }

    #[test]
    fn interpret_openrouter_response_truncates_huge_error_body_to_500_chars() {
        let huge = "a".repeat(2_000);
        let err = interpret_openrouter_response(reqwest::StatusCode::BAD_GATEWAY, huge)
            .expect_err("non-2xx must error");
        let preview_only: String = err
            .split_once(':')
            .map(|(_, rest)| rest)
            .unwrap_or("")
            .trim()
            .chars()
            .collect();
        assert_eq!(preview_only.chars().count(), 500);
    }

    #[test]
    fn interpret_openrouter_response_errors_when_2xx_body_is_not_json() {
        let err =
            interpret_openrouter_response(reqwest::StatusCode::OK, "definitely not json".into())
                .expect_err("malformed JSON must error");
        assert!(err.starts_with("json parse failed:"));
    }

    /// Builds an HTTP/1.1 response framed with `Connection: close` so the
    /// caller doesn't need to compute Content-Length for the canned body.
    fn http_close_response(status_line: &str, body: &str) -> String {
        format!(
            "{status_line}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}"
        )
    }

    fn spawn_canned_responses(responses: Vec<String>) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("addr").port();

        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        });

        format!("http://127.0.0.1:{port}/")
    }

    fn clear_swimmers_env() {
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_2");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_3");
    }

    #[test]
    fn openrouter_client_complete_returns_content_from_canned_http_response() {
        let _lock = lock_env();
        std::env::set_var("OPENROUTER_API_KEY", "test-key-not-real");
        clear_swimmers_env();

        let url = spawn_canned_responses(vec![http_close_response(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"content":"  remote ok  "}}]}"#,
        )]);
        let client = OpenRouterModelClient::with_chat_url(url).expect("client");
        let answer = client.complete("hello", None).expect("ok");
        assert_eq!(answer, "remote ok");

        std::env::remove_var("OPENROUTER_API_KEY");
    }

    #[test]
    fn openrouter_client_complete_falls_back_to_suppress_reasoning_when_first_response_blank() {
        let _lock = lock_env();
        std::env::set_var("OPENROUTER_API_KEY", "test-key-not-real");
        clear_swimmers_env();

        let url = spawn_canned_responses(vec![
            http_close_response(
                "HTTP/1.1 200 OK",
                r#"{"choices":[{"message":{"content":"   "}}]}"#,
            ),
            http_close_response(
                "HTTP/1.1 200 OK",
                r#"{"choices":[{"message":{"content":"with reasoning"}}]}"#,
            ),
        ]);
        let client = OpenRouterModelClient::with_chat_url(url).expect("client");
        let answer = client.complete("hi", None).expect("fallback ok");
        assert_eq!(answer, "with reasoning");

        std::env::remove_var("OPENROUTER_API_KEY");
    }

    #[test]
    fn openrouter_client_complete_surfaces_status_error_from_remote() {
        let _lock = lock_env();
        std::env::set_var("OPENROUTER_API_KEY", "test-key-not-real");
        clear_swimmers_env();

        // Both attempts return 500. The client must error out with all-models
        // surfaced via complete_with_models.
        let url = spawn_canned_responses(vec![
            http_close_response("HTTP/1.1 500 Internal Server Error", "boom-503"),
            http_close_response("HTTP/1.1 500 Internal Server Error", "boom-503"),
        ]);
        let client = OpenRouterModelClient::with_chat_url(url).expect("client");
        let err = client
            .complete("hi", None)
            .expect_err("must propagate failure");
        assert!(err.contains("all models failed"));
        assert!(err.contains("500"));

        std::env::remove_var("OPENROUTER_API_KEY");
    }

    #[test]
    fn openrouter_complete_errors_when_api_key_missing() {
        let _lock = lock_env();
        std::env::remove_var("OPENROUTER_API_KEY");
        let client = OpenRouterModelClient::new().expect("build client");
        let err = client
            .complete("hello", None)
            .expect_err("must fail without key");
        assert!(err.contains("OPENROUTER_API_KEY"));
    }
}
