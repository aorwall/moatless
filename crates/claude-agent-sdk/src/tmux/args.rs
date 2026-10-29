//! Build the `claude` CLI argument list for tmux (TUI) mode.
//!
//! Mirrors `transport::subprocess::build_cli_args` but drops flags that only
//! work with `--print` (`--output-format`, `--input-format`,
//! `--fallback-model`, `--max-budget-usd`, etc.) and always adds
//! `--dangerously-skip-permissions`. The `PermissionHandler` is ignored in tmux
//! mode.

use tracing::warn;

use crate::api::options::{ClaudeOptions, SystemPrompt};

/// `settings_json`, from [`crate::tmux::hooks::HookHarness`], adds `--settings
/// <json>` registering `SessionStart`/Stop hooks for the supervisor's FIFO,
/// merged with the settings `options` carry.
pub(crate) fn build_tmux_cli_args(
    options: &ClaudeOptions,
    settings_json: Option<&str>,
) -> Vec<String> {
    warn_ignored_options(options);

    let mut args = TmuxArgs::new();
    args.push_base(options.settings_json(settings_json).as_deref());
    args.push_session(options);
    args.push_tooling(options);
    args.push_prompts(options);
    args.push_mcp(options);
    args.push_agents(options);
    args.push_extra(options);
    args.into_vec()
}

fn warn_ignored_options(options: &ClaudeOptions) {
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
}

struct TmuxArgs {
    args: Vec<String>,
}

impl TmuxArgs {
    fn new() -> Self {
        Self {
            args: Vec::with_capacity(24),
        }
    }

    fn into_vec(self) -> Vec<String> {
        self.args
    }

    fn push_flag(&mut self, flag: &str) {
        self.args.push(flag.to_string());
    }

    fn push_pair(&mut self, flag: &str, value: impl Into<String>) {
        self.push_flag(flag);
        self.args.push(value.into());
    }

    fn push_optional_pair(&mut self, flag: &str, value: &Option<String>) {
        if let Some(value) = value {
            self.push_pair(flag, value.clone());
        }
    }

    fn push_joined<T: ToString>(&mut self, flag: &str, values: Option<&[T]>) {
        if let Some(values) = values {
            if !values.is_empty() {
                self.push_pair(
                    flag,
                    values
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
        }
    }

    fn push_json<T: serde::Serialize>(&mut self, flag: &str, value: &T) {
        if let Ok(json) = serde_json::to_string(value) {
            self.push_pair(flag, json);
        }
    }

    fn push_base(&mut self, settings_json: Option<&str>) {
        // Always bypass permissions in tmux mode — there is no SDK-side
        // permission handler.
        self.push_flag("--dangerously-skip-permissions");
        // `--verbose` is accepted in interactive mode (it just overrides config).
        self.push_flag("--verbose");

        if let Some(json) = settings_json {
            self.push_pair("--settings", json);
        }
    }

    fn push_session(&mut self, options: &ClaudeOptions) {
        self.push_optional_pair("--model", &options.model);
        self.push_optional_pair("--effort", &options.effort);
        self.push_optional_pair("--resume", &options.resume);

        if options.continue_conversation == Some(true) {
            self.push_flag("--continue");
        }
        if options.fork_session == Some(true) {
            self.push_flag("--fork-session");
        }
        if let Some(turns) = options.max_turns {
            self.push_pair("--max-turns", turns.to_string());
        }
    }

    fn push_tooling(&mut self, options: &ClaudeOptions) {
        self.push_joined("--allowedTools", options.allowed_tools.as_deref());
        self.push_joined("--disallowedTools", options.disallowed_tools.as_deref());

        if let Some(dirs) = &options.additional_directories {
            for dir in dirs {
                self.push_pair("--add-dir", dir.clone());
            }
        }

        self.push_joined("--setting-sources", options.setting_sources.as_deref());

        if let Some(plugins) = &options.plugins {
            for plugin in plugins {
                self.push_pair("--plugin-dir", plugin.path.clone());
            }
        }
    }

    fn push_prompts(&mut self, options: &ClaudeOptions) {
        if let Some(prompt) = &options.system_prompt {
            match prompt {
                SystemPrompt::Replace(text) => self.push_pair("--system-prompt", text.clone()),
                SystemPrompt::Preset {
                    append: Some(text), ..
                } => self.push_pair("--append-system-prompt", text.clone()),
                SystemPrompt::Preset { append: None, .. } => {}
            }
        }
    }

    fn push_mcp(&mut self, options: &ClaudeOptions) {
        if let Some(servers) = options
            .mcp_servers
            .as_ref()
            .filter(|servers| !servers.is_empty())
        {
            let wrapper = serde_json::json!({ "mcpServers": servers });
            self.push_json("--mcp-config", &wrapper);
        }

        if options.strict_mcp_config == Some(true) {
            self.push_flag("--strict-mcp-config");
        }
    }

    fn push_agents(&mut self, options: &ClaudeOptions) {
        self.push_joined("--betas", options.betas.as_deref());
        self.push_optional_pair("--agent", &options.agent);

        if let Some(agents) = options.agents.as_ref().filter(|agents| !agents.is_empty()) {
            self.push_json("--agents", agents);
        }
    }

    fn push_extra(&mut self, options: &ClaudeOptions) {
        if let Some(extra) = &options.extra_args {
            for (key, value) in extra {
                self.push_flag(&format!("--{key}"));
                if let Some(value) = value {
                    self.args.push(value.clone());
                }
            }
        }
    }
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

    /// Unlike `permission_mode`, effort is honoured in tmux mode: `--effort` is
    /// a launch flag the TUI takes just as `--print` does.
    #[test]
    fn includes_effort() {
        let options = ClaudeOptions {
            effort: Some("max".to_string()),
            ..Default::default()
        };
        let args = build_tmux_cli_args(&options, None);
        let idx = args.iter().position(|a| a == "--effort").unwrap();
        assert_eq!(args[idx + 1], "max");
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
    fn an_api_key_helper_joins_the_hook_settings() {
        let options = ClaudeOptions {
            api_key_helper: Some("cat /run/token".to_string()),
            ..Default::default()
        };
        let args = build_tmux_cli_args(&options, Some(r#"{"hooks":{}}"#));
        let idx = args.iter().position(|a| a == "--settings").unwrap();
        let settings: serde_json::Value = serde_json::from_str(&args[idx + 1]).unwrap();
        assert_eq!(
            settings,
            serde_json::json!({ "hooks": {}, "apiKeyHelper": "cat /run/token" })
        );
        assert_eq!(args.iter().filter(|a| *a == "--settings").count(), 1);
    }

    #[test]
    fn settings_json_omitted_when_none() {
        let args = build_tmux_cli_args(&ClaudeOptions::default(), None);
        assert!(!args.iter().any(|a| a == "--settings"));
    }
}
