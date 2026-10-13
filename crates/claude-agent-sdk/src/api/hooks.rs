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
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreToolUse => write!(f, "PreToolUse"),
            Self::PostToolUse => write!(f, "PostToolUse"),
            Self::PostToolUseFailure => write!(f, "PostToolUseFailure"),
            Self::Notification => write!(f, "Notification"),
            Self::UserPromptSubmit => write!(f, "UserPromptSubmit"),
            Self::SessionStart => write!(f, "SessionStart"),
            Self::SessionEnd => write!(f, "SessionEnd"),
            Self::Stop => write!(f, "Stop"),
            Self::SubagentStart => write!(f, "SubagentStart"),
            Self::SubagentStop => write!(f, "SubagentStop"),
            Self::PreCompact => write!(f, "PreCompact"),
            Self::PermissionRequest => write!(f, "PermissionRequest"),
            Self::Setup => write!(f, "Setup"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Handler trait for hook callbacks from the CLI.
///
/// The CLI sends `HookCallback` control requests for registered hook events;
/// the SDK invokes this to determine the response.
#[async_trait]
pub trait HookHandler: Send + Sync {
    /// `input` carries event-specific data (`hookEventName`, `toolName`,
    /// `toolInput`); `tool_use_id` is set when the hook is tied to a tool
    /// invocation. Returns a `HookOutput` controlling continue/stop,
    /// approve/deny, and injected messages.
    async fn on_hook(&self, input: &Value, tool_use_id: Option<&str>) -> HookOutput;
}

/// A hook definition that tells the CLI which events to send as callbacks.
///
/// Serialized into the Initialize control request; a matching event triggers a
/// `HookCallback` control request back to the SDK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    /// The hook event type to listen for.
    #[serde(rename = "type")]
    pub event: HookEvent,
    /// Optional regex pattern to match against tool names (for
    /// PreToolUse/PostToolUse). If `None`, matches all tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// Timeout in seconds for the hook callback. If the handler doesn't respond
    /// within this time, the CLI will proceed with the default behavior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_event_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&HookEvent::PreToolUse).unwrap(),
            "\"PreToolUse\""
        );
        assert_eq!(
            serde_json::to_string(&HookEvent::PostToolUse).unwrap(),
            "\"PostToolUse\""
        );
        assert_eq!(serde_json::to_string(&HookEvent::Stop).unwrap(), "\"Stop\"");
    }

    #[test]
    fn hook_definition_serializes_for_initialize() {
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: Some("Bash".to_string()),
            timeout: Some(10),
        };

        let json = serde_json::to_value(&def).unwrap();
        assert_eq!(json["type"], "PreToolUse");
        assert_eq!(json["matcher"], "Bash");
        assert_eq!(json["timeout"], 10);
    }

    #[test]
    fn hook_definition_omits_optional_fields() {
        let def = HookDefinition {
            event: HookEvent::PostToolUse,
            matcher: None,
            timeout: None,
        };

        let json = serde_json::to_value(&def).unwrap();
        assert_eq!(json["type"], "PostToolUse");
        assert!(json.get("matcher").is_none());
        assert!(json.get("timeout").is_none());
    }

    #[test]
    fn hook_definitions_serialize_as_array() {
        let defs = vec![
            HookDefinition {
                event: HookEvent::PreToolUse,
                matcher: Some(".*".to_string()),
                timeout: None,
            },
            HookDefinition {
                event: HookEvent::PostToolUse,
                matcher: None,
                timeout: Some(5),
            },
        ];

        let json = serde_json::to_value(&defs).unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
        assert_eq!(json[0]["type"], "PreToolUse");
        assert_eq!(json[1]["type"], "PostToolUse");
    }

    #[test]
    fn sync_hook_output_serializes_with_camel_case() {
        let output = SyncHookOutput {
            should_continue: Some(true),
            suppress_output: None,
            stop_reason: None,
            decision: Some("approve".to_string()),
            system_message: Some("Check permissions".to_string()),
            reason: None,
            hook_specific_output: None,
        };

        let json = serde_json::to_value(&output).unwrap();
        // `should_continue` → serialized as `continue`
        assert_eq!(json["continue"], true);
        assert_eq!(json["decision"], "approve");
        assert_eq!(json["systemMessage"], "Check permissions");
        assert!(json.get("suppressOutput").is_none());
        assert!(json.get("stopReason").is_none());
    }

    #[test]
    fn sync_hook_output_default_is_empty() {
        let output = SyncHookOutput::default();
        let json = serde_json::to_value(&output).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.is_empty());
    }

    #[test]
    fn async_hook_output_serializes_correctly() {
        let output = AsyncHookOutput {
            is_async: true,
            async_timeout: Some(30),
        };

        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["async"], true);
        assert_eq!(json["async_timeout"], 30);
    }

    #[test]
    fn hook_output_sync_variant() {
        let output = HookOutput::Sync(SyncHookOutput {
            should_continue: Some(false),
            stop_reason: Some("Blocked".to_string()),
            ..Default::default()
        });

        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["continue"], false);
        assert_eq!(json["stopReason"], "Blocked");
    }

    #[test]
    fn hook_output_async_variant() {
        let output = HookOutput::Async(AsyncHookOutput {
            is_async: true,
            async_timeout: None,
        });

        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["async"], true);
    }
}
