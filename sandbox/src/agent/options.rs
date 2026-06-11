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

    // Build the permission handler for interactive prompts (AskUserQuestion,
    // ExitPlanMode). It also enforces plan-mode write blocking when active.
    let permission_handler =
        InteractivePermissionHandler::with_plan_mode(agent_state, is_plan_mode);

    // Determine session resume
    let (resume, continue_conversation) = resolve_resume(request);
    let init_timeout = std::time::Duration::from_secs(120);

    // tmux mode always launches the TUI with --dangerously-skip-permissions,
    // which the CLI refuses under root unless IS_SANDBOX=1. The cluster sets
    // it via gitops extra env; Docker mode doesn't — set it for the child
    // unconditionally so the transport works in both runtimes.
    let env = if config.use_tmux {
        let mut env = env.unwrap_or_default();
        env.entry("IS_SANDBOX".to_string())
            .or_insert_with(|| "1".to_string());
        Some(env)
    } else {
        env
    };

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
        // Disallowed tools are removed from the CLI's context entirely (a bare
        // tool name in the deny list), so the agent never sees or calls them.
        //
        // - AskUserQuestion (both modes): we don't surface the CLI's
        //   multiple-choice question UI; the agent should ask in plain text
        //   instead. Removing the tool keeps it from offering choice prompts
        //   we can't render.
        // - Monitor (subprocess mode only): the CLI's built-in Monitor tool
        //   only delivers events while its own process is alive. The
        //   subprocess per-turn `--resume` model kills that process at turn
        //   end, so the Monitor watcher dies before any events reach the
        //   agent. Disallow it so the agent gets a loud "tool not allowed"
        //   error and falls back to Bash background. In tmux mode the TUI
        //   process persists across turns, so Monitor (and the CLI's internal
        //   ScheduleWakeup timer) work natively and Monitor stays allowed.
        disallowed_tools: Some(if config.use_tmux {
            vec!["AskUserQuestion".to_string()]
        } else {
            vec!["Monitor".to_string(), "AskUserQuestion".to_string()]
        }),
        ..ClaudeOptions::default()
    }
}

/// Build the system prompt configuration.
#[allow(clippy::unnecessary_wraps)] // Option for caller API consistency
fn build_system_prompt(config: &Config, request: &AgentRunRequest) -> Option<SystemPrompt> {
    Some(SystemPrompt::Preset {
        prompt_type: "preset".to_string(),
        preset: "claude_code".to_string(),
        append: Some(build_prompt_append(config, request)),
    })
}

/// Return the backend-supplied append-system-prompt verbatim.
///
/// The backend assembles the entire prompt (task identity, repo/channel
/// system prompts, preview servers, task URL, CLI reference, GitHub hint)
/// and stamps it onto every `PendingMessage.system_prompt`. The sandbox
/// is a passive recipient. The `_config` argument is retained for caller
/// API consistency.
pub fn build_prompt_append(_config: &Config, request: &AgentRunRequest) -> String {
    let append = request.system_prompt.clone().unwrap_or_default();
    info!(
        append_system_prompt_len = append.len(),
        has_request_prompt = request.system_prompt.is_some(),
        "Assembled system prompt"
    );
    append
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, ProxyConfig};

    fn test_config(use_tmux: bool) -> Config {
        Config {
            port: 0,
            grpc_port: 0,
            task_id: "task-test".to_string(),
            workspace_dir: "/tmp/workspace".to_string(),
            data_dir: "/tmp/data".to_string(),
            upload_dir: "/tmp/uploads".to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://localhost:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            agent_config: AgentConfig::default(),
            claude_cli_path: "/usr/bin/claude".to_string(),
            codex_cli_path: "/usr/bin/codex".to_string(),
            proxy: ProxyConfig::default(),
            servers: vec![],
            frontend_url: None,
            default_model: None,
            default_codex_model: None,
            use_tmux,
            home_dir: "/tmp/home".to_string(),
            codex_home_dir: "/tmp/home/.codex".to_string(),
            claude_config_dir: "/tmp/home/.claude".to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repo_config_raw: None,
            setup_commands: vec![],
            plugin_ids: vec![],
            plugin_repos: vec![],
        }
    }

    fn test_request() -> AgentRunRequest {
        AgentRunRequest {
            content: Vec::new(),
            model: None,
            system_prompt: None,
            resume: None,
            agent_type: None,
            agent_mode: None,
        }
    }

    fn build_test_options(use_tmux: bool) -> ClaudeOptions {
        build_options(
            &test_config(use_tmux),
            &test_request(),
            CancellationToken::new(),
            Arc::new(AgentState::new()),
            None,
        )
    }

    #[test]
    fn disallowed_tools_subprocess_mode_blocks_monitor() {
        let options = build_test_options(false);
        let disallowed = options.disallowed_tools.expect("disallowed_tools set");
        assert!(disallowed.contains(&"Monitor".to_string()));
        assert!(disallowed.contains(&"AskUserQuestion".to_string()));
    }

    #[test]
    fn disallowed_tools_tmux_mode_allows_monitor() {
        let options = build_test_options(true);
        let disallowed = options.disallowed_tools.expect("disallowed_tools set");
        assert!(!disallowed.contains(&"Monitor".to_string()));
        assert!(disallowed.contains(&"AskUserQuestion".to_string()));
    }

    #[test]
    fn tmux_mode_sets_is_sandbox_env() {
        // The TUI refuses --dangerously-skip-permissions under root unless
        // IS_SANDBOX=1 — tmux mode must always set it for the child.
        let options = build_test_options(true);
        let env = options.env.expect("env set in tmux mode");
        assert_eq!(env.get("IS_SANDBOX").map(String::as_str), Some("1"));
    }

    #[test]
    fn subprocess_mode_leaves_env_untouched() {
        let options = build_test_options(false);
        assert!(options.env.is_none());
    }
}
