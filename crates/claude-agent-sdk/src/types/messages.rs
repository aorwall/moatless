use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;

use super::content::{ApiMessage, ModelUsage, PermissionDenial, Usage};
use super::hooks::HookEvent;
use super::mcp::McpServerEntry;
use super::options::PluginEntry;
use super::permissions::PermissionMode;

/// Attempt to deserialize a JSON value, logging a warning on failure instead of
/// silently discarding the error (which `.ok()` would do).
fn try_deser<T: DeserializeOwned>(value: Value, label: &str) -> Option<T> {
    match serde_json::from_value::<T>(value) {
        Ok(v) => Some(v),
        Err(e) => {
            warn!(
                error = %e,
                message_type = label,
                "Failed to deserialize SDK message"
            );
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStyle {
    Normal,
    Concise,
    Verbose,
    Silent,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for OutputStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "normal"),
            Self::Concise => write!(f, "concise"),
            Self::Verbose => write!(f, "verbose"),
            Self::Silent => write!(f, "silent"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusValue {
    Idle,
    Busy,
    Thinking,
    Processing,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for AgentStatusValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Busy => write!(f, "busy"),
            Self::Thinking => write!(f, "thinking"),
            Self::Processing => write!(f, "processing"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactTrigger {
    Auto,
    Manual,
    MaxTokens,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for CompactTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Manual => write!(f, "manual"),
            Self::MaxTokens => write!(f, "max_tokens"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookOutcome {
    Success,
    Error,
    Timeout,
    Skipped,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for HookOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::Error => write!(f, "error"),
            Self::Timeout => write!(f, "timeout"),
            Self::Skipped => write!(f, "skipped"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskNotificationStatus {
    Started,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for TaskNotificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Started => write!(f, "started"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// The core SDK message type — a union of all 16 message variants
/// that the Claude CLI can emit over NDJSON stdout.
///
/// Custom deserialization is required because:
/// - `"system"` type has 8 subtypes dispatched via `subtype` field
/// - `"result"` has success vs error variants dispatched via `subtype`
/// - Some types like `"user"` branch on `isReplay`
#[derive(Debug, Clone)]
pub enum SdkMessage {
    /// type: "assistant" — a complete assistant turn
    Assistant(AssistantMessage),
    /// type: "user" — a user message (without replay)
    User(UserMessage),
    /// type: "user" with isReplay: true
    UserReplay(UserMessageReplay),
    /// type: "result" — final result (success or error)
    Result(ResultMessage),
    /// type: "system", subtype: "init"
    System(SystemInitMessage),
    /// type: "system", subtype: "compact_boundary"
    CompactBoundary(CompactBoundaryMessage),
    /// type: "system", subtype: "status"
    Status(StatusMessage),
    /// type: "system", subtype: "hook_started"
    HookStarted(HookStartedMessage),
    /// type: "system", subtype: "hook_progress"
    HookProgress(HookProgressMessage),
    /// type: "system", subtype: "hook_response"
    HookResponse(HookResponseMessage),
    /// type: "system", subtype: "task_notification"
    TaskNotification(TaskNotificationMessage),
    /// type: "system", subtype: "files_persisted"
    FilesPersisted(FilesPersistedMessage),
    /// type: "stream_event" — partial assistant message (streaming)
    StreamEvent(PartialAssistantMessage),
    /// type: "tool_progress"
    ToolProgress(ToolProgressMessage),
    /// type: "auth_status"
    AuthStatus(AuthStatusMessage),
    /// type: "tool_use_summary"
    ToolUseSummary(ToolUseSummaryMessage),
}

impl SdkMessage {
    /// Parse a raw JSON value into an `SdkMessage`, returning `None` if unrecognized.
    pub fn from_value(value: Value) -> Option<Self> {
        let msg_type = value.get("type")?.as_str()?;

        match msg_type {
            "assistant" => {
                try_deser::<AssistantMessage>(value, "assistant").map(SdkMessage::Assistant)
            }
            "user" => {
                let is_replay = value
                    .get("isReplay")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if is_replay {
                    try_deser::<UserMessageReplay>(value, "user/replay")
                        .map(SdkMessage::UserReplay)
                } else {
                    try_deser::<UserMessage>(value, "user").map(SdkMessage::User)
                }
            }
            "result" => {
                let subtype = value.get("subtype").and_then(|v| v.as_str());
                match subtype {
                    Some("success") => try_deser::<ResultSuccess>(value, "result/success")
                        .map(|r| SdkMessage::Result(ResultMessage::Success(r))),
                    _ => try_deser::<ResultError>(value, "result/error")
                        .map(|r| SdkMessage::Result(ResultMessage::Error(r))),
                }
            }
            "system" => {
                let subtype = value.get("subtype").and_then(|v| v.as_str())?;
                match subtype {
                    "init" => {
                        try_deser::<SystemInitMessage>(value, "system/init")
                            .map(SdkMessage::System)
                    }
                    "compact_boundary" => {
                        try_deser::<CompactBoundaryMessage>(value, "system/compact_boundary")
                            .map(SdkMessage::CompactBoundary)
                    }
                    "status" => {
                        try_deser::<StatusMessage>(value, "system/status")
                            .map(SdkMessage::Status)
                    }
                    "hook_started" => {
                        try_deser::<HookStartedMessage>(value, "system/hook_started")
                            .map(SdkMessage::HookStarted)
                    }
                    "hook_progress" => {
                        try_deser::<HookProgressMessage>(value, "system/hook_progress")
                            .map(SdkMessage::HookProgress)
                    }
                    "hook_response" => {
                        try_deser::<HookResponseMessage>(value, "system/hook_response")
                            .map(SdkMessage::HookResponse)
                    }
                    "task_notification" => {
                        try_deser::<TaskNotificationMessage>(value, "system/task_notification")
                            .map(SdkMessage::TaskNotification)
                    }
                    "files_persisted" => {
                        try_deser::<FilesPersistedMessage>(value, "system/files_persisted")
                            .map(SdkMessage::FilesPersisted)
                    }
                    _ => None,
                }
            }
            "stream_event" => {
                try_deser::<PartialAssistantMessage>(value, "stream_event")
                    .map(SdkMessage::StreamEvent)
            }
            "tool_progress" => {
                try_deser::<ToolProgressMessage>(value, "tool_progress")
                    .map(SdkMessage::ToolProgress)
            }
            "auth_status" => {
                try_deser::<AuthStatusMessage>(value, "auth_status")
                    .map(SdkMessage::AuthStatus)
            }
            "tool_use_summary" => {
                try_deser::<ToolUseSummaryMessage>(value, "tool_use_summary")
                    .map(SdkMessage::ToolUseSummary)
            }
            _ => None,
        }
    }
}

// ---------- Assistant ----------

/// A complete assistant turn message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssistantMessage {
    pub uuid: String,
    pub session_id: String,
    pub message: ApiMessage,
    pub parent_tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AssistantMessageError>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isApiErrorMessage")]
    pub is_api_error_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "apiError")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub api_error: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "requestId")]
    pub request_id: Option<String>,
}

/// Error classification for assistant messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AssistantMessageError {
    AuthenticationFailed,
    BillingError,
    RateLimit,
    InvalidRequest,
    ServerError,
    Unknown,
}

// ---------- User ----------

/// Priority for injected user messages.
///
/// Controls when a mid-loop message gets processed by the CLI:
/// - `Now` — process immediately, even interrupting current tool execution.
/// - `Next` — queue for the next API round-trip (default).
/// - `Later` — wait for current turn to finish, then process as a new query.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessagePriority {
    Now,
    #[default]
    Next,
    Later,
}

/// A user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    pub session_id: String,
    pub message: ApiMessage,
    pub parent_tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isSynthetic")]
    pub is_synthetic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "toolUseResult")]
    pub tool_use_result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isMeta")]
    pub is_meta: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isCompactSummary")]
    pub is_compact_summary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isVisibleInTranscriptOnly")]
    pub is_visible_in_transcript_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "planContent")]
    pub plan_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "thinkingMetadata")]
    pub thinking_metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "summarizeMetadata")]
    pub summarize_metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sourceToolAssistantUUID")]
    pub source_tool_assistant_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sourceToolUseID")]
    pub source_tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "permissionMode")]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub priority: MessagePriority,
}

/// A replayed user message (from session resume).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessageReplay {
    pub uuid: String,
    pub session_id: String,
    pub message: ApiMessage,
    pub parent_tool_use_id: Option<String>,
    #[serde(rename = "isReplay")]
    pub is_replay: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isSynthetic")]
    pub is_synthetic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "toolUseResult")]
    pub tool_use_result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isMeta")]
    pub is_meta: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isCompactSummary")]
    pub is_compact_summary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isVisibleInTranscriptOnly")]
    pub is_visible_in_transcript_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "planContent")]
    pub plan_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "thinkingMetadata")]
    pub thinking_metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "summarizeMetadata")]
    pub summarize_metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sourceToolAssistantUUID")]
    pub source_tool_assistant_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sourceToolUseID")]
    pub source_tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "permissionMode")]
    pub permission_mode: Option<String>,
}

// ---------- Result ----------

/// The final result message — either success or error.
#[derive(Debug, Clone)]
pub enum ResultMessage {
    Success(ResultSuccess),
    Error(ResultError),
}

/// Successful result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResultSuccess {
    pub uuid: String,
    pub session_id: String,
    pub duration_ms: u64,
    pub duration_api_ms: u64,
    pub is_error: bool,
    pub num_turns: u32,
    pub result: String,
    pub total_cost_usd: f64,
    pub usage: Usage,
    #[serde(rename = "modelUsage")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub model_usage: HashMap<String, ModelUsage>,
    pub permission_denials: Vec<PermissionDenial>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub structured_output: Option<Value>,
}

/// Error result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResultError {
    pub subtype: String,
    pub uuid: String,
    pub session_id: String,
    pub duration_ms: u64,
    pub duration_api_ms: u64,
    pub is_error: bool,
    pub num_turns: u32,
    pub total_cost_usd: f64,
    pub usage: Usage,
    #[serde(rename = "modelUsage")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub model_usage: HashMap<String, ModelUsage>,
    pub permission_denials: Vec<PermissionDenial>,
    pub errors: Vec<String>,
}

// ---------- System Init ----------

/// API key source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeySource {
    /// No API key configured (e.g., using Max plan).
    None,
    User,
    Project,
    Org,
    Temporary,
}

/// System init message — sent once at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInitMessage {
    pub uuid: String,
    pub session_id: String,
    pub tools: Vec<String>,
    pub model: String,
    pub mcp_servers: Vec<McpServerEntry>,
    #[serde(rename = "permissionMode")]
    pub permission_mode: PermissionMode,
    pub cwd: String,
    #[serde(rename = "apiKeySource")]
    pub api_key_source: ApiKeySource,
    pub slash_commands: Vec<String>,
    pub output_style: OutputStyle,
    pub skills: Vec<String>,
    pub plugins: Vec<PluginEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub betas: Option<Vec<String>>,
    pub claude_code_version: String,
}

// ---------- System Subtypes ----------

/// Compact boundary message — marks a context compaction event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactBoundaryMessage {
    pub uuid: String,
    pub session_id: String,
    pub compact_metadata: CompactMetadata,
}

/// Metadata about a compaction event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactMetadata {
    pub trigger: CompactTrigger,
    pub pre_tokens: u64,
}

/// Status message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusMessage {
    pub uuid: String,
    pub session_id: String,
    pub status: Option<AgentStatusValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "permissionMode")]
    pub permission_mode: Option<PermissionMode>,
}

/// Hook started notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookStartedMessage {
    pub uuid: String,
    pub session_id: String,
    pub hook_id: String,
    pub hook_name: String,
    pub hook_event: HookEvent,
}

/// Hook progress update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookProgressMessage {
    pub uuid: String,
    pub session_id: String,
    pub hook_id: String,
    pub hook_name: String,
    pub hook_event: HookEvent,
    pub stdout: String,
    pub stderr: String,
    pub output: String,
}

/// Hook response (completion).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResponseMessage {
    pub uuid: String,
    pub session_id: String,
    pub hook_id: String,
    pub hook_name: String,
    pub hook_event: HookEvent,
    pub output: String,
    pub stdout: String,
    pub stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub outcome: HookOutcome,
}

/// Task notification from a subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNotificationMessage {
    pub uuid: String,
    pub session_id: String,
    pub task_id: String,
    pub status: TaskNotificationStatus,
    pub output_file: String,
    pub summary: String,
}

/// Files persisted event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesPersistedMessage {
    pub uuid: String,
    pub session_id: String,
    pub files: Vec<PersistedFile>,
    pub failed: Vec<FailedFile>,
    pub processed_at: String,
}

/// A successfully persisted file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedFile {
    pub filename: String,
    pub file_id: String,
}

/// A file that failed to persist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedFile {
    pub filename: String,
    pub error: String,
}

// ---------- Stream Event ----------

/// Partial assistant message (streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialAssistantMessage {
    pub uuid: String,
    pub session_id: String,
    pub event: Value,
    pub parent_tool_use_id: Option<String>,
}

// ---------- Tool Progress ----------

/// Tool progress update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolProgressMessage {
    pub uuid: String,
    pub session_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub parent_tool_use_id: Option<String>,
    pub elapsed_time_seconds: f64,
}

// ---------- Auth Status ----------

/// Authentication status update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatusMessage {
    pub uuid: String,
    pub session_id: String,
    #[serde(rename = "isAuthenticating")]
    pub is_authenticating: bool,
    pub output: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------- Tool Use Summary ----------

/// Tool use summary message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseSummaryMessage {
    pub uuid: String,
    pub session_id: String,
    pub summary: String,
    pub preceding_tool_use_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_assistant_message() {
        let json = serde_json::json!({
            "type": "assistant",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Hello!"}
                ]
            },
            "parent_tool_use_id": null
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::Assistant(_)));
    }

    #[test]
    fn parse_result_success() {
        let json = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "duration_ms": 1234,
            "duration_api_ms": 1000,
            "is_error": false,
            "num_turns": 3,
            "result": "Done!",
            "total_cost_usd": 0.05,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50
            },
            "modelUsage": {},
            "permission_denials": []
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::Result(ResultMessage::Success(_))));
    }

    #[test]
    fn parse_result_error() {
        let json = serde_json::json!({
            "type": "result",
            "subtype": "error_max_turns",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "duration_ms": 5000,
            "duration_api_ms": 4500,
            "is_error": true,
            "num_turns": 10,
            "total_cost_usd": 0.25,
            "usage": {
                "input_tokens": 500,
                "output_tokens": 200
            },
            "modelUsage": {},
            "permission_denials": [],
            "errors": ["Max turns exceeded"]
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::Result(ResultMessage::Error(_))));
    }

    #[test]
    fn parse_system_init() {
        let json = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "tools": ["Read", "Write", "Bash"],
            "model": "claude-sonnet-4-5-20250929",
            "mcp_servers": [],
            "permissionMode": "default",
            "cwd": "/tmp",
            "apiKeySource": "user",
            "slash_commands": [],
            "output_style": "normal",
            "skills": [],
            "plugins": [],
            "claude_code_version": "1.0.0"
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::System(_)));
    }

    #[test]
    fn parse_stream_event() {
        let json = serde_json::json!({
            "type": "stream_event",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hi"}},
            "parent_tool_use_id": null
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::StreamEvent(_)));
    }

    #[test]
    fn parse_tool_progress() {
        let json = serde_json::json!({
            "type": "tool_progress",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "tool_use_id": "tool-1",
            "tool_name": "Bash",
            "parent_tool_use_id": null,
            "elapsed_time_seconds": 2.5
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::ToolProgress(_)));
    }

    #[test]
    fn parse_auth_status() {
        let json = serde_json::json!({
            "type": "auth_status",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "isAuthenticating": true,
            "output": ["Authenticating..."]
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::AuthStatus(_)));
    }

    #[test]
    fn parse_hook_started() {
        let json = serde_json::json!({
            "type": "system",
            "subtype": "hook_started",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "hook_id": "hook-1",
            "hook_name": "pre-tool-use",
            "hook_event": "PreToolUse"
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::HookStarted(_)));
    }

    #[test]
    fn parse_compact_boundary() {
        let json = serde_json::json!({
            "type": "system",
            "subtype": "compact_boundary",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "compact_metadata": {
                "trigger": "auto",
                "pre_tokens": 50000
            }
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::CompactBoundary(_)));
    }

    #[test]
    fn parse_tool_use_summary() {
        let json = serde_json::json!({
            "type": "tool_use_summary",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "summary": "Created file foo.rs",
            "preceding_tool_use_ids": ["tool-1", "tool-2"]
        });

        let msg = SdkMessage::from_value(json).unwrap();
        assert!(matches!(msg, SdkMessage::ToolUseSummary(_)));
    }

    #[test]
    fn unknown_type_returns_none() {
        let json = serde_json::json!({
            "type": "unknown_future_type",
            "uuid": "abc-123"
        });

        assert!(SdkMessage::from_value(json).is_none());
    }

    #[test]
    fn parse_assistant_with_enriched_fields() {
        let json = serde_json::json!({
            "type": "assistant",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello!"}],
                "model": "claude-sonnet-4-5-20250929",
                "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 50
                }
            },
            "parent_tool_use_id": null,
            "isApiErrorMessage": true,
            "apiError": {"type": "overloaded", "message": "Overloaded"},
            "requestId": "req-abc-123"
        });

        let msg = SdkMessage::from_value(json).unwrap();
        if let SdkMessage::Assistant(a) = msg {
            assert_eq!(a.message.model.as_deref(), Some("claude-sonnet-4-5-20250929"));
            assert_eq!(a.message.id.as_deref(), Some("msg_01XFDUDYJgAACzvnptvVoYEL"));
            assert_eq!(a.message.stop_reason.as_deref(), Some("end_turn"));
            assert!(a.message.stop_sequence.is_none());
            assert_eq!(a.is_api_error_message, Some(true));
            assert!(a.api_error.is_some());
            assert_eq!(a.request_id.as_deref(), Some("req-abc-123"));
        } else {
            panic!("Expected SdkMessage::Assistant");
        }
    }

    #[test]
    fn parse_assistant_with_redacted_thinking() {
        let json = serde_json::json!({
            "type": "assistant",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "redacted_thinking", "data": "c2VjcmV0LXRoaW5raW5n"},
                    {"type": "text", "text": "Here is my response."}
                ]
            },
            "parent_tool_use_id": null
        });

        let msg = SdkMessage::from_value(json).unwrap();
        if let SdkMessage::Assistant(a) = msg {
            assert_eq!(a.message.content.len(), 2);
            match &a.message.content[0] {
                super::super::content::ContentBlock::RedactedThinking { data } => {
                    assert_eq!(data, "c2VjcmV0LXRoaW5raW5n");
                }
                other => panic!("Expected RedactedThinking, got: {other:?}"),
            }
        } else {
            panic!("Expected SdkMessage::Assistant");
        }
    }

    #[test]
    fn parse_usage_with_extended_fields() {
        let json = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "uuid": "abc-123",
            "session_id": "sess-1",
            "duration_ms": 1234,
            "duration_api_ms": 1000,
            "is_error": false,
            "num_turns": 1,
            "result": "Done",
            "total_cost_usd": 0.01,
            "usage": {
                "input_tokens": 500,
                "output_tokens": 200,
                "cache_read_input_tokens": 100,
                "cache_creation_input_tokens": 50,
                "service_tier": "standard",
                "cache_creation": {"foo": "bar"},
                "server_tool_use": {"input_tokens": 10, "output_tokens": 5}
            },
            "modelUsage": {},
            "permission_denials": []
        });

        let msg = SdkMessage::from_value(json).unwrap();
        if let SdkMessage::Result(ResultMessage::Success(s)) = msg {
            assert_eq!(s.usage.input_tokens, 500);
            assert_eq!(s.usage.output_tokens, 200);
            assert_eq!(s.usage.cache_read_input_tokens, Some(100));
            assert_eq!(s.usage.cache_creation_input_tokens, Some(50));
            assert_eq!(s.usage.service_tier.as_deref(), Some("standard"));
            assert!(s.usage.cache_creation.is_some());
            assert!(s.usage.server_tool_use.is_some());
        } else {
            panic!("Expected ResultMessage::Success");
        }
    }

    #[test]
    fn parse_user_message_with_flags() {
        let json = serde_json::json!({
            "type": "user",
            "uuid": "u-123",
            "session_id": "sess-1",
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "hello"}]
            },
            "parent_tool_use_id": null,
            "isMeta": true,
            "isCompactSummary": true,
            "isVisibleInTranscriptOnly": true,
            "planContent": "## Plan\n1. Do stuff",
            "thinkingMetadata": {"budget_tokens": 1024},
            "todos": [{"id": "t1", "text": "Fix bug", "done": false}],
            "summarizeMetadata": {"reason": "auto"},
            "sourceToolAssistantUUID": "asst-uuid-1",
            "sourceToolUseID": "tool-use-1",
            "permissionMode": "plan"
        });

        let msg = SdkMessage::from_value(json).unwrap();
        if let SdkMessage::User(u) = msg {
            assert_eq!(u.is_meta, Some(true));
            assert_eq!(u.is_compact_summary, Some(true));
            assert_eq!(u.is_visible_in_transcript_only, Some(true));
            assert_eq!(u.plan_content.as_deref(), Some("## Plan\n1. Do stuff"));
            assert!(u.thinking_metadata.is_some());
            assert!(u.todos.is_some());
            assert!(u.summarize_metadata.is_some());
            assert_eq!(u.source_tool_assistant_uuid.as_deref(), Some("asst-uuid-1"));
            assert_eq!(u.source_tool_use_id.as_deref(), Some("tool-use-1"));
            assert_eq!(u.permission_mode.as_deref(), Some("plan"));
        } else {
            panic!("Expected SdkMessage::User");
        }
    }

    #[test]
    fn parse_user_message_with_tool_use_result() {
        let json = serde_json::json!({
            "type": "user",
            "uuid": "u-456",
            "session_id": "sess-1",
            "message": {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool-1",
                        "content": "output text"
                    }
                ]
            },
            "parent_tool_use_id": null,
            "toolUseResult": {
                "type": "tool_result",
                "tool_use_id": "tool-1",
                "name": "Bash",
                "stdout": "hello world\n",
                "stderr": "",
                "exitCode": 0
            }
        });

        let msg = SdkMessage::from_value(json).unwrap();
        if let SdkMessage::User(u) = msg {
            let tur = u.tool_use_result.as_ref().expect("Expected toolUseResult");
            assert_eq!(tur["name"], "Bash");
            assert_eq!(tur["exitCode"], 0);
        } else {
            panic!("Expected SdkMessage::User");
        }
    }

    #[test]
    fn parse_user_replay_with_flags() {
        let json = serde_json::json!({
            "type": "user",
            "uuid": "u-replay-1",
            "session_id": "sess-1",
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "replayed msg"}]
            },
            "parent_tool_use_id": null,
            "isReplay": true,
            "isMeta": true,
            "isCompactSummary": false,
            "isVisibleInTranscriptOnly": true,
            "planContent": "## Replay Plan",
            "thinkingMetadata": {"budget_tokens": 2048},
            "todos": [{"id": "t2", "text": "Review", "done": true}],
            "summarizeMetadata": {"reason": "manual"},
            "sourceToolAssistantUUID": "asst-replay-uuid",
            "sourceToolUseID": "tool-replay-1",
            "permissionMode": "default"
        });

        let msg = SdkMessage::from_value(json).unwrap();
        if let SdkMessage::UserReplay(ur) = msg {
            assert!(ur.is_replay);
            assert_eq!(ur.is_meta, Some(true));
            assert_eq!(ur.is_compact_summary, Some(false));
            assert_eq!(ur.is_visible_in_transcript_only, Some(true));
            assert_eq!(ur.plan_content.as_deref(), Some("## Replay Plan"));
            assert!(ur.thinking_metadata.is_some());
            assert!(ur.todos.is_some());
            assert!(ur.summarize_metadata.is_some());
            assert_eq!(ur.source_tool_assistant_uuid.as_deref(), Some("asst-replay-uuid"));
            assert_eq!(ur.source_tool_use_id.as_deref(), Some("tool-replay-1"));
            assert_eq!(ur.permission_mode.as_deref(), Some("default"));
        } else {
            panic!("Expected SdkMessage::UserReplay");
        }
    }
}
