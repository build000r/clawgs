//! Model backend clients (OpenRouter, Grok) for live thought generation.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

const OPENROUTER_TIMEOUT: Duration = Duration::from_secs(15);
const OPENROUTER_CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const COMMAND_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const GROK_CLI_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
const DEFAULT_GROK_CLI_MODEL: &str = "";
const DEFAULT_GROK_MAX_TURNS: u32 = 20;
const MODEL_BACKEND_ENV: &str = "CLAWGS_MODEL_BACKEND";
const GROK_BIN_ENV: &str = "CLAWGS_GROK_BIN";
const GROK_WORKDIR_ENV: &str = "CLAWGS_GROK_WORKDIR";
const GROK_MAX_TURNS_ENV: &str = "CLAWGS_GROK_MAX_TURNS";
const GROK_RUNTIME_DIR: &str = "clawgs-grok-headless";
const MODEL_ENV_KEYS: [&str; 3] = [
    "SWIMMERS_THOUGHT_MODEL",
    "SWIMMERS_THOUGHT_MODEL_2",
    "SWIMMERS_THOUGHT_MODEL_3",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelBackend {
    OpenRouter,
    GrokCli,
}

impl ModelBackend {
    pub fn from_env_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some(Self::OpenRouter),
            "grok" | "grok_cli" | "grok-cli" => Some(Self::GrokCli),
            "codex" | "codex_cli" | "codex-cli" => Some(Self::GrokCli),
            "claude" | "claude_cli" | "claude-cli" => Some(Self::GrokCli),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::GrokCli => "grok",
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
        ModelBackend::GrokCli => Ok(Box::new(GrokCliModelClient::new())),
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
        ModelBackend::GrokCli => {
            if !command_available(GROK_BIN_ENV, "grok") {
                return Err(format!("{}: grok binary not found", backend.as_str()));
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

pub struct GrokCliModelClient {
    bin: String,
    runtime_dir: PathBuf,
    workdir: PathBuf,
    max_turns: String,
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

impl GrokCliModelClient {
    pub fn new() -> Self {
        Self {
            bin: configured_bin(GROK_BIN_ENV, "grok"),
            runtime_dir: std::env::temp_dir().join(GROK_RUNTIME_DIR),
            workdir: nonempty_env_var(GROK_WORKDIR_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            max_turns: configured_positive_u32(GROK_MAX_TURNS_ENV, DEFAULT_GROK_MAX_TURNS)
                .to_string(),
        }
    }
}

impl Default for GrokCliModelClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelClient for GrokCliModelClient {
    fn complete(&self, prompt: &str, model_override: Option<&str>) -> Result<String, String> {
        complete_with_models(
            &candidate_models(model_override, ModelBackend::GrokCli),
            |model| self.complete_once(prompt, model),
        )
    }
}

impl GrokCliModelClient {
    fn complete_once(&self, prompt: &str, model: &str) -> Result<String, String> {
        ensure_private_runtime_dir(&self.runtime_dir)?;

        let stamp = unique_stamp();
        let prompt_path = self.runtime_dir.join(format!("{stamp}.prompt.txt"));
        let stdout_path = self.runtime_dir.join(format!("{stamp}.stdout.log"));
        let stderr_path = self.runtime_dir.join(format!("{stamp}.stderr.log"));
        let _cleanup = TempFileGuard::new(&[
            prompt_path.as_path(),
            stdout_path.as_path(),
            stderr_path.as_path(),
        ]);

        write_private_file(&prompt_path, prompt.as_bytes())
            .map_err(|error| format!("failed to write {}: {error}", prompt_path.display()))?;

        let output = run_subprocess_capturing(SubprocessSpec {
            bin: &self.bin,
            args: build_grok_headless_args(model, &prompt_path, &self.workdir, &self.max_turns),
            stdin_payload: &[],
            stdout_path: &stdout_path,
            stderr_path: &stderr_path,
            timeout: GROK_CLI_TIMEOUT,
            label: "grok headless",
        })?;

        if !output.success {
            return Err(format!(
                "grok headless failed: {}",
                failure_preview(&output.stderr, &output.stdout)
            ));
        }

        let trimmed = output.stdout.trim();
        if trimmed.is_empty() {
            return Err("grok headless returned empty output".to_string());
        }
        Ok(trimmed.to_string())
    }
}

fn auto_detect_model_backend() -> ModelBackend {
    if nonempty_env_var("OPENROUTER_API_KEY").is_some() {
        ModelBackend::OpenRouter
    } else if command_available(GROK_BIN_ENV, "grok") {
        ModelBackend::GrokCli
    } else {
        ModelBackend::OpenRouter
    }
}

fn command_available(env_key: &str, default: &str) -> bool {
    command_available_with_timeout(env_key, default, COMMAND_PROBE_TIMEOUT)
}

fn command_available_with_timeout(env_key: &str, default: &str, timeout: Duration) -> bool {
    let mut child = match Command::new(configured_bin(env_key, default))
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    wait_with_timeout(&mut child, timeout, "model backend version probe")
        .map(|status| status.success())
        .unwrap_or(false)
}

fn configured_bin(env_key: &str, default: &str) -> String {
    nonempty_env_var(env_key).unwrap_or_else(|| default.to_string())
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
        ModelBackend::GrokCli => Some(DEFAULT_GROK_CLI_MODEL),
    }
}

fn configured_positive_u32(env_key: &str, default: u32) -> u32 {
    nonempty_env_var(env_key)
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn build_grok_headless_args(
    model: &str,
    prompt_path: &Path,
    workdir: &Path,
    max_turns: &str,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("--prompt-file"),
        prompt_path.as_os_str().to_os_string(),
        OsString::from("--output-format"),
        OsString::from("plain"),
        OsString::from("--no-alt-screen"),
        OsString::from("--max-turns"),
        OsString::from(max_turns),
        OsString::from("--cwd"),
        workdir.as_os_str().to_os_string(),
    ];
    if !model.trim().is_empty() {
        args.push(OsString::from("-m"));
        args.push(OsString::from(model));
    }
    args
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

#[derive(Debug)]
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

#[cfg(unix)]
fn ensure_private_runtime_dir(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing symlink runtime dir {}", path.display()));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "runtime path is not a directory: {}",
            path.display()
        ));
    }

    let mut permissions = metadata.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("failed to chmod {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn ensure_private_runtime_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))
}

fn create_private_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut file = create_private_file(path)?;
    file.write_all(contents)
        .map_err(|error| format!("failed to write file: {error}"))
}

fn run_subprocess_capturing(spec: SubprocessSpec<'_>) -> Result<SubprocessOutput, String> {
    let stdout_file = create_private_file(spec.stdout_path)?;
    let stderr_file = create_private_file(spec.stderr_path)?;

    let mut command = Command::new(spec.bin);
    command.args(&spec.args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::from(stdout_file));
    command.stderr(Stdio::from(stderr_file));

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn {}: {error}", spec.label))?;

    let Some(mut stdin) = child.stdin.take() else {
        terminate_child(&mut child);
        return Err(format!("{} missing stdin pipe", spec.label));
    };
    if let Err(error) = stdin.write_all(spec.stdin_payload) {
        drop(stdin);
        terminate_child(&mut child);
        return Err(format!("failed to write {} prompt: {error}", spec.label));
    }
    drop(stdin);

    let status = wait_with_timeout(&mut child, spec.timeout, spec.label)?;
    let stdout = fs::read_to_string(spec.stdout_path).unwrap_or_default();
    let stderr = fs::read_to_string(spec.stderr_path).unwrap_or_default();
    Ok(SubprocessOutput {
        stdout,
        stderr,
        success: status.success(),
    })
}

fn terminate_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
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
        build_grok_headless_args, build_openrouter_request_body, command_available_with_timeout,
        default_model_for_backend, extract_openrouter_content, failure_preview,
        interpret_openrouter_response, pick_nonempty_or_fallback, run_subprocess_capturing,
        thought_models, validate_backend_credentials, GrokCliModelClient, ModelBackend,
        ModelClient, OpenRouterModelClient, SubprocessSpec,
    };

    #[test]
    fn thought_models_prefers_override() {
        let models = thought_models(Some("custom/model"), ModelBackend::GrokCli);
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
    fn grok_backend_uses_cli_default_model_when_unset() {
        let _lock = lock_env();
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_2");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_3");

        let model = default_model_for_backend(ModelBackend::GrokCli);

        assert!(model.is_empty());
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
    fn build_grok_headless_args_uses_prompt_file_and_optional_model() {
        let args = build_grok_headless_args(
            "grok-4",
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/project"),
            "20",
        );
        let args: Vec<String> = args
            .into_iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();

        assert!(args.contains(&"--prompt-file".to_string()));
        assert!(args.contains(&"/tmp/prompt.txt".to_string()));
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"plain".to_string()));
        assert!(
            !args.contains(&"--always-approve".to_string()),
            "status summarization must not auto-approve tool-capable CLI actions"
        );
        assert!(args.contains(&"--max-turns".to_string()));
        assert!(args.contains(&"20".to_string()));
        assert!(args.contains(&"--cwd".to_string()));
        assert!(args.contains(&"/tmp/project".to_string()));
        assert!(args.contains(&"-m".to_string()));
        assert!(args.contains(&"grok-4".to_string()));

        let args_without_model = build_grok_headless_args(
            "",
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/project"),
            "20",
        );
        let args_without_model: Vec<String> = args_without_model
            .into_iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert!(!args_without_model.contains(&"-m".to_string()));
    }

    #[test]
    fn model_backend_from_env_value_maps_cli_aliases_to_grok() {
        assert_eq!(
            ModelBackend::from_env_value("claude"),
            Some(ModelBackend::GrokCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("claude_cli"),
            Some(ModelBackend::GrokCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("claude-cli"),
            Some(ModelBackend::GrokCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("CLAUDE"),
            Some(ModelBackend::GrokCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("codex"),
            Some(ModelBackend::GrokCli)
        );
        assert_eq!(
            ModelBackend::from_env_value("grok_cli"),
            Some(ModelBackend::GrokCli)
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
        assert_eq!(ModelBackend::GrokCli.as_str(), "grok");
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
    fn validate_backend_credentials_rejects_missing_grok_binary() {
        let _lock = lock_env();
        std::env::set_var("CLAWGS_GROK_BIN", "/nonexistent/clawgs-grok-zzz");
        let err = validate_backend_credentials(ModelBackend::GrokCli)
            .expect_err("must fail when grok bin missing");
        assert!(err.starts_with("grok:"));
        assert!(err.contains("not found"));
        std::env::remove_var("CLAWGS_GROK_BIN");
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

    fn write_fake_backend(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-backend");
        fs::write(&script, body).expect("write fake backend");
        let mut permissions = fs::metadata(&script)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("chmod");
        script
    }

    fn process_exists(pid: &str) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn subprocess_write_failure_terminates_child() {
        use std::fs;
        use std::time::Duration;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let script = write_fake_backend(
            dir.path(),
            concat!(
                "#!/bin/sh\n",
                "printf '%s' \"$$\" > \"$(dirname \"$0\")/child.pid\"\n",
                "exec 0<&-\n",
                "sleep 30\n"
            ),
        );
        let stdout_path = dir.path().join("stdout.log");
        let stderr_path = dir.path().join("stderr.log");
        let prompt = vec![b'x'; 1024 * 1024];
        let script_bin = script.to_string_lossy().into_owned();

        let error = run_subprocess_capturing(SubprocessSpec {
            bin: &script_bin,
            args: Vec::new(),
            stdin_payload: &prompt,
            stdout_path: &stdout_path,
            stderr_path: &stderr_path,
            timeout: Duration::from_secs(5),
            label: "fake backend",
        })
        .expect_err("closed stdin should fail while writing prompt");

        assert!(error.contains("failed to write fake backend prompt"));
        let pid = fs::read_to_string(dir.path().join("child.pid")).expect("child pid");
        assert!(
            !process_exists(pid.trim()),
            "child must be killed and waited after prompt write failure"
        );
    }

    #[cfg(unix)]
    #[test]
    fn grok_runtime_paths_are_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        use tempfile::tempdir;

        fn mode(path: &std::path::Path) -> u32 {
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        }

        let dir = tempdir().expect("tempdir");
        let runtime_dir = dir.path().join("runtime");
        super::ensure_private_runtime_dir(&runtime_dir).expect("runtime dir");
        assert_eq!(mode(&runtime_dir), 0o700, "runtime dir must be private");

        let prompt_path = runtime_dir.join("prompt.txt");
        super::write_private_file(&prompt_path, b"status prompt").expect("prompt write");
        assert_eq!(mode(&prompt_path), 0o600, "prompt file must be private");

        let script = write_fake_backend(
            dir.path(),
            concat!(
                "#!/bin/sh\n",
                "printf 'backend stdout\\n'\n",
                "printf 'backend stderr\\n' >&2\n",
            ),
        );
        let stdout_path = runtime_dir.join("stdout.log");
        let stderr_path = runtime_dir.join("stderr.log");
        let script_bin = script.to_string_lossy().into_owned();

        let output = run_subprocess_capturing(SubprocessSpec {
            bin: &script_bin,
            args: Vec::new(),
            stdin_payload: b"",
            stdout_path: &stdout_path,
            stderr_path: &stderr_path,
            timeout: Duration::from_secs(5),
            label: "fake backend",
        })
        .expect("subprocess run");

        assert!(output.success);
        assert_eq!(output.stdout, "backend stdout\n");
        assert_eq!(output.stderr, "backend stderr\n");
        assert_eq!(mode(&stdout_path), 0o600, "stdout file must be private");
        assert_eq!(mode(&stderr_path), 0o600, "stderr file must be private");
    }

    #[test]
    fn command_available_probe_times_out() {
        use std::time::{Duration, Instant};
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let script = write_fake_backend(dir.path(), concat!("#!/bin/sh\n", "sleep 30\n"));
        let script_bin = script.to_string_lossy().into_owned();

        let started = Instant::now();
        let available = command_available_with_timeout(
            "CLAWGS_TEST_GROK_BIN_UNUSED",
            &script_bin,
            Duration::from_millis(100),
        );

        assert!(!available);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "availability probe should not hang on a wedged backend"
        );
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

    fn auto_detect_with_isolated_env(api_key: Option<&str>, grok_bin: &str) -> ModelBackend {
        let _lock = lock_env();
        let priors: [(&str, Option<String>); 2] = [
            (
                "OPENROUTER_API_KEY",
                std::env::var("OPENROUTER_API_KEY").ok(),
            ),
            ("CLAWGS_GROK_BIN", std::env::var("CLAWGS_GROK_BIN").ok()),
        ];
        override_env("OPENROUTER_API_KEY", api_key);
        std::env::set_var("CLAWGS_GROK_BIN", grok_bin);
        let backend = super::auto_detect_model_backend();
        for (key, prior) in &priors {
            override_env(key, prior.as_deref());
        }
        backend
    }

    #[test]
    fn auto_detect_prefers_openrouter_when_api_key_set() {
        let backend = auto_detect_with_isolated_env(Some("any-test-key"), ALWAYS_OK_BIN);
        assert_eq!(backend, ModelBackend::OpenRouter);
    }

    #[test]
    fn auto_detect_chooses_grok_when_grok_is_runnable() {
        let backend = auto_detect_with_isolated_env(None, ALWAYS_OK_BIN);
        assert_eq!(backend, ModelBackend::GrokCli);
    }

    #[test]
    fn auto_detect_falls_back_to_openrouter_when_nothing_available() {
        let backend = auto_detect_with_isolated_env(None, "/nonexistent/grok-zzz");
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
    fn validate_backend_credentials_accepts_runnable_grok_binary() {
        let _lock = lock_env();
        let prior = std::env::var("CLAWGS_GROK_BIN").ok();
        std::env::set_var("CLAWGS_GROK_BIN", ALWAYS_OK_BIN);
        validate_backend_credentials(ModelBackend::GrokCli)
            .expect("runnable grok bin should validate");
        match prior {
            Some(value) => std::env::set_var("CLAWGS_GROK_BIN", value),
            None => std::env::remove_var("CLAWGS_GROK_BIN"),
        }
    }

    #[test]
    fn grok_cli_client_new_uses_defaults_when_env_unset() {
        let _lock = lock_env();
        std::env::remove_var("CLAWGS_GROK_BIN");
        std::env::remove_var("CLAWGS_GROK_WORKDIR");
        std::env::remove_var("CLAWGS_GROK_MAX_TURNS");
        let client = GrokCliModelClient::new();
        assert_eq!(client.bin, "grok");
        assert_eq!(client.max_turns, "20");
        assert!(client.runtime_dir.ends_with("clawgs-grok-headless"));
    }

    #[test]
    fn grok_client_complete_reads_prompt_file_and_returns_trimmed_stdout() {
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        let script = write_fake_backend(
            dir.path(),
            concat!(
                "#!/bin/sh\n",
                "prompt=\"\"\n",
                "model=\"\"\n",
                "while [ \"$#\" -gt 0 ]; do\n",
                "  case \"$1\" in\n",
                "    --prompt-file) prompt=\"$2\"; shift 2;;\n",
                "    -m) model=\"$2\"; shift 2;;\n",
                "    *) shift;;\n",
                "  esac\n",
                "done\n",
                "test -f \"$prompt\" || exit 3\n",
                "stat -c '%a' \"$(dirname \"$prompt\")\" > \"$(dirname \"$0\")/runtime.mode\"\n",
                "stat -c '%a' \"$prompt\" > \"$(dirname \"$0\")/prompt.mode\"\n",
                "grep -q 'status prompt' \"$prompt\" || exit 4\n",
                "test \"$model\" = 'grok-test-model' || exit 5\n",
                "printf '   grok ok   \\n'\n",
            ),
        );
        let client = GrokCliModelClient {
            bin: script.to_string_lossy().into_owned(),
            runtime_dir: dir.path().join("runtime"),
            workdir: dir.path().to_path_buf(),
            max_turns: "7".to_string(),
        };

        let out = client
            .complete("status prompt", Some("grok-test-model"))
            .expect("complete should succeed");

        assert_eq!(out, "grok ok");
        #[cfg(unix)]
        {
            assert_eq!(
                std::fs::read_to_string(dir.path().join("runtime.mode"))
                    .expect("runtime mode")
                    .trim(),
                "700"
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join("prompt.mode"))
                    .expect("prompt mode")
                    .trim(),
                "600"
            );
        }
    }

    #[test]
    fn grok_client_complete_tries_configured_model_fallbacks() {
        let _lock = lock_env();
        use tempfile::tempdir;

        std::env::set_var("SWIMMERS_THOUGHT_MODEL", "bad-model");
        std::env::set_var("SWIMMERS_THOUGHT_MODEL_2", "good-model");
        std::env::remove_var("SWIMMERS_THOUGHT_MODEL_3");

        let dir = tempdir().expect("tempdir");
        let script = write_fake_backend(
            dir.path(),
            concat!(
                "#!/bin/sh\n",
                "model=''\n",
                "while [ \"$#\" -gt 0 ]; do\n",
                "  case \"$1\" in\n",
                "    -m) model=\"$2\"; shift 2;;\n",
                "    *) shift;;\n",
                "  esac\n",
                "done\n",
                "if [ \"$model\" = 'bad-model' ]; then\n",
                "  echo 'bad model failed' >&2\n",
                "  exit 7\n",
                "fi\n",
                "test \"$model\" = 'good-model' || exit 8\n",
                "printf 'fallback ok\\n'\n",
            ),
        );
        let client = GrokCliModelClient {
            bin: script.to_string_lossy().into_owned(),
            runtime_dir: dir.path().join("runtime"),
            workdir: dir.path().to_path_buf(),
            max_turns: "7".to_string(),
        };

        let out = client
            .complete("status prompt", None)
            .expect("fallback should succeed");

        assert_eq!(out, "fallback ok");
        clear_swimmers_env();
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
