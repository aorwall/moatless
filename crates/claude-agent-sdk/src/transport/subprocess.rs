use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use serde_json::{Map, Value};
use tracing::{debug, info, trace, warn};

use crate::error::{Result, SdkError};
use crate::types::content::ContentBlock;
use crate::types::control::{ControlResponseInner, IncomingControlRequestInner, RawStdoutMessage};
use crate::types::messages::{ResultMessage, SdkMessage};
use crate::types::options::{ClaudeOptions, SystemPrompt};

/// Manages the Claude CLI subprocess: spawning, reading NDJSON, writing, and cleanup.
pub(crate) struct SubprocessTransport {
    child: Child,
    pub(crate) stdin: Arc<Mutex<ChildStdin>>,
    stdout_lines: Lines<BufReader<ChildStdout>>,
    cancellation_token: CancellationToken,
    _stderr_task: JoinHandle<Vec<String>>,
}

impl SubprocessTransport {
    /// Spawn the Claude CLI subprocess with the given options.
    pub(crate) async fn spawn(options: &ClaudeOptions) -> Result<Self> {
        let cli_path = resolve_cli_path(options)?;
        let args = build_cli_args(options);
        let cancellation_token = options
            .cancellation_token
            .clone()
            .unwrap_or_default();

        info!(cli_path = %cli_path, args = ?args, "Spawning Claude CLI");

        let mut cmd = tokio::process::Command::new(&cli_path);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(cwd) = &options.cwd {
            cmd.current_dir(cwd);
        }

        if let Some(env) = &options.env {
            cmd.envs(env);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SdkError::CliNotFound { path: cli_path.clone() }
            } else {
                SdkError::SpawnError(e)
            }
        })?;

        let pid = child.id().unwrap_or(0);
        info!(cli_path = %cli_path, pid, "Claude CLI subprocess spawned");

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SdkError::Internal("Failed to capture stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SdkError::Internal("Failed to capture stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SdkError::Internal("Failed to capture stderr".into()))?;

        let stdin = Arc::new(Mutex::new(stdin));
        let stdout_lines = BufReader::new(stdout).lines();

        // Drain stderr in background
        let stderr_callback = options.stderr_callback.as_ref().map(|_| ());
        let stderr_task = tokio::spawn(drain_stderr(stderr, stderr_callback.is_some()));

        Ok(Self {
            child,
            stdin,
            stdout_lines,
            cancellation_token,
            _stderr_task: stderr_task,
        })
    }

    /// Read the next message from stdout. Returns `None` on EOF.
    pub(crate) async fn read_message(&mut self) -> Result<Option<RawStdoutMessage>> {
        loop {
            let line = tokio::select! {
                result = self.stdout_lines.next_line() => {
                    match result {
                        Ok(Some(line)) => line,
                        Ok(None) => return Ok(None),
                        Err(e) => return Err(SdkError::SpawnError(e)),
                    }
                }
                () = self.cancellation_token.cancelled() => {
                    return Err(SdkError::Cancelled);
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
                SdkError::JsonParseError {
                    source: e,
                    raw_line: trimmed.to_string(),
                }
            })?;

            let msg = RawStdoutMessage::from_value(value);
            log_stdout_message(&msg);
            return Ok(Some(msg));
        }
    }

    /// Write a JSON line to stdin.
    #[allow(dead_code)]
    pub(crate) async fn write_line(&self, json: &str) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(json.as_bytes())
            .await
            .map_err(SdkError::SpawnError)?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(SdkError::SpawnError)?;
        stdin.flush().await.map_err(SdkError::SpawnError)?;
        Ok(())
    }

    /// Close the subprocess: send SIGTERM, wait 5 seconds, then SIGKILL.
    pub(crate) async fn close(&mut self) {
        // Close stdin first to signal EOF
        // (dropping the Arc<Mutex<ChildStdin>> won't close it immediately
        // if other references exist, but we try)
        drop(self.stdin.clone());

        // Send SIGTERM
        if let Err(e) = self.child.start_kill() {
            warn!(error = %e, "Failed to send kill signal to CLI process");
        }

        // Wait with timeout
        let exit_status = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.child.wait(),
        )
        .await
        {
            Ok(Ok(status)) => Some(status),
            Ok(Err(e)) => {
                warn!(error = %e, "Error waiting for CLI process");
                None
            }
            Err(_) => {
                warn!("CLI process did not exit within 5s, force killing");
                let _ = self.child.kill().await;
                None
            }
        };

        // Collect stderr output from the background drain task
        let stderr_lines = collect_stderr(&mut self._stderr_task).await;

        // Log exit status with full context
        if let Some(status) = exit_status {
            if status.success() {
                info!(code = status.code(), "CLI process exited");
            } else {
                // On Unix, a signal-killed process has code() == None;
                // use the signal number from .signal() instead.
                #[cfg(unix)]
                let signal = std::os::unix::process::ExitStatusExt::signal(&status);
                #[cfg(not(unix))]
                let signal: Option<i32> = None;

                warn!(
                    code = status.code(),
                    signal = signal,
                    stderr_lines = stderr_lines.len(),
                    "CLI process exited with error"
                );

                // Log last stderr lines for debugging (cap at 20 lines)
                let tail = if stderr_lines.len() > 20 {
                    &stderr_lines[stderr_lines.len() - 20..]
                } else {
                    &stderr_lines
                };
                for line in tail {
                    debug!(line = %line, "CLI stderr (tail)");
                }
            }
        }
    }
}

/// Resolve the CLI binary path.
fn resolve_cli_path(options: &ClaudeOptions) -> Result<String> {
    if let Some(path) = &options.cli_path {
        return Ok(path.clone());
    }

    // Try `which claude`
    if let Ok(output) = std::process::Command::new("which")
        .arg("claude")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }

    // Try default location
    let home = std::env::var("HOME").unwrap_or_default();
    let default_path = format!("{home}/.claude/local/claude");
    if std::path::Path::new(&default_path).exists() {
        return Ok(default_path);
    }

    // Fall back to just "claude" and let PATH resolution handle it
    Ok("claude".to_string())
}

/// Build CLI arguments from options.
pub(crate) fn build_cli_args(options: &ClaudeOptions) -> Vec<String> {
    let mut args = Vec::with_capacity(32);

    // Always-present flags
    args.extend_from_slice(&[
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--print".to_string(),
    ]);

    // Model
    if let Some(model) = &options.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    if let Some(model) = &options.fallback_model {
        args.push("--fallback-model".to_string());
        args.push(model.clone());
    }

    // Permission prompt tool — when a PermissionHandler is set, tell CLI to
    // send permission requests via the control protocol (stdin/stdout)
    if options.can_use_tool.is_some() {
        args.push("--permission-prompt-tool".to_string());
        args.push("stdio".to_string());
    } else if let Some(tool_name) = &options.permission_prompt_tool_name {
        args.push("--permission-prompt-tool".to_string());
        args.push(tool_name.clone());
    }

    // Permission mode
    if let Some(mode) = &options.permission_mode {
        args.push("--permission-mode".to_string());
        args.push(mode.to_string());
    }

    // Allow dangerously skip permissions
    if options.allow_dangerously_skip_permissions == Some(true) {
        args.push("--allow-dangerously-skip-permissions".to_string());
    }

    // Session
    if let Some(session) = &options.resume {
        args.push("--resume".to_string());
        args.push(session.clone());
    }
    if options.continue_conversation == Some(true) {
        args.push("--continue".to_string());
    }
    if options.fork_session == Some(true) {
        args.push("--fork-session".to_string());
    }
    if options.persist_session == Some(false) {
        args.push("--no-persist-session".to_string());
    }

    // Limits
    if let Some(turns) = options.max_turns {
        args.push("--max-turns".to_string());
        args.push(turns.to_string());
    }
    if let Some(budget) = options.max_budget_usd {
        args.push("--max-budget-usd".to_string());
        args.push(budget.to_string());
    }

    // Tools
    if let Some(tools) = &options.allowed_tools {
        if !tools.is_empty() {
            args.push("--allowedTools".to_string());
            args.push(tools.join(","));
        }
    }
    if let Some(tools) = &options.disallowed_tools {
        if !tools.is_empty() {
            args.push("--disallowedTools".to_string());
            args.push(tools.join(","));
        }
    }

    // Directories
    if let Some(dirs) = &options.additional_directories {
        for dir in dirs {
            args.push("--add-dir".to_string());
            args.push(dir.clone());
        }
    }

    // Settings sources
    if let Some(sources) = &options.setting_sources {
        if !sources.is_empty() {
            args.push("--setting-sources".to_string());
            args.push(
                sources
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
    }

    // System prompt
    if let Some(prompt) = &options.system_prompt {
        match prompt {
            SystemPrompt::Replace(text) => {
                args.push("--system-prompt".to_string());
                args.push(text.clone());
            }
            SystemPrompt::Preset { append, .. } => {
                if let Some(append_text) = append {
                    args.push("--append-system-prompt".to_string());
                    args.push(append_text.clone());
                }
            }
        }
    }

    // Plugins
    if let Some(plugins) = &options.plugins {
        for plugin in plugins {
            args.push("--plugin-dir".to_string());
            args.push(plugin.path.clone());
        }
    }

    // MCP servers — CLI expects {"mcpServers": {...}} wrapper
    if let Some(servers) = &options.mcp_servers {
        if !servers.is_empty() {
            let wrapper = serde_json::json!({ "mcpServers": servers });
            if let Ok(json) = serde_json::to_string(&wrapper) {
                args.push("--mcp-config".to_string());
                args.push(json);
            }
        }
    }

    // Strict MCP config
    if options.strict_mcp_config == Some(true) {
        args.push("--strict-mcp-config".to_string());
    }

    // Betas
    if let Some(betas) = &options.betas {
        if !betas.is_empty() {
            args.push("--betas".to_string());
            args.push(betas.join(","));
        }
    }

    // Streaming
    if options.include_partial_messages == Some(true) {
        args.push("--include-partial-messages".to_string());
    }

    // Output format
    if let Some(format) = &options.output_format {
        if let Ok(json) = serde_json::to_string(format) {
            args.push("--json-schema".to_string());
            args.push(json);
        }
    }

    // File checkpointing
    if options.enable_file_checkpointing == Some(true) {
        args.push("--enable-file-checkpointing".to_string());
    }

    // Agent
    if let Some(agent) = &options.agent {
        args.push("--agent".to_string());
        args.push(agent.clone());
    }

    // Agents (multi-agent definitions)
    if let Some(agents) = &options.agents {
        if !agents.is_empty() {
            if let Ok(json) = serde_json::to_string(agents) {
                args.push("--agents".to_string());
                args.push(json);
            }
        }
    }

    // Extra args
    if let Some(extra) = &options.extra_args {
        for (key, value) in extra {
            args.push(format!("--{key}"));
            if let Some(val) = value {
                args.push(val.clone());
            }
        }
    }

    args
}

/// Background task to drain stderr.
async fn drain_stderr(
    stderr: tokio::process::ChildStderr,
    _has_callback: bool,
) -> Vec<String> {
    let mut lines = BufReader::new(stderr).lines();
    let mut collected = Vec::new();

    while let Ok(Some(line)) = lines.next_line().await {
        debug!(line = %line, "CLI stderr");
        collected.push(line);
    }

    collected
}

/// Collect stderr lines from the drain task (with a short timeout).
async fn collect_stderr(task: &mut JoinHandle<Vec<String>>) -> Vec<String> {
    match tokio::time::timeout(std::time::Duration::from_secs(2), task).await {
        Ok(Ok(lines)) => lines,
        Ok(Err(e)) => {
            debug!(error = %e, "Stderr drain task panicked");
            Vec::new()
        }
        Err(_) => {
            debug!("Stderr drain task did not complete within 2s");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Pretty-print logging helpers
// ---------------------------------------------------------------------------

/// Log a parsed stdout message at the appropriate level with human-readable formatting.
fn log_stdout_message(msg: &RawStdoutMessage) {
    match msg {
        RawStdoutMessage::Sdk(sdk_msg) => match sdk_msg {
            SdkMessage::Assistant(m) => {
                let uuid_short = short_id(&m.uuid);
                let blocks = format_content_blocks(&m.message.content);
                debug!("CLI stdout: assistant (uuid={})\n{}", uuid_short, blocks);
            }
            SdkMessage::User(m) => {
                let blocks = format_content_blocks(&m.message.content);
                debug!("CLI stdout: user\n{}", blocks);
            }
            SdkMessage::UserReplay(m) => {
                let uuid_short = short_id(&m.uuid);
                let blocks = format_content_blocks(&m.message.content);
                debug!("CLI stdout: user_replay (uuid={})\n{}", uuid_short, blocks);
            }
            SdkMessage::Result(ResultMessage::Success(r)) => {
                let dur = r.duration_ms as f64 / 1000.0;
                let text = indent(&r.result, "  ");
                info!(
                    "CLI stdout: result success turns={} duration={:.1}s cost=${:.4}\n{}",
                    r.num_turns, dur, r.total_cost_usd, text
                );
            }
            SdkMessage::Result(ResultMessage::Error(r)) => {
                let dur = r.duration_ms as f64 / 1000.0;
                let errors = r.errors.join("; ");
                warn!(
                    "CLI stdout: result error subtype={} turns={} duration={:.1}s cost=${:.4} errors={}",
                    r.subtype, r.num_turns, dur, r.total_cost_usd, errors
                );
            }
            SdkMessage::System(m) => {
                let session_short = short_id(&m.session_id);
                info!(
                    "CLI stdout: system init session={} model={} tools={}",
                    session_short,
                    m.model,
                    m.tools.len()
                );
            }
            SdkMessage::Status(m) => {
                let status = m.status.as_deref().unwrap_or("?");
                debug!("CLI stdout: status={}", status);
            }
            SdkMessage::CompactBoundary(m) => {
                debug!(
                    "CLI stdout: compact_boundary trigger={} pre_tokens={}",
                    m.compact_metadata.trigger, m.compact_metadata.pre_tokens
                );
            }
            SdkMessage::HookStarted(m) => {
                debug!(
                    "CLI stdout: hook_started id={} name={} event={}",
                    m.hook_id, m.hook_name, m.hook_event
                );
            }
            SdkMessage::HookProgress(m) => {
                debug!(
                    "CLI stdout: hook_progress id={} name={}",
                    m.hook_id, m.hook_name
                );
            }
            SdkMessage::HookResponse(m) => {
                debug!(
                    "CLI stdout: hook_response id={} name={} outcome={}",
                    m.hook_id, m.hook_name, m.outcome
                );
            }
            SdkMessage::TaskNotification(m) => {
                debug!(
                    "CLI stdout: task_notification task={} status={}",
                    m.task_id, m.status
                );
            }
            SdkMessage::FilesPersisted(m) => {
                debug!(
                    "CLI stdout: files_persisted files={} failed={}",
                    m.files.len(),
                    m.failed.len()
                );
            }
            SdkMessage::StreamEvent(_) => {
                trace!("CLI stdout: stream_event");
            }
            SdkMessage::ToolProgress(m) => {
                trace!(
                    "CLI stdout: tool_progress tool={} elapsed={:.1}s",
                    m.tool_name, m.elapsed_time_seconds
                );
            }
            SdkMessage::AuthStatus(m) => {
                debug!(
                    "CLI stdout: auth_status authenticating={}",
                    m.is_authenticating
                );
            }
            SdkMessage::ToolUseSummary(m) => {
                debug!("CLI stdout: tool_use_summary {}", truncate(&m.summary, 120));
            }
        },
        RawStdoutMessage::IncomingControlRequest {
            request_id,
            request,
        } => match request {
            IncomingControlRequestInner::CanUseTool {
                tool_name, input, ..
            } => {
                let input_summary = format_tool_input(tool_name, input);
                debug!(
                    "CLI stdout: control_request can_use_tool req={}\n  tool: {}\n{}",
                    request_id, tool_name, input_summary
                );
            }
            IncomingControlRequestInner::HookCallback { callback_id, .. } => {
                debug!(
                    "CLI stdout: control_request hook_callback req={} callback={}",
                    request_id, callback_id
                );
            }
            IncomingControlRequestInner::McpMessage { server_name, .. } => {
                debug!(
                    "CLI stdout: control_request mcp_message req={} server={}",
                    request_id, server_name
                );
            }
        },
        RawStdoutMessage::ControlResponse(resp) => match resp {
            ControlResponseInner::Success { request_id, .. } => {
                debug!("CLI stdout: control_response success req={}", request_id);
            }
            ControlResponseInner::Error {
                request_id, error, ..
            } => {
                debug!(
                    "CLI stdout: control_response error req={} error={}",
                    request_id,
                    truncate(error, 200)
                );
            }
        },
        RawStdoutMessage::ControlCancelRequest { request_id } => {
            debug!("CLI stdout: control_cancel req={}", request_id);
        }
        RawStdoutMessage::KeepAlive => {
            trace!("CLI stdout: keep_alive");
        }
        RawStdoutMessage::Unknown(value) => {
            let type_str = value
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            debug!(raw = %value, "CLI stdout: unknown type={}", type_str);
        }
    }
}

/// Format content blocks into a human-readable multi-line string.
fn format_content_blocks(blocks: &[ContentBlock]) -> String {
    use std::fmt::Write;

    if blocks.is_empty() {
        return "  (no content)".to_string();
    }

    let mut out = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                let _ = writeln!(out, "  Text:");
                out.push_str(&indent(&truncate(text, 1000), "    "));
                out.push('\n');
            }
            ContentBlock::ToolUse { id, name, input } => {
                let _ = writeln!(out, "  ToolUse[{}]: {}", id, name);
                let input_str =
                    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
                out.push_str(&indent(&truncate(&input_str, 500), "    "));
                out.push('\n');
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let status = if is_error.unwrap_or(false) {
                    "err"
                } else {
                    "ok"
                };
                let _ = writeln!(out, "  ToolResult[{}]: {}", tool_use_id, status);
                if let Some(content) = content {
                    let content_str = match content {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string_pretty(other)
                            .unwrap_or_else(|_| other.to_string()),
                    };
                    out.push_str(&indent(&truncate(&content_str, 500), "    "));
                    out.push('\n');
                }
            }
            ContentBlock::Thinking { thinking, .. } => {
                let _ = writeln!(out, "  Thinking:");
                out.push_str(&indent(&truncate(thinking, 500), "    "));
                out.push('\n');
            }
            ContentBlock::Image { source } => {
                let _ = writeln!(out, "  Image: (media={})", source.media_type);
            }
            ContentBlock::ServerToolUse { id, name, input } => {
                let _ = writeln!(out, "  ServerToolUse[{}]: {}", id, name);
                let input_str =
                    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
                out.push_str(&indent(&truncate(&input_str, 500), "    "));
                out.push('\n');
            }
            ContentBlock::WebSearchToolResult {
                tool_use_id,
                content,
            } => {
                let _ = writeln!(out, "  WebSearchToolResult[{}]", tool_use_id);
                if let Some(content) = content {
                    let content_str = serde_json::to_string_pretty(content)
                        .unwrap_or_else(|_| content.to_string());
                    out.push_str(&indent(&truncate(&content_str, 500), "    "));
                    out.push('\n');
                }
            }
        }
    }
    // Trim trailing newlines
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Format tool input for CanUseTool control requests, showing key fields for well-known tools.
fn format_tool_input(tool_name: &str, input: &Map<String, Value>) -> String {
    let key_fields: &[&str] = match tool_name {
        "Bash" => &["command", "description"],
        "Read" => &["file_path"],
        "Write" => &["file_path"],
        "Edit" | "MultiEdit" => &["file_path"],
        "Glob" => &["pattern", "path"],
        "Grep" => &["pattern", "path", "glob"],
        _ => &[],
    };

    let mut out = String::new();
    for &field in key_fields {
        if let Some(val) = input.get(field) {
            let val_str = match val {
                Value::String(s) => truncate(s, 200),
                other => truncate(&other.to_string(), 200),
            };
            out.push_str(&format!("  input.{}: {}\n", field, val_str));
        }
    }

    // If no key fields were found, show all keys
    if out.is_empty() {
        let keys: Vec<&str> = input.keys().map(|k| k.as_str()).collect();
        out.push_str(&format!("  input keys: [{}]\n", keys.join(", ")));
    }

    out.trim_end().to_string()
}

/// Return the first 8 characters of an ID string.
fn short_id(id: &str) -> &str {
    let end = id.len().min(8);
    // Ensure we're at a char boundary
    let mut e = end;
    while e > 0 && !id.is_char_boundary(e) {
        e -= 1;
    }
    &id[..e]
}

/// Indent each line of text with the given prefix.
fn indent(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{}{}", prefix, line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncate a string to max_len bytes, appending a note if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    // Find valid UTF-8 boundary
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...[+{} bytes]", &s[..end], s.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::permissions::PermissionMode;
    use std::collections::HashMap;

    #[test]
    fn build_args_default() {
        let options = ClaudeOptions::default();
        let args = build_cli_args(&options);

        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"stream-json".to_string()));
        assert!(args.contains(&"--input-format".to_string()));
        assert!(args.contains(&"--verbose".to_string()));
        assert!(args.contains(&"--print".to_string()));
    }

    #[test]
    fn build_args_with_model() {
        let options = ClaudeOptions {
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let model_idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[model_idx + 1], "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn build_args_with_permission_mode() {
        let options = ClaudeOptions {
            permission_mode: Some(PermissionMode::BypassPermissions),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let idx = args
            .iter()
            .position(|a| a == "--permission-mode")
            .unwrap();
        assert_eq!(args[idx + 1], "bypassPermissions");
    }

    #[test]
    fn build_args_with_max_turns() {
        let options = ClaudeOptions {
            max_turns: Some(10),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let idx = args.iter().position(|a| a == "--max-turns").unwrap();
        assert_eq!(args[idx + 1], "10");
    }

    #[test]
    fn build_args_with_additional_dirs() {
        let options = ClaudeOptions {
            additional_directories: Some(vec!["/tmp/a".to_string(), "/tmp/b".to_string()]),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let count = args.iter().filter(|a| a.as_str() == "--add-dir").count();
        assert_eq!(count, 2);
    }

    #[test]
    fn build_args_with_tools() {
        let options = ClaudeOptions {
            allowed_tools: Some(vec!["Read".to_string(), "Write".to_string()]),
            disallowed_tools: Some(vec!["Bash".to_string()]),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let allowed_idx = args
            .iter()
            .position(|a| a == "--allowedTools")
            .unwrap();
        assert_eq!(args[allowed_idx + 1], "Read,Write");

        let disallowed_idx = args
            .iter()
            .position(|a| a == "--disallowedTools")
            .unwrap();
        assert_eq!(args[disallowed_idx + 1], "Bash");
    }

    #[test]
    fn build_args_with_system_prompt_replace() {
        let options = ClaudeOptions {
            system_prompt: Some(SystemPrompt::Replace("You are helpful.".to_string())),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let idx = args
            .iter()
            .position(|a| a == "--system-prompt")
            .unwrap();
        assert_eq!(args[idx + 1], "You are helpful.");
    }

    #[test]
    fn build_args_with_system_prompt_preset_append() {
        let options = ClaudeOptions {
            system_prompt: Some(SystemPrompt::Preset {
                prompt_type: "preset".to_string(),
                preset: "claude_code".to_string(),
                append: Some("Also be funny.".to_string()),
            }),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let idx = args
            .iter()
            .position(|a| a == "--append-system-prompt")
            .unwrap();
        assert_eq!(args[idx + 1], "Also be funny.");
    }

    #[test]
    fn build_args_with_continue() {
        let options = ClaudeOptions {
            continue_conversation: Some(true),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        assert!(args.contains(&"--continue".to_string()));
    }

    #[test]
    fn build_args_with_resume() {
        let options = ClaudeOptions {
            resume: Some("session-123".to_string()),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let idx = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[idx + 1], "session-123");
    }

    #[test]
    fn build_args_with_extra_args() {
        let mut extra = HashMap::new();
        extra.insert("custom-flag".to_string(), Some("value".to_string()));
        extra.insert("bool-flag".to_string(), None);

        let options = ClaudeOptions {
            extra_args: Some(extra),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        assert!(args.contains(&"--custom-flag".to_string()));
        assert!(args.contains(&"--bool-flag".to_string()));
    }

    #[test]
    fn build_args_no_persist_session() {
        let options = ClaudeOptions {
            persist_session: Some(false),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        assert!(args.contains(&"--no-persist-session".to_string()));
    }

    #[test]
    fn build_args_betas() {
        let options = ClaudeOptions {
            betas: Some(vec!["context-1m-2025-08-07".to_string()]),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let idx = args.iter().position(|a| a == "--betas").unwrap();
        assert_eq!(args[idx + 1], "context-1m-2025-08-07");
    }

    #[test]
    fn build_args_with_mcp_config() {
        use crate::types::mcp::McpServerConfig;

        let mut servers = HashMap::new();
        servers.insert(
            "my-server".to_string(),
            McpServerConfig::Stdio {
                command: "npx".to_string(),
                args: Some(vec!["-y".to_string(), "my-mcp-server".to_string()]),
                env: None,
            },
        );
        let options = ClaudeOptions {
            mcp_servers: Some(servers),
            ..Default::default()
        };
        let args = build_cli_args(&options);

        let idx = args.iter().position(|a| a == "--mcp-config").unwrap();
        let json: serde_json::Value = serde_json::from_str(&args[idx + 1]).unwrap();
        // Must be wrapped in {"mcpServers": {...}}
        assert!(json.get("mcpServers").is_some());
        assert!(json["mcpServers"]["my-server"].is_object());
    }

    #[test]
    fn build_args_with_strict_mcp_config() {
        let options = ClaudeOptions {
            strict_mcp_config: Some(true),
            ..Default::default()
        };
        let args = build_cli_args(&options);
        assert!(args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn build_args_with_permission_prompt_tool() {
        let options = ClaudeOptions {
            permission_prompt_tool_name: Some("mcp_auth_tool".to_string()),
            ..Default::default()
        };
        let args = build_cli_args(&options);
        let idx = args
            .iter()
            .position(|a| a == "--permission-prompt-tool")
            .unwrap();
        assert_eq!(args[idx + 1], "mcp_auth_tool");
    }

    #[test]
    fn build_args_with_permission_handler_sets_stdio() {
        use crate::types::permissions::{PermissionContext, PermissionHandler, PermissionResult};
        use std::sync::Arc;

        struct TestHandler;
        #[async_trait::async_trait]
        impl PermissionHandler for TestHandler {
            async fn can_use_tool(
                &self,
                _tool_name: &str,
                _input: &serde_json::Map<String, serde_json::Value>,
                _context: PermissionContext,
            ) -> PermissionResult {
                PermissionResult::Allow {
                    updated_input: None,
                    updated_permissions: None,
                }
            }
        }

        let options = ClaudeOptions {
            can_use_tool: Some(Arc::new(TestHandler)),
            ..Default::default()
        };
        let args = build_cli_args(&options);
        let idx = args
            .iter()
            .position(|a| a == "--permission-prompt-tool")
            .unwrap();
        assert_eq!(args[idx + 1], "stdio");
    }

    #[test]
    fn build_args_permission_handler_takes_precedence_over_tool_name() {
        use crate::types::permissions::{PermissionContext, PermissionHandler, PermissionResult};
        use std::sync::Arc;

        struct TestHandler;
        #[async_trait::async_trait]
        impl PermissionHandler for TestHandler {
            async fn can_use_tool(
                &self,
                _tool_name: &str,
                _input: &serde_json::Map<String, serde_json::Value>,
                _context: PermissionContext,
            ) -> PermissionResult {
                PermissionResult::Allow {
                    updated_input: None,
                    updated_permissions: None,
                }
            }
        }

        let options = ClaudeOptions {
            can_use_tool: Some(Arc::new(TestHandler)),
            permission_prompt_tool_name: Some("should_be_ignored".to_string()),
            ..Default::default()
        };
        let args = build_cli_args(&options);
        let idx = args
            .iter()
            .position(|a| a == "--permission-prompt-tool")
            .unwrap();
        // When both are set, handler wins and we get "stdio"
        assert_eq!(args[idx + 1], "stdio");
    }

    #[test]
    fn build_args_with_allow_dangerously_skip_permissions() {
        let options = ClaudeOptions {
            allow_dangerously_skip_permissions: Some(true),
            ..Default::default()
        };
        let args = build_cli_args(&options);
        assert!(args.contains(
            &"--allow-dangerously-skip-permissions".to_string()
        ));
    }

    #[test]
    fn build_args_with_agents() {
        use crate::types::agents::AgentDefinition;

        let mut agents = HashMap::new();
        agents.insert(
            "reviewer".to_string(),
            AgentDefinition {
                description: "Reviews code".to_string(),
                prompt: "You are a code reviewer".to_string(),
                tools: None,
                disallowed_tools: None,
                model: None,
                mcp_servers: None,
                skills: None,
                max_turns: None,
            },
        );
        let options = ClaudeOptions {
            agents: Some(agents),
            ..Default::default()
        };
        let args = build_cli_args(&options);
        let idx = args.iter().position(|a| a == "--agents").unwrap();
        let json: serde_json::Value = serde_json::from_str(&args[idx + 1]).unwrap();
        assert!(json["reviewer"].is_object());
    }
}
