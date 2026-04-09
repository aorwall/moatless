//! Build agent prompt/options from sandbox config + per-request overrides.

use std::collections::HashMap;
use std::sync::Arc;

use claude_agent_sdk::types::options::{PluginConfig, SystemPrompt};
use claude_agent_sdk::{ClaudeOptions, PermissionMode};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::agent::types::AgentRunRequest;
use crate::config::Config;

use super::agent_state::AgentState;
use super::permission_handler::InteractivePermissionHandler;

/// Build ClaudeOptions from config + per-request overrides.
pub fn build_options(
    config: &Config,
    request: &AgentRunRequest,
    cancel_token: CancellationToken,
    agent_state: Arc<AgentState>,
    env: Option<HashMap<String, String>>,
) -> ClaudeOptions {
    // Determine model: request > agentConfig > DEFAULT_CLAUDE_CODE_MODEL env
    let model = request
        .model
        .clone()
        .or_else(|| config.agent_config.model.clone())
        .or_else(|| config.default_model.clone());

    // Build system prompt
    let system_prompt = build_system_prompt(config, request);

    // Build plugin configs from paths
    let plugins = build_plugins(config);

    // Detect plan mode from agent_mode.
    let is_plan_mode = request.agent_mode.as_deref() == Some("plan");

    // Build the permission handler for interactive prompts
    let permission_handler =
        InteractivePermissionHandler::with_plan_mode(agent_state, is_plan_mode);

    // Determine session resume
    let (resume, continue_conversation) = resolve_resume(request);
    let init_timeout = std::time::Duration::from_secs(120);

    info!(
        task_id = %config.task_id,
        model = ?model,
        resume = ?resume,
        continue_conversation = ?continue_conversation,
        cli_path = %config.claude_cli_path,
        init_timeout_secs = init_timeout.as_secs(),
        is_plan_mode,
        "Built agent options"
    );

    ClaudeOptions {
        model,
        system_prompt,
        permission_mode: Some(if is_plan_mode {
            PermissionMode::Plan
        } else {
            PermissionMode::AcceptEdits
        }),
        cwd: Some(config.workspace_dir.clone()),
        cli_path: Some(config.claude_cli_path.clone()),
        plugins: if plugins.is_empty() {
            None
        } else {
            Some(plugins)
        },
        resume,
        continue_conversation,
        can_use_tool: Some(Arc::new(permission_handler)),
        cancellation_token: Some(cancel_token),
        max_turns: config.agent_config.timeout.map(|t| t as u32),
        init_timeout: Some(init_timeout),
        env,
        ..ClaudeOptions::default()
    }
}

/// Behavioral directive prepended before all other appended prompt content
/// so it is never buried under reference material.
const REQUIRED_FIRST_ACTIONS: &str = "\
# Required First Actions

Before responding to the user's first message, you MUST run:
  `moat set-name \"<name>\"`
Choose a short, descriptive name based on the user's request (under 50 characters).";

/// CLI reference appended at the end of the system prompt.
const SANDBOX_TOOLS_PROMPT: &str = "\
# Sandbox CLI Reference

Available `moat` commands:
- `moat set-name \"<name>\"` — Update the task name
- `moat set-tags <tag1> <tag2> ...` — Set tags on this task
- `moat tasks status $TASK_ID` — Show sandbox status including server states
- `moat tasks restart $TASK_ID` — Restart the sandbox (restarts all servers)
- `moat tasks logs $TASK_ID --server <name>` — Stream server logs (e.g., `--server preview`)
- `moat tasks redeploy $TASK_ID` — Recreate sandbox with fresh repo config";

/// Build the system prompt configuration.
#[allow(clippy::unnecessary_wraps)] // Option for caller API consistency
fn build_system_prompt(config: &Config, request: &AgentRunRequest) -> Option<SystemPrompt> {
    Some(SystemPrompt::Preset {
        prompt_type: "preset".to_string(),
        preset: "claude_code".to_string(),
        append: Some(build_prompt_append(config, request)),
    })
}

/// Build the shared prompt appendix used by both Claude and Codex.
pub fn build_prompt_append(config: &Config, request: &AgentRunRequest) -> String {
    let mut append_parts = Vec::new();

    // Only ask the agent to set a task name on the first session in this pod's lifetime.
    if request.is_first_session {
        append_parts.push(REQUIRED_FIRST_ACTIONS.to_string());
    }

    if let Some(ref sp) = config.agent_config.system_prompt {
        append_parts.push(sp.clone());
    }

    if let Some(ref sp) = request.system_prompt {
        append_parts.push(sp.clone());
    }

    // Add preview server instructions if servers are configured
    if let Some(server_prompt) = build_server_prompt(config) {
        append_parts.push(server_prompt);
    }

    // Add task URL so the agent can reference it
    if let Some(ref frontend_url) = config.frontend_url {
        append_parts.push(format!(
            "# Task URL\n\nThis task is available at: {}/tasks/{}",
            frontend_url.trim_end_matches('/'),
            config.task_id
        ));
    }

    // CLI reference last
    append_parts.push(SANDBOX_TOOLS_PROMPT.to_string());

    let append = append_parts.join("\n\n");
    info!(
        append_system_prompt_len = append.len(),
        parts = append_parts.len(),
        has_config_prompt = config.agent_config.system_prompt.is_some(),
        has_request_prompt = request.system_prompt.is_some(),
        "Assembled system prompt"
    );

    append
}

/// Build a system prompt section describing configured preview servers.
///
/// Returns `None` if no servers are configured.
fn build_server_prompt(config: &Config) -> Option<String> {
    if config.servers.is_empty() {
        return None;
    }

    let mut lines = vec![
        "# Preview Servers".to_string(),
        String::new(),
        "The following preview servers are running as sidecars in this sandbox environment. \
         They start automatically when the workspace is ready — you do not need to start them manually."
            .to_string(),
    ];

    for server in &config.servers {
        lines.push(String::new());
        let label = server.label.as_deref().unwrap_or(&server.name);
        lines.push(format!("## {label} (port {port})", port = server.port));

        // Local URL — agent can access directly via same-pod networking
        lines.push(format!(
            "- **Local URL**: http://localhost:{} (use this to curl/verify the server — no auth needed)",
            server.port
        ));

        // External URL — for the user's browser (they are already authenticated)
        if let Some(ref domain) = config.proxy.domain {
            let url = format!(
                "{}://{}--{}.{}",
                config.proxy.protocol, config.task_id, server.port, domain
            );
            lines.push(format!(
                "- **External URL**: {url} (share with the user to open in their browser — they are already authenticated)"
            ));
        }

        if let Some(ref cmds) = server.start_commands {
            let joined = cmds.join(" && ");
            lines.push(format!(
                "- **Configured start command**: `{joined}` (runs automatically, configured in the repository)"
            ));
        }

        if server.auto_reload {
            lines.push(
                "- **Auto-reload**: enabled (file changes are reflected automatically)".to_string(),
            );
        }
    }

    Some(lines.join("\n"))
}

/// Build plugin configs from config paths and cloned plugin repos.
fn build_plugins(config: &Config) -> Vec<PluginConfig> {
    let mut plugins: Vec<PluginConfig> = config
        .agent_config
        .plugin_paths
        .iter()
        .map(|path| PluginConfig {
            plugin_type: "local".to_string(),
            path: path.clone(),
        })
        .collect();

    // Discover cloned plugin repos in /opt/moatless/plugins/
    let plugins_dir = std::path::Path::new("/opt/moatless/plugins");
    if plugins_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    plugins.push(PluginConfig {
                        plugin_type: "local".to_string(),
                        path: path.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }

    plugins
}

/// Resolve session resume from request.
///
/// Returns `(resume_session_id, continue_conversation)`:
/// - `WithId(id)` → `--resume <id>`
/// - `Last` → `--continue` (let the CLI find the latest session)
fn resolve_resume(request: &AgentRunRequest) -> (Option<String>, Option<bool>) {
    use super::types::ResumeSession;
    match &request.resume {
        Some(ResumeSession::WithId(session_id)) => (Some(session_id.clone()), None),
        Some(ResumeSession::Last) => (None, Some(true)),
        None => (None, None),
    }
}
