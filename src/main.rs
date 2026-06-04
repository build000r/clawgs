mod demo;

use std::io::{self, BufRead, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use clawgs::emit::engine::{EmitEngine, DEFAULT_AGENT_PREAMBLE, DEFAULT_TERMINAL_PREAMBLE};
use clawgs::emit::model_client::{
    build_model_client, default_model_for_backend, resolve_model_backend,
};
use clawgs::emit::protocol::{ErrorMessage, HelloMessage, SyncRequest, SyncResultMessage};
use clawgs::tmux::TmuxScanTracker;
use clawgs::{extract, resolve_input, AgentTool, ExtractOptions, ToolSelection};

const MAX_TMUX_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;
const TMUX_SOCKET_PROBE_ATTEMPTS: usize = 3;
const TMUX_SOCKET_PROBE_RETRY_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Parser)]
#[command(name = "clawgs")]
#[command(about = "Extract structured JSON snapshots from Claude/Codex JSONL transcripts")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Show built-in Claude/Codex transcripts and the snapshots they produce — no local logs or credentials needed.
    Demo(DemoArgs),
    /// Normalize a Claude or Codex JSONL transcript into a `clawgs.v2` JSON snapshot on stdout.
    Extract(ExtractArgs),
    /// Run the live emit protocol daemon (NDJSON over stdio); requires `--stdio`.
    Emit(EmitArgs),
    /// Scan tmux panes, infer per-pane sessions, and emit changed thoughts as NDJSON.
    TmuxEmit(TmuxEmitArgs),
    /// Send a one-shot rescan signal to a running `tmux-emit` daemon over its UDP-style unix socket.
    TmuxNotify(TmuxNotifyArgs),
    /// Read Claude Code hook JSON from stdin and wake a running `tmux-emit` daemon without blocking Claude.
    ClaudeHookNotify(ClaudeHookNotifyArgs),
    /// Print resolved daemon defaults as JSON.
    Defaults,
}

#[derive(Debug, Args)]
struct ExtractArgs {
    /// Pick the parser. `auto` picks the newest of Claude/Codex transcripts under the matching cwd.
    #[arg(long, value_enum, default_value_t = ToolArg::Auto)]
    tool: ToolArg,

    /// Project working directory used to discover the transcript file. Defaults to the process cwd.
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Explicit JSONL transcript path. Skips discovery; `--tool` may still be required if the format can't be inferred.
    #[arg(long)]
    input: Option<PathBuf>,

    /// Pretty-print the JSON output (default: single-line compact).
    #[arg(long)]
    pretty: bool,

    /// Maximum number of recent agent actions to retain in the snapshot.
    #[arg(long, default_value_t = 10)]
    max_actions: usize,

    /// Truncation budget (in characters) for the user task field.
    #[arg(long, default_value_t = 300)]
    max_task_chars: usize,

    /// Truncation budget (in characters) for per-action detail strings (commands, file paths, patterns).
    #[arg(long, default_value_t = 100)]
    max_detail_chars: usize,

    /// Include the last 20 raw transcript events under `raw_events` for debugging.
    #[arg(long)]
    include_raw: bool,
}

#[derive(Debug, Args)]
struct EmitArgs {
    /// Run as an NDJSON daemon: read sync requests from stdin, write hello/sync_result/error to stdout.
    #[arg(long)]
    stdio: bool,
}

#[derive(Debug, Args)]
struct DemoArgs {
    #[command(subcommand)]
    command: DemoCommands,
}

#[derive(Debug, Subcommand)]
enum DemoCommands {
    /// Show a built-in transcript corpus and the extracted snapshot it produces.
    Extract(DemoExtractArgs),
    /// Show a built-in emit protocol exchange without external credentials.
    Emit(DemoEmitArgs),
}

#[derive(Debug, Args)]
struct DemoExtractArgs {
    /// Which embedded fixture to replay: `claude` or `codex`.
    #[arg(long, value_enum, default_value_t = DemoToolArg::Codex)]
    tool: DemoToolArg,

    /// Pretty-print the JSON output (default: single-line compact).
    #[arg(long)]
    pretty: bool,

    /// Maximum number of recent agent actions to retain in the snapshot.
    #[arg(long, default_value_t = 10)]
    max_actions: usize,

    /// Truncation budget (in characters) for the user task field.
    #[arg(long, default_value_t = 300)]
    max_task_chars: usize,

    /// Truncation budget (in characters) for per-action detail strings.
    #[arg(long, default_value_t = 100)]
    max_detail_chars: usize,

    /// Include the last 20 raw transcript events under `raw_events`.
    #[arg(long)]
    include_raw: bool,
}

#[derive(Debug, Args)]
struct DemoEmitArgs {
    /// Pretty-print the demo NDJSON exchange (default: one JSON object per line).
    #[arg(long)]
    pretty: bool,
}

#[derive(Debug, Args)]
struct TmuxEmitArgs {
    /// Polling interval in milliseconds between full pane rescans (1 to 86400000).
    #[arg(long, default_value_t = 15_000)]
    interval_ms: u64,

    /// Maximum number of trailing terminal lines captured from each pane per scan.
    #[arg(long, default_value_t = 200)]
    max_capture_lines: usize,

    /// Run a single scan and exit instead of looping forever.
    #[arg(long)]
    once: bool,

    /// Override the model used for thought generation. Empty string defers to backend default.
    #[arg(long, default_value = "")]
    model: String,

    /// Inline JSON-encoded `ThoughtConfig` (see references/emit-protocol-v2.md). Omitting it uses defaults.
    #[arg(long)]
    config_json: Option<String>,

    /// Path to the unix datagram socket used by `tmux-notify` to wake this daemon.
    #[arg(long)]
    socket: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct TmuxNotifyArgs {
    /// Path to the running daemon's notify socket. Defaults to the same path the daemon picks.
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Free-form event tag included in the notify payload (for log-side correlation).
    #[arg(long, default_value = "tmux-event")]
    event: String,
}

#[derive(Debug, Args)]
struct ClaudeHookNotifyArgs {
    /// Path to the running daemon's notify socket. Defaults to the same path `tmux-emit` picks.
    #[arg(long)]
    socket: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeHookInput {
    hook_event_name: Option<String>,
    session_id: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ToolArg {
    Auto,
    Claude,
    Codex,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DemoToolArg {
    Claude,
    Codex,
}

fn main() {
    if let Err(error) = run() {
        if is_broken_pipe(&error) {
            return;
        }
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io_error| io_error.kind() == io::ErrorKind::BrokenPipe)
        {
            return true;
        }

        cause
            .downcast_ref::<serde_json::Error>()
            .and_then(serde_json::Error::io_error_kind)
            .is_some_and(|kind| kind == io::ErrorKind::BrokenPipe)
    })
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Demo(args) => run_demo(args),
        Commands::Extract(args) => run_extract(args),
        Commands::Emit(args) => run_emit(args),
        Commands::TmuxEmit(args) => run_tmux_emit(args),
        Commands::TmuxNotify(args) => run_tmux_notify(args),
        Commands::ClaudeHookNotify(args) => run_claude_hook_notify(args),
        Commands::Defaults => run_defaults(),
    }
}

fn run_extract(args: ExtractArgs) -> Result<()> {
    validate_extract_limits(args.max_actions, args.max_task_chars, args.max_detail_chars)?;

    let cwd = match args.cwd {
        Some(path) => path,
        None => std::env::current_dir().context("failed to resolve current directory")?,
    };

    let selection = match args.tool {
        ToolArg::Auto => ToolSelection::Auto,
        ToolArg::Claude => ToolSelection::Claude,
        ToolArg::Codex => ToolSelection::Codex,
    };

    let resolved = resolve_input(selection, &cwd, args.input.as_deref())?;

    let options = ExtractOptions {
        max_actions: args.max_actions,
        max_task_chars: args.max_task_chars,
        max_detail_chars: args.max_detail_chars,
        include_raw: args.include_raw,
    };

    let output = extract(
        resolved.tool,
        &resolved.path,
        &cwd,
        resolved.discovered,
        &options,
    )?;

    print_json(&output, args.pretty)?;

    Ok(())
}

fn run_demo(args: DemoArgs) -> Result<()> {
    match args.command {
        DemoCommands::Extract(args) => run_demo_extract(args),
        DemoCommands::Emit(args) => run_demo_emit(args),
    }
}

fn run_demo_extract(args: DemoExtractArgs) -> Result<()> {
    validate_extract_limits(args.max_actions, args.max_task_chars, args.max_detail_chars)?;

    let output = demo::build_extract_demo(
        demo_tool(args.tool),
        ExtractOptions {
            max_actions: args.max_actions,
            max_task_chars: args.max_task_chars,
            max_detail_chars: args.max_detail_chars,
            include_raw: args.include_raw,
        },
    )?;

    print_json(&output, args.pretty)
}

fn run_demo_emit(args: DemoEmitArgs) -> Result<()> {
    let output = demo::build_emit_demo();
    print_json(&output, args.pretty)
}

fn run_emit(args: EmitArgs) -> Result<()> {
    ensure_emit_stdio(&args)?;
    let model_client = build_model_client()
        .map_err(|error| anyhow::anyhow!("failed to initialize model client: {error}"))?;
    let mut engine = EmitEngine::new(model_client);

    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    write_json_line(&mut stdout, &HelloMessage::new())?;

    for line in stdin.lock().lines() {
        let line = line.context("failed to read stdin line")?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        handle_emit_line(&mut stdout, &mut engine, trimmed)?;
    }

    Ok(())
}

fn ensure_emit_stdio(args: &EmitArgs) -> Result<()> {
    if args.stdio {
        Ok(())
    } else {
        anyhow::bail!(
            "`emit` runs as a daemon and requires --stdio.\n  \
             Add --stdio to read sync requests from stdin and write hello/sync_result \
             NDJSON to stdout, or run `clawgs demo emit` to see the protocol exchange."
        );
    }
}

fn handle_emit_line<W: Write>(
    stdout: &mut W,
    engine: &mut EmitEngine,
    trimmed: &str,
) -> Result<()> {
    let response = sync_response_for_line(engine, trimmed)
        .map(EmitLineResult::Sync)
        .unwrap_or_else(EmitLineResult::Error);
    response.write(stdout)
}

/// Lightweight peek at the envelope before attempting the typed parse.
/// Captures only `type` and `id`, so we can dispatch + preserve error codes
/// without materializing a full `serde_json::Value` tree per sync tick.
#[derive(Deserialize)]
struct EmitMessageHeader {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    id: Option<String>,
}

fn sync_response_for_line(
    engine: &mut EmitEngine,
    trimmed: &str,
) -> std::result::Result<SyncResultMessage, ErrorMessage> {
    let header: EmitMessageHeader = serde_json::from_str(trimmed).map_err(|error| {
        ErrorMessage::new(None, "invalid_json", format!("invalid JSON: {error}"))
    })?;

    if header.msg_type != "sync" {
        return Err(ErrorMessage::new(
            header.id,
            "unknown_message_type",
            format!("unsupported message type: {}", header.msg_type),
        ));
    }

    let request: SyncRequest = serde_json::from_str(trimmed).map_err(|error| {
        ErrorMessage::new(
            header.id,
            "invalid_request",
            format!("invalid sync request shape: {error}"),
        )
    })?;

    request.config.validate().map_err(|error| {
        ErrorMessage::new(
            Some(request.id.clone()),
            "invalid_config",
            error.to_string(),
        )
    })?;
    validate_request_action_cues(&request)
        .map_err(|error| ErrorMessage::new(Some(request.id.clone()), "invalid_request", error))?;
    Ok(engine.sync(&request))
}

fn validate_request_action_cues(request: &SyncRequest) -> std::result::Result<(), String> {
    for session in &request.sessions {
        for (idx, cue) in session.action_cues.iter().enumerate() {
            if !cue.is_valid() {
                return Err(format!(
                    "invalid action_cues[{idx}] for session {}: evidence must exactly match kind {}",
                    session.session_id,
                    cue.kind.as_str()
                ));
            }
        }
    }

    Ok(())
}

enum EmitLineResult {
    Sync(SyncResultMessage),
    Error(ErrorMessage),
}

impl EmitLineResult {
    fn write<W: Write>(self, stdout: &mut W) -> Result<()> {
        match self {
            Self::Sync(response) => write_json_line(stdout, &response),
            Self::Error(error) => write_json_line(stdout, &error),
        }
    }
}

fn run_defaults() -> Result<()> {
    let backend = resolve_model_backend();
    let model = default_model_for_backend(backend);

    #[derive(Serialize)]
    struct Defaults {
        model: String,
        backend: &'static str,
        agent_prompt: &'static str,
        terminal_prompt: &'static str,
    }

    let defaults = Defaults {
        model,
        backend: backend.as_str(),
        agent_prompt: DEFAULT_AGENT_PREAMBLE,
        terminal_prompt: DEFAULT_TERMINAL_PREAMBLE,
    };

    print_json(&defaults, false)
}

fn validate_extract_limits(
    max_actions: usize,
    max_task_chars: usize,
    max_detail_chars: usize,
) -> Result<()> {
    if max_actions == 0 {
        anyhow::bail!("--max-actions must be greater than 0");
    }
    if max_task_chars == 0 {
        anyhow::bail!("--max-task-chars must be greater than 0");
    }
    if max_detail_chars == 0 {
        anyhow::bail!("--max-detail-chars must be greater than 0");
    }

    Ok(())
}

fn validate_tmux_emit_args(args: &TmuxEmitArgs) -> Result<()> {
    if args.interval_ms == 0 {
        anyhow::bail!("--interval-ms must be greater than 0");
    }
    next_tmux_reconcile_deadline(args.interval_ms)?;
    Ok(())
}

fn print_json<T: Serialize>(value: &T, pretty: bool) -> Result<()> {
    let mut stdout = io::stdout().lock();
    if pretty {
        serde_json::to_writer_pretty(&mut stdout, value)
            .context("failed to write JSON response")?;
    } else {
        serde_json::to_writer(&mut stdout, value).context("failed to write JSON response")?;
    }
    writeln!(stdout).context("failed to write JSON response")?;
    Ok(())
}

fn demo_tool(tool: DemoToolArg) -> AgentTool {
    match tool {
        DemoToolArg::Claude => AgentTool::Claude,
        DemoToolArg::Codex => AgentTool::Codex,
    }
}

fn run_tmux_emit(args: TmuxEmitArgs) -> Result<()> {
    validate_tmux_emit_args(&args)?;

    let model_client = build_model_client()
        .map_err(|error| anyhow::anyhow!("failed to initialize model client: {error}"))?;
    let mut engine = EmitEngine::new(model_client);
    let mut tracker = TmuxScanTracker::new();
    let mut stdout = io::stdout().lock();
    let mut seq = 0u64;
    let tmux_config = tmux_emit_config(&args)?;
    let socket_path = args.socket.unwrap_or_else(default_tmux_socket_path);
    let mut socket_guard = None;

    write_json_line(&mut stdout, &HelloMessage::new())?;

    if !args.once {
        socket_guard = Some(bind_tmux_socket(&socket_path)?);
    }

    emit_tmux_scan(
        &mut stdout,
        &mut engine,
        &mut tracker,
        &mut seq,
        args.max_capture_lines,
        &tmux_config,
    )?;

    if args.once {
        return Ok(());
    }

    let Some(socket) = socket_guard.as_ref() else {
        return Err(anyhow::anyhow!("socket guard missing for tmux emit loop"));
    };
    run_tmux_emit_loop(
        &mut stdout,
        &mut engine,
        &mut tracker,
        &mut seq,
        args.max_capture_lines,
        args.interval_ms,
        &tmux_config,
        &socket.reader,
    )
}

// Loop wiring needs the output, engine, tracker, scan settings, config, and
// notify socket as independently-owned values. Keeping them explicit avoids a
// broad mutable context type for one call site and its focused test.
#[allow(clippy::too_many_arguments)]
fn run_tmux_emit_loop<W: Write>(
    stdout: &mut W,
    engine: &mut EmitEngine,
    tracker: &mut TmuxScanTracker,
    seq: &mut u64,
    max_capture_lines: usize,
    interval_ms: u64,
    tmux_config: &clawgs::emit::protocol::ThoughtConfig,
    socket: &UnixDatagram,
) -> Result<()> {
    let mut next_reconcile_at = next_tmux_reconcile_deadline(interval_ms)?;
    let mut buf = [0u8; 512];

    loop {
        if !should_scan_tmux(socket, &mut buf, next_reconcile_at)? {
            continue;
        }

        emit_tmux_scan(stdout, engine, tracker, seq, max_capture_lines, tmux_config)?;
        next_reconcile_at = next_tmux_reconcile_deadline(interval_ms)?;
    }
}

fn next_tmux_reconcile_deadline(interval_ms: u64) -> Result<Instant> {
    if interval_ms > MAX_TMUX_INTERVAL_MS {
        anyhow::bail!("--interval-ms must be at most {MAX_TMUX_INTERVAL_MS}");
    }

    Instant::now()
        .checked_add(Duration::from_millis(interval_ms))
        .ok_or_else(|| anyhow::anyhow!("--interval-ms is too large"))
}

fn tmux_emit_config(args: &TmuxEmitArgs) -> Result<clawgs::emit::protocol::ThoughtConfig> {
    let mut config = match args.config_json.as_deref() {
        Some(raw) => {
            serde_json::from_str(raw).context("failed to parse --config-json for tmux-emit")?
        }
        None => clawgs::emit::protocol::ThoughtConfig::default(),
    };

    if !args.model.trim().is_empty() {
        config.model = args.model.clone();
    }

    config = config
        .normalize_and_validate()
        .map_err(|error| anyhow::anyhow!("invalid tmux emit config: {error}"))?;
    Ok(config)
}

fn write_json_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value).context("failed to write JSON response")?;
    writer.write_all(b"\n").context("failed to write newline")?;
    writer.flush().context("failed to flush output")?;
    Ok(())
}

fn run_tmux_notify(args: TmuxNotifyArgs) -> Result<()> {
    let socket_path = args.socket.unwrap_or_else(default_tmux_socket_path);
    send_notify_datagram_strict(&socket_path, args.event.as_bytes())?;
    Ok(())
}

fn run_claude_hook_notify(args: ClaudeHookNotifyArgs) -> Result<()> {
    let socket_path = args.socket.unwrap_or_else(default_tmux_socket_path);
    let mut stdin = io::stdin().lock();
    let mut raw = String::new();
    let _ = stdin.read_to_string(&mut raw);
    let event = claude_hook_event_tag(&raw);
    send_notify_datagram_best_effort(&socket_path, event.as_bytes())?;
    Ok(())
}

fn send_notify_datagram_strict(socket_path: &PathBuf, payload: &[u8]) -> Result<()> {
    let sender = UnixDatagram::unbound().context("failed to create tmux notify socket")?;
    sender
        .send_to(payload, socket_path)
        .with_context(|| format!("failed to notify tmux daemon at {}", socket_path.display()))?;
    Ok(())
}

fn send_notify_datagram_best_effort(socket_path: &PathBuf, payload: &[u8]) -> Result<()> {
    let sender = UnixDatagram::unbound().context("failed to create tmux notify socket")?;
    // Claude hooks should be safe to install even before the daemon is running.
    let _ = sender.send_to(payload, socket_path);
    Ok(())
}

fn claude_hook_event_tag(raw: &str) -> String {
    let input = serde_json::from_str::<ClaudeHookInput>(raw).unwrap_or_default();
    let event = compact_hook_part(input.hook_event_name.as_deref(), "unknown");
    let session = compact_hook_part(input.session_id.as_deref(), "");
    let cwd = compact_hook_part(input.cwd.as_deref().and_then(last_path_segment), "");
    let transcript = compact_hook_part(
        input.transcript_path.as_deref().and_then(last_path_segment),
        "",
    );

    let mut tag = format!("claude-code:{event}");
    append_hook_part(&mut tag, "session", &session);
    append_hook_part(&mut tag, "cwd", &cwd);
    append_hook_part(&mut tag, "transcript", &transcript);
    tag
}

fn append_hook_part(tag: &mut String, key: &str, value: &str) {
    if !value.is_empty() {
        tag.push(' ');
        tag.push_str(key);
        tag.push('=');
        tag.push_str(value);
    }
}

fn last_path_segment(path: &str) -> Option<&str> {
    path.rsplit('/').find(|segment| !segment.is_empty())
}

fn compact_hook_part(value: Option<&str>, fallback: &str) -> String {
    let raw = value.map(str::trim).filter(|value| !value.is_empty());
    let compact = raw.unwrap_or(fallback).chars().take(64).map(safe_tag_char);
    compact.collect()
}

fn safe_tag_char(ch: char) -> char {
    if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':') {
        ch
    } else {
        '_'
    }
}

fn emit_tmux_scan<W: Write>(
    stdout: &mut W,
    engine: &mut EmitEngine,
    tracker: &mut TmuxScanTracker,
    seq: &mut u64,
    max_capture_lines: usize,
    config: &clawgs::emit::protocol::ThoughtConfig,
) -> Result<()> {
    *seq += 1;
    let now = chrono::Utc::now();
    let sessions = tracker.scan(now, max_capture_lines)?;

    let result = engine.sync(&SyncRequest {
        id: format!("tmux-{}", *seq),
        now,
        config: config.clone(),
        sessions,
    });

    write_json_line(stdout, &result)
}

fn default_tmux_socket_path() -> PathBuf {
    tmux_socket_override().unwrap_or_else(fallback_tmux_socket_path)
}

#[derive(Debug)]
struct TmuxSocketGuard {
    path: PathBuf,
    reader: UnixDatagram,
}

impl Drop for TmuxSocketGuard {
    fn drop(&mut self) {
        if path_is_unix_socket(&self.path) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn bind_tmux_socket(path: &PathBuf) -> Result<TmuxSocketGuard> {
    remove_existing_socket(path)?;
    let socket = UnixDatagram::bind(path)
        .with_context(|| format!("failed to bind tmux notify socket at {}", path.display()))?;
    Ok(TmuxSocketGuard {
        path: path.clone(),
        reader: socket,
    })
}

fn tmux_socket_override() -> Option<PathBuf> {
    trimmed_env_var("CLAWGS_TMUX_SOCKET").map(PathBuf::from)
}

fn fallback_tmux_socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("clawgs-tmux-{}.sock", username_or_default()))
}

fn username_or_default() -> String {
    trimmed_env_var("USER").unwrap_or_else(|| "default".to_string())
}

fn trimmed_env_var(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn remove_existing_socket(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    if !path_is_unix_socket(path) {
        anyhow::bail!(
            "refusing to remove non-socket path for tmux notify socket: {}",
            path.display()
        );
    }

    if unix_socket_has_listener(path) {
        anyhow::bail!(
            "tmux notify socket is already in use; refusing to replace active daemon socket: {}",
            path.display()
        );
    }

    std::fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove existing tmux socket at {}",
            path.display()
        )
    })
}

fn unix_socket_has_listener(path: &PathBuf) -> bool {
    for attempt in 0..TMUX_SOCKET_PROBE_ATTEMPTS {
        if UnixDatagram::unbound()
            .and_then(|sender| sender.send_to(b"clawgs-probe", path))
            .is_err()
        {
            return false;
        }
        if attempt + 1 < TMUX_SOCKET_PROBE_ATTEMPTS {
            std::thread::sleep(TMUX_SOCKET_PROBE_RETRY_DELAY);
        }
    }

    true
}

fn path_is_unix_socket(path: &PathBuf) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

fn should_scan_tmux(
    socket: &UnixDatagram,
    buf: &mut [u8],
    next_reconcile_at: Instant,
) -> Result<bool> {
    socket
        .set_read_timeout(Some(tmux_socket_timeout(next_reconcile_at)))
        .context("failed to set tmux socket timeout")?;
    match socket.recv(buf) {
        Ok(_) => {
            drain_tmux_socket(socket, buf)?;
            Ok(true)
        }
        Err(error) if is_tmux_retryable_error(&error) => Ok(Instant::now() >= next_reconcile_at),
        Err(error) => Err(error).context("failed to read tmux notify socket"),
    }
}

fn tmux_socket_timeout(next_reconcile_at: Instant) -> Duration {
    // UnixDatagram::set_read_timeout rejects Some(Duration::ZERO) with
    // InvalidInput, so clamp the low end to 1ms. If the deadline has already
    // passed, a 1ms wait is enough for should_scan_tmux's timeout branch to
    // trip its `Instant::now() >= next_reconcile_at` check and force a scan.
    next_reconcile_at
        .saturating_duration_since(Instant::now())
        .clamp(Duration::from_millis(1), Duration::from_millis(1_000))
}

fn is_tmux_retryable_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn drain_tmux_socket(socket: &UnixDatagram, buf: &mut [u8]) -> Result<()> {
    socket
        .set_nonblocking(true)
        .context("failed to set tmux socket nonblocking")?;

    loop {
        match socket.recv(buf) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => {
                socket
                    .set_nonblocking(false)
                    .context("failed to restore tmux socket blocking mode")?;
                return Err(error).context("failed while draining tmux socket");
            }
        }
    }

    socket
        .set_nonblocking(false)
        .context("failed to restore tmux socket blocking mode")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use clawgs::emit::model_client::ModelClient;
    use tempfile::tempdir;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct DummyModelClient;

    impl ModelClient for DummyModelClient {
        fn complete(&self, _prompt: &str, _model_override: Option<&str>) -> Result<String, String> {
            Ok("unused".to_string())
        }
    }

    #[test]
    fn run_tmux_emit_loop_surfaces_scan_errors_after_socket_event() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let previous_tmux_bin = std::env::var("CLAWGS_TMUX_BIN").ok();
        std::env::set_var("CLAWGS_TMUX_BIN", "/definitely/missing-tmux");

        let (sender, receiver) = UnixDatagram::pair().expect("socket pair");
        sender.send(b"tick").expect("send tick");

        let mut stdout = Vec::new();
        let mut engine = EmitEngine::new(Box::new(DummyModelClient));
        let mut tracker = TmuxScanTracker::new();
        let mut seq = 0u64;
        let config = clawgs::emit::protocol::ThoughtConfig::default();

        let error = run_tmux_emit_loop(
            &mut stdout,
            &mut engine,
            &mut tracker,
            &mut seq,
            50,
            1_000,
            &config,
            &receiver,
        )
        .expect_err("scan failure");

        assert!(error
            .to_string()
            .contains("failed to run /definitely/missing-tmux list-panes"));

        if let Some(value) = previous_tmux_bin {
            std::env::set_var("CLAWGS_TMUX_BIN", value);
        } else {
            std::env::remove_var("CLAWGS_TMUX_BIN");
        }
    }

    #[test]
    fn default_tmux_socket_path_uses_override_when_present() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        std::env::set_var("CLAWGS_TMUX_SOCKET", " /tmp/custom.sock ");

        assert_eq!(
            default_tmux_socket_path(),
            PathBuf::from("/tmp/custom.sock")
        );

        std::env::remove_var("CLAWGS_TMUX_SOCKET");
    }

    #[test]
    fn default_tmux_socket_path_falls_back_to_username() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("CLAWGS_TMUX_SOCKET");
        std::env::set_var("USER", "tester");

        let socket_path = default_tmux_socket_path();

        assert!(socket_path.ends_with(Path::new("clawgs-tmux-tester.sock")));
        std::env::remove_var("USER");
    }

    #[test]
    fn bind_tmux_socket_replaces_existing_socket() {
        let dir = tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let stale = UnixDatagram::bind(&socket_path).expect("bind stale socket");
        drop(stale);

        let guard = bind_tmux_socket(&socket_path).expect("bind socket");

        assert!(socket_path.exists());
        drop(guard);
        assert!(!socket_path.exists());
    }

    #[test]
    fn bind_tmux_socket_refuses_active_socket() {
        let dir = tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let _active = UnixDatagram::bind(&socket_path).expect("bind active socket");

        let error = bind_tmux_socket(&socket_path).expect_err("active socket should be refused");

        assert!(error.to_string().contains("already in use"));
        assert!(
            path_is_unix_socket(&socket_path),
            "active daemon socket must not be unlinked"
        );
    }

    #[test]
    fn bind_tmux_socket_refuses_regular_file() {
        let dir = tempdir().expect("tempdir");
        let socket_path = dir.path().join("not-a-socket");
        std::fs::write(&socket_path, "do not delete").expect("write regular file");

        let error = bind_tmux_socket(&socket_path).expect_err("regular file should not be removed");

        assert!(error.to_string().contains("refusing to remove non-socket"));
        assert_eq!(
            std::fs::read_to_string(&socket_path).expect("regular file should remain"),
            "do not delete"
        );
    }

    #[test]
    fn tmux_socket_timeout_never_returns_zero_duration() {
        let past = Instant::now() - Duration::from_secs(5);
        assert!(
            tmux_socket_timeout(past) >= Duration::from_millis(1),
            "past deadlines must not produce a zero timeout (set_read_timeout rejects it)"
        );

        let future = Instant::now() + Duration::from_secs(30);
        assert!(
            tmux_socket_timeout(future) <= Duration::from_millis(1_000),
            "future deadlines are capped to the reconcile poll ceiling"
        );
    }

    #[test]
    fn tmux_emit_args_reject_zero_interval() {
        let args = TmuxEmitArgs {
            interval_ms: 0,
            max_capture_lines: 200,
            once: false,
            model: String::new(),
            config_json: None,
            socket: None,
        };

        let error = validate_tmux_emit_args(&args).expect_err("zero interval should fail");
        assert!(error
            .to_string()
            .contains("--interval-ms must be greater than 0"));
    }

    #[test]
    fn tmux_emit_args_reject_too_large_interval() {
        let args = TmuxEmitArgs {
            interval_ms: u64::MAX,
            max_capture_lines: 200,
            once: false,
            model: String::new(),
            config_json: None,
            socket: None,
        };

        let error = validate_tmux_emit_args(&args).expect_err("huge interval should fail");
        assert!(error.to_string().contains("--interval-ms must be at most"));
    }

    #[test]
    fn claude_hook_event_tag_compacts_common_fields() {
        let raw = r#"{
            "hook_event_name": "PostToolUse",
            "session_id": "abc 123",
            "cwd": "/tmp/my project",
            "transcript_path": "/Users/me/.claude/projects/demo/session.jsonl"
        }"#;

        assert_eq!(
            claude_hook_event_tag(raw),
            "claude-code:PostToolUse session=abc_123 cwd=my_project transcript=session.jsonl"
        );
    }

    #[test]
    fn claude_hook_event_tag_tolerates_invalid_input() {
        assert_eq!(claude_hook_event_tag("not-json"), "claude-code:unknown");
    }

    #[test]
    fn remove_existing_socket_is_noop_for_missing_path() {
        let dir = tempdir().expect("tempdir");
        let socket_path = dir.path().join("missing.sock");

        remove_existing_socket(&socket_path).expect("missing path should be fine");

        assert!(!socket_path.exists());
    }
}
