use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::agents::AgentDefinition;
use super::hooks::{HookDefinition, HookHandler};
use super::mcp::McpServerConfig;
use super::permissions::{PermissionHandler, PermissionMode};
use crate::mcp::SdkMcpServer;

/// All options for configuring a Claude CLI invocation.
#[derive(Default)]
pub struct ClaudeOptions {
    // -- Model --
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    /// Reasoning effort for the session (`--effort`). The CLI accepts `low`,
    /// `medium`, `high`, `xhigh` and `max`, and warns-and-ignores anything else.
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
    pub additional_directories: Option<Vec<String>>,

    // -- MCP & Plugins --
    pub mcp_servers: Option<HashMap<String, McpServerConfig>>,
    /// In-process SDK MCP servers. These are handled locally by the SDK
    /// without spawning separate processes. Keys are server names used
    /// in tool naming: `mcp__<server_name>__<tool_name>`.
    pub sdk_mcp_servers: Option<HashMap<String, SdkMcpServer>>,
    pub plugins: Option<Vec<PluginConfig>>,
    pub strict_mcp_config: Option<bool>,

    // -- Agents --
    pub agents: Option<HashMap<String, AgentDefinition>>,
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
    pub extra_args: Option<HashMap<String, Option<String>>>,

    // -- Hooks --
    /// Hook definitions to register with the CLI in the Initialize request.
    /// These tell the CLI which events should trigger `HookCallback` requests.
    pub hooks: Option<Vec<HookDefinition>>,

    // -- Callbacks (runtime only, not serializable) --
    pub stderr_callback: Option<Box<dyn Fn(String) + Send + Sync>>,
    pub can_use_tool: Option<Arc<dyn PermissionHandler>>,
    /// Handler for hook callbacks from the CLI.
    /// When the CLI sends a `HookCallback` control request, this handler is invoked
    /// to determine the response. If `None`, hook callbacks are auto-responded
    /// with an empty success (default behavior — continue with no modifications).
    pub hook_handler: Option<Arc<dyn HookHandler>>,
    pub cancellation_token: Option<CancellationToken>,
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
