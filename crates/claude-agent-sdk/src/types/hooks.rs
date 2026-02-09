use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// All hook event types supported by the Claude CLI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Notification,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Stop,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PermissionRequest,
    Setup,
}

/// Handler trait for hook callbacks.
#[async_trait]
pub trait HookHandler: Send + Sync {
    async fn on_hook(&self, input: &Value, tool_use_id: Option<&str>) -> HookOutput;
}

/// Matcher that associates hook callbacks with patterns.
pub struct HookCallbackMatcher {
    /// Regex pattern to match against tool names or other identifiers.
    pub matcher: Option<String>,
    /// Handler implementations to invoke.
    pub handlers: Vec<Arc<dyn HookHandler>>,
    /// Timeout in seconds for the hook callback.
    pub timeout: Option<u64>,
}

/// Output from a hook handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookOutput {
    Async(AsyncHookOutput),
    Sync(SyncHookOutput),
}

/// Asynchronous hook output — tells the CLI to continue and notify later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncHookOutput {
    /// Must be `true` for async hooks.
    #[serde(rename = "async")]
    pub is_async: bool,
    /// Timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub async_timeout: Option<u64>,
}

/// Synchronous hook output — immediate decision.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHookOutput {
    /// Whether to continue execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "continue")]
    pub should_continue: Option<bool>,

    /// Whether to suppress hook output in CLI display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,

    /// Reason for stopping (if continue is false).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,

    /// Permission decision: approve or block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,

    /// System message to inject.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,

    /// Reason for the decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Hook-specific output data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<Value>,
}
