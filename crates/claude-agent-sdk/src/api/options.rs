use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::mcp::McpServerConfig;
use super::permissions::{PermissionHandler, PermissionMode};

/// All options for configuring a Claude CLI invocation.
#[derive(Default)]
pub struct ClaudeOptions {
    // -- Model --
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    /// Reasoning effort for the session (`--effort`). The CLI accepts `low`,
    /// `medium`, `high`, `xhigh` and `max`, and warns-and-ignores anything
    /// else.
    pub effort: Option<String>,

    // -- System prompt --
    pub system_prompt: Option<SystemPrompt>,

    // -- Permissions --
    pub permission_mode: Option<PermissionMode>,
    pub allow_dangerously_skip_permissions: Option<bool>,
    pub permission_prompt_tool_name: Option<String>,

    // -- Tools --
    pub tools: Option<ToolsSpec>,
    pub allowed_tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,

    // -- Limits --
    pub max_turns: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub max_thinking_tokens: Option<i64>,

    // -- Timeouts --
    /// Timeout for the initialize control request. Default: 60s.
    pub init_timeout: Option<std::time::Duration>,

    // -- Session --
    pub resume: Option<String>,
    pub resume_session_at: Option<String>,
    pub continue_conversation: Option<bool>,
    pub fork_session: Option<bool>,
    pub persist_session: Option<bool>,

    // -- Directories --
    pub cwd: Option<String>,
    /// Claude's config directory, where the CLI writes its transcripts; pinned
    /// onto the CLI as `CLAUDE_CONFIG_DIR`. `$HOME/.claude` when unset.
    pub config_dir: Option<String>,
    pub additional_directories: Option<Vec<String>>,

    // -- MCP & Plugins --
    pub mcp_servers: Option<HashMap<String, McpServerConfig>>,
    pub plugins: Option<Vec<PluginConfig>>,
    pub strict_mcp_config: Option<bool>,

    // -- Agents --
    pub agent: Option<String>,

    // -- Settings --
    pub setting_sources: Option<Vec<SettingSource>>,
    pub sandbox: Option<SandboxSettings>,

    // -- Streaming --
    pub include_partial_messages: Option<bool>,
    pub betas: Option<Vec<String>>,

    // -- Output --
    pub output_format: Option<OutputFormat>,

    // -- Checkpointing --
    pub enable_file_checkpointing: Option<bool>,

    // -- Process --
    pub cli_path: Option<String>,
    pub env: Option<HashMap<String, String>>,
    /// A shell command whose output the CLI sends as its API key, re-run on a
    /// `401`: the `apiKeyHelper` setting.
    pub api_key_helper: Option<String>,
    pub extra_args: Option<HashMap<String, Option<String>>>,

    // -- Callbacks (runtime only, not serializable) --
    pub stderr_callback: Option<Box<dyn Fn(String) + Send + Sync>>,
    pub can_use_tool: Option<Arc<dyn PermissionHandler>>,
    pub cancellation_token: Option<CancellationToken>,
}

impl ClaudeOptions {
    /// The inline `--settings` JSON: `base`, a harness-owned settings object,
    /// plus the settings these options carry. `None` when both are empty.
    pub(crate) fn settings_json(&self, base: Option<&str>) -> Option<String> {
        let Some(helper) = &self.api_key_helper else {
            return base.map(str::to_string);
        };
        let mut settings = base
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        settings["apiKeyHelper"] = serde_json::Value::String(helper.clone());
        Some(settings.to_string())
    }
}

/// System prompt configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
    /// Replace the default system prompt entirely.
    Replace(String),
    /// Use a preset with optional appended text.
    Preset {
        #[serde(rename = "type")]
        prompt_type: String,
        preset: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        append: Option<String>,
    },
}

/// Tool specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolsSpec {
    /// List of tool names.
    Names(Vec<String>),
    /// Preset tool configuration.
    Preset {
        #[serde(rename = "type")]
        tools_type: String,
        preset: String,
    },
}

/// Setting source for configuration loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingSource {
    User,
    Project,
    Local,
}

impl std::fmt::Display for SettingSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Project => write!(f, "project"),
            Self::Local => write!(f, "local"),
        }
    }
}

/// Plugin configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(rename = "type")]
    pub plugin_type: String,
    pub path: String,
}

/// Sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_allow_bash_if_sandboxed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_unsandboxed_commands: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxNetworkConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_violations: Option<HashMap<String, Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_weaker_nested_sandbox: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_commands: Option<Vec<String>>,
}

/// Network configuration within a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxNetworkConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_unix_sockets: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_all_unix_sockets: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_local_binding: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_proxy_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socks_proxy_port: Option<u16>,
}

/// Output format specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputFormat {
    JsonSchema { schema: Value },
}

/// Plugin entry as reported in system init messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub path: String,
}
