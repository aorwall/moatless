//! Build `ClaudeOptions` from sandbox config + per-request overrides.

use std::collections::HashMap;
use std::sync::Arc;

use claude_agent_sdk::{ClaudeOptions, PermissionMode};
use claude_agent_sdk::types::options::{PluginConfig, SystemPrompt};
use tokio_util::sync::CancellationToken;

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
    // Determine model: request > agentConfig > default
    let model = request
        .model
        .clone()
        .or_else(|| config.agent_config.model.clone());

    // Build system prompt
    let system_prompt = build_system_prompt(config, request);

    // Build plugin configs from paths
    let plugins = build_plugins(config);

    // Build the permission handler for interactive prompts
    let permission_handler = InteractivePermissionHandler::new(agent_state);

    // Determine session resume
    let (resume, continue_conversation) = resolve_resume(request);

    ClaudeOptions {
        model,
        system_prompt,
        permission_mode: Some(PermissionMode::AcceptEdits),
        cwd: Some(config.workspace_dir.clone()),
        cli_path: Some(config.claude_cli_path.clone()),
        plugins: if plugins.is_empty() { None } else { Some(plugins) },
        resume,
        continue_conversation,
        can_use_tool: Some(Arc::new(permission_handler)),
        cancellation_token: Some(cancel_token),
        max_turns: config.agent_config.timeout.map(|t| t as u32),
        init_timeout: Some(std::time::Duration::from_secs(120)),
        env,
        ..ClaudeOptions::default()
    }
}

/// Build the system prompt configuration.
#[allow(clippy::unnecessary_wraps)] // Option for caller API consistency
fn build_system_prompt(config: &Config, request: &AgentRunRequest) -> Option<SystemPrompt> {
    // Combine config system prompt + request system prompt as appended text
    let mut append_parts = Vec::new();

    if let Some(ref sp) = config.agent_config.system_prompt {
        append_parts.push(sp.clone());
    }

    if let Some(ref sp) = request.system_prompt {
        append_parts.push(sp.clone());
    }

    if append_parts.is_empty() {
        // Use the default preset with no append
        Some(SystemPrompt::Preset {
            prompt_type: "preset".to_string(),
            preset: "claude_code".to_string(),
            append: None,
        })
    } else {
        Some(SystemPrompt::Preset {
            prompt_type: "preset".to_string(),
            preset: "claude_code".to_string(),
            append: Some(append_parts.join("\n\n")),
        })
    }
}

/// Build plugin configs from config paths.
fn build_plugins(config: &Config) -> Vec<PluginConfig> {
    config
        .agent_config
        .plugin_paths
        .iter()
        .map(|path| PluginConfig {
            plugin_type: "local".to_string(),
            path: path.clone(),
        })
        .collect()
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
