//! Build the `claude` CLI argument list for tmux (TUI) mode.
//!
//! Mirrors `transport::subprocess::build_cli_args` but drops flags that only
//! work with `--print` (`--output-format`, `--input-format`,
//! `--fallback-model`, `--max-budget-usd`, etc.) and always adds
//! `--dangerously-skip-permissions`. The `PermissionHandler` is ignored in
//! tmux mode.

use tracing::warn;

use crate::types::options::{ClaudeOptions, SystemPrompt};

/// Build CLI args appropriate for running `claude` inside a tmux session
/// (interactive TUI, no stream-json plumbing).
///
/// `settings_json` is the inline JSON produced by [`crate::tmux::hooks::HookHarness`].
/// When `Some`, an `--settings <json>` pair is added so claude registers the
/// SessionStart + Stop hooks for the supervisor's FIFO. The JSON is harness-
/// owned, not caller-supplied (no field on [`ClaudeOptions`] for it).
pub(crate) fn build_tmux_cli_args(
    options: &ClaudeOptions,
    settings_json: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::with_capacity(24);

    // Always bypass permissions in tmux mode — there is no SDK-side
    // permission handler.
    args.push("--dangerously-skip-permissions".to_string());

    // `--verbose` is accepted in interactive mode (it just overrides config).
    args.push("--verbose".to_string());

    if let Some(json) = settings_json {
        args.push("--settings".to_string());
        args.push(json.to_string());
    }

    if options.can_use_tool.is_some() {
        warn!(
            "tmux mode ignores ClaudeOptions.can_use_tool — \
             permission interception is not supported in TUI mode"
        );
    }
    if options.sdk_mcp_servers.is_some() {
        warn!(
            "tmux mode ignores ClaudeOptions.sdk_mcp_servers — \
             in-process MCP servers are not supported in TUI mode"
        );
    }
    if options.permission_prompt_tool_name.is_some() {
        warn!(
            "tmux mode ignores ClaudeOptions.permission_prompt_tool_name \
             (always uses --dangerously-skip-permissions)"
        );
    }
    if options.permission_mode.is_some() {
        warn!(
            "tmux mode ignores ClaudeOptions.permission_mode \
             (always uses --dangerously-skip-permissions)"
        );
    }
    if options.fallback_model.is_some() {
        warn!("tmux mode ignores fallback_model (only works with --print)");
    }
    if options.max_budget_usd.is_some() {
        warn!("tmux mode ignores max_budget_usd (only works with --print)");
    }
    if options.include_partial_messages == Some(true) {
        warn!("tmux mode ignores include_partial_messages (only works with --print)");
    }
    if options.output_format.is_some() {
        warn!("tmux mode ignores output_format / json-schema (only works with --print)");
    }
    if options.persist_session == Some(false) {
        warn!("tmux mode ignores persist_session=false (only works with --print)");
    }

    if let Some(model) = &options.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }

    // Session resume
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

    if let Some(turns) = options.max_turns {
        args.push("--max-turns".to_string());
        args.push(turns.to_string());
    }

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

    if let Some(dirs) = &options.additional_directories {
        for dir in dirs {
            args.push("--add-dir".to_string());
            args.push(dir.clone());
        }
    }

    if let Some(sources) = &options.setting_sources {
        if !sources.is_empty() {
            args.push("--setting-sources".to_string());
            args.push(
                sources
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
    }

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

    if let Some(plugins) = &options.plugins {
        for plugin in plugins {
            args.push("--plugin-dir".to_string());
            args.push(plugin.path.clone());
        }
    }

    if let Some(servers) = &options.mcp_servers {
        if !servers.is_empty() {
            let wrapper = serde_json::json!({ "mcpServers": servers });
            if let Ok(json) = serde_json::to_string(&wrapper) {
                args.push("--mcp-config".to_string());
                args.push(json);
            }
        }
    }
    if options.strict_mcp_config == Some(true) {
        args.push("--strict-mcp-config".to_string());
    }

    if let Some(betas) = &options.betas {
        if !betas.is_empty() {
            args.push("--betas".to_string());
            args.push(betas.join(","));
        }
    }

    if let Some(agent) = &options.agent {
        args.push("--agent".to_string());
        args.push(agent.clone());
    }
    if let Some(agents) = &options.agents {
        if !agents.is_empty() {
            if let Ok(json) = serde_json::to_string(agents) {
                args.push("--agents".to_string());
                args.push(json);
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_skip_permissions_and_verbose() {
        let args = build_tmux_cli_args(&ClaudeOptions::default(), None);
        assert!(args.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn never_includes_print_or_streaming_flags() {
        let options = ClaudeOptions {
            include_partial_messages: Some(true),
            fallback_model: Some("claude-haiku".to_string()),
            max_budget_usd: Some(1.0),
            persist_session: Some(false),
            ..Default::default()
        };
        let args = build_tmux_cli_args(&options, None);
        assert!(!args.contains(&"--print".to_string()));
        assert!(!args.contains(&"--output-format".to_string()));
        assert!(!args.contains(&"--input-format".to_string()));
        assert!(!args.contains(&"--fallback-model".to_string()));
        assert!(!args.contains(&"--max-budget-usd".to_string()));
        assert!(!args.contains(&"--include-partial-messages".to_string()));
        assert!(!args.contains(&"--no-persist-session".to_string()));
    }

    #[test]
    fn includes_resume_continue_model() {
        let options = ClaudeOptions {
            resume: Some("sess-xyz".to_string()),
            continue_conversation: Some(true),
            model: Some("claude-sonnet-4-5".to_string()),
            ..Default::default()
        };
        let args = build_tmux_cli_args(&options, None);
        let resume_idx = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[resume_idx + 1], "sess-xyz");
        assert!(args.contains(&"--continue".to_string()));
        let model_idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[model_idx + 1], "claude-sonnet-4-5");
    }

    #[test]
    fn append_system_prompt_passes_through() {
        let options = ClaudeOptions {
            system_prompt: Some(SystemPrompt::Preset {
                prompt_type: "preset".to_string(),
                preset: "claude_code".to_string(),
                append: Some("Always be brief.".to_string()),
            }),
            ..Default::default()
        };
        let args = build_tmux_cli_args(&options, None);
        let idx = args
            .iter()
            .position(|a| a == "--append-system-prompt")
            .unwrap();
        assert_eq!(args[idx + 1], "Always be brief.");
    }

    #[test]
    fn settings_json_threaded_after_skip_permissions() {
        let args = build_tmux_cli_args(&ClaudeOptions::default(), Some(r#"{"hooks":{}}"#));
        let idx = args
            .iter()
            .position(|a| a == "--settings")
            .expect("--settings present");
        assert_eq!(args[idx + 1], r#"{"hooks":{}}"#);
        // --settings appears after --dangerously-skip-permissions and --verbose.
        let skip_idx = args
            .iter()
            .position(|a| a == "--dangerously-skip-permissions")
            .unwrap();
        assert!(skip_idx < idx);
    }

    #[test]
    fn settings_json_omitted_when_none() {
        let args = build_tmux_cli_args(&ClaudeOptions::default(), None);
        assert!(!args.iter().any(|a| a == "--settings"));
    }
}
