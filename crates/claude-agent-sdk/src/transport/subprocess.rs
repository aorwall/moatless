use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::error::{Result, SdkError};
use crate::types::control::RawStdoutMessage;
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

        debug!(cli_path = %cli_path, args = ?args, "Spawning Claude CLI");

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
                SdkError::CliNotFound { path: cli_path }
            } else {
                SdkError::SpawnError(e)
            }
        })?;

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

            return Ok(Some(RawStdoutMessage::from_value(value)));
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
        match tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(status)) => {
                debug!(status = ?status, "CLI process exited");
            }
            Ok(Err(e)) => {
                warn!(error = %e, "Error waiting for CLI process");
            }
            Err(_) => {
                warn!("CLI process did not exit within 5s, force killing");
                let _ = self.child.kill().await;
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
        debug!(stderr_line = %line, "CLI stderr");
        collected.push(line);
    }

    collected
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
