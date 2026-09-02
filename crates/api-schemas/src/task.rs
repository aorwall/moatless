//! Task request/response types.

use serde::{Deserialize, Serialize};

use crate::bindings::TaskBindingSummary;
use crate::enums::{
    CheckoutMode, SandboxDesiredState, SandboxStopReason, Scope, TaskRelationType, TaskStatus,
    Visibility,
};
use crate::shared::{AgentMode, AgentType};

/// A typed link from one task to another, with optional JSON context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskRelationDto {
    pub id: String,
    pub from_task_id: String,
    pub to_task_id: String,
    pub relation_type: TaskRelationType,
    /// Free-form metadata describing why/how the relation was created.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub context: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    /// The repository to run the task against. Optional when `workspace_id` is
    /// supplied — the workspace's primary repository is used instead. At least
    /// one of `repository_id` / `workspace_id` is required.
    #[serde(default)]
    pub repository_id: Option<String>,
    /// Explicit workspace to run the task in. When set, its repositories drive
    /// the task (workspace wins over `repository_id`). When unset, the task
    /// derives the repository's deterministic shadow workspace, keeping the
    /// single-repo path unchanged.
    pub workspace_id: Option<String>,
    /// Existing running Sandbox to join. Omit to create the task's own Sandbox.
    pub sandbox_id: Option<String>,
    /// Checkout relationship to other members when `sandbox_id` is supplied.
    /// Omit for the shared checkout, which is also the single-member behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_mode: Option<CheckoutMode>,
    pub agent_type: Option<AgentType>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub branch: Option<String>,
    pub message: Option<String>,
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
    /// Reasoning effort for the first turn, in the agent's own vocabulary
    /// (`low`/`medium`/`high`/`xhigh`/`max` for Claude,
    /// `minimal`/`low`/`medium`/`high` for Codex). Omit to leave the agent on
    /// its own default.
    pub effort: Option<String>,
    #[serde(default)]
    pub contexts: Vec<MessageContextInput>,
    #[serde(default)]
    pub file_ids: Vec<String>,
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub headless: Option<bool>,
    /// Skills to activate for the first turn. Names only — the sandbox resolves
    /// each one's `SKILL.md` on disk and frontloads its body ahead of the message.
    #[serde(default)]
    pub skills: Vec<String>,
}

/// Branch a Task off another Task at one of its completed turns.
///
/// Everything the fork runs against — repository, workspace, branch, agent,
/// model — comes from the Task being forked; only what the fork is *for* is
/// stated here.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ForkTaskRequest {
    /// The source Task's turn the copy stops after. Omit to cut after its
    /// newest turn that reached a terminal status.
    pub at_turn: Option<i32>,
    /// The fork's own first message — what to try instead. Omit to create the
    /// fork idle, and send to it later.
    pub message: Option<String>,
    /// Join the source Task's Sandbox on its shared checkout instead of
    /// provisioning one from a copy of its workspace archive.
    #[serde(default)]
    pub same_sandbox: Option<bool>,
    /// Reasoning effort for the first turn, in the agent's own vocabulary.
    pub effort: Option<String>,
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
    /// Skills to activate for the first turn.
    #[serde(default)]
    pub skills: Vec<String>,
}

/// Data for an element context (selected UI component).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ElementContextData {
    pub component_id: String,
    pub path: String,
    pub line: i32,
    pub column: i32,
    pub component_name: String,
}

/// Data for a preview URL context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PreviewUrlContextData {
    pub path: String,
    pub url: Option<String>,
}

/// Data for a file attachment context. Filled in by the backend at read
/// time from `file_metadata`; senders supply only `reference_id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileContextData {
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}

/// A context attached to a message, discriminated by `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type")]
pub enum MessageContextInput {
    /// Selected UI element context.
    #[serde(rename = "element", rename_all = "camelCase")]
    Element {
        reference_id: String,
        data: ElementContextData,
    },
    /// Preview URL context.
    #[serde(rename = "previewUrl", rename_all = "camelCase")]
    PreviewUrl {
        reference_id: String,
        data: PreviewUrlContextData,
    },
    /// Uploaded file attachment context.
    #[serde(rename = "file", rename_all = "camelCase")]
    File {
        /// Upload ID returned by `POST /api/v1/uploads`.
        reference_id: String,
        /// Backend-resolved metadata. Senders may pass an empty default — the
        /// backend overwrites it from `file_metadata` before persisting.
        #[serde(default)]
        data: FileContextData,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskMetadataRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CloseTaskRequest {
    /// Whether descendant tasks created by this task should also be closed.
    /// Required when the task has children; optional for leaf tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_children: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateVisibilityRequest {
    pub visibility: Visibility,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskTtlRequest {
    /// New idle TTL in minutes. Must be `>= 0`. `0` disables the idle reaper
    /// for this task (sandbox stays alive until explicitly stopped). Omit to
    /// leave the current idle TTL unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_minutes: Option<i32>,
    /// New inactivity TTL in days. Must be `>= 0`. `0` disables auto-close for
    /// this task (it stays Open until closed explicitly). Omit to leave the
    /// current inactivity TTL unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inactive_ttl_days: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetTagsRequest {
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct ListTasksQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub status: Option<TaskStatus>,
    pub repository_id: Option<String>,
    pub created_by: Option<String>,
    pub tag: Option<String>,
    pub parent_task_id: Option<String>,
    /// When set, restrict results to tasks this user follows. Drives the
    /// personal task list (own tasks plus followed tasks from other users).
    pub followed_by: Option<String>,
}

/// Where a forked task was branched from, derived from its `forked_from`
/// relation. `at_turn` is the source task's turn the copy stops after; `None`
/// means the whole conversation was copied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ForkOrigin {
    pub task_id: String,
    pub at_turn: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskResponse {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub agent_type: AgentType,
    pub repository_id: String,
    pub repository_name: Option<String>,
    /// Workspace the task runs in. Every task has one — the insert resolves the
    /// repository's shadow workspace when the creating request named none, and
    /// pre-workspace tasks were backfilled the same way — so this is only
    /// `None` for a response built without reading the persisted row.
    pub workspace_id: Option<String>,
    /// Display name for `workspace_id`, resolved alongside `repository_name`.
    /// `None` when the workspace row is deleted or its lookup failed; clients
    /// showing this as a label fall back to the repository.
    pub workspace_name: Option<String>,
    /// Sandbox this Task belongs to. `None` means it has never been started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    /// Checkout relationship to other members of the Sandbox. `None` reads as
    /// `shared` for tasks created before this field existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout_mode: Option<CheckoutMode>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
    pub total_cost_usd: f64,
    pub site_url: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub created_by: Option<String>,
    pub created_by_display_name: Option<String>,
    pub visibility: Visibility,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    /// Derived from whichever parent relation in `relations` makes this task a
    /// child — `created_by` or `forked_from`. Kept for backward compatibility
    /// with clients that pre-date the relations field. Read `forked_from` to
    /// tell the two apart.
    pub parent_task_id: Option<String>,
    /// Set when this task was branched off another one. `parent_task_id` names
    /// the same task; this field says the edge is a fork and where it was cut.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ForkOrigin>,
    /// Typed relations this task has to other tasks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<TaskRelationDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append_system_prompt: Option<String>,
    pub tags: Vec<TagResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<TaskBindingSummary>,
    /// Idle TTL in minutes. The reaper stops the sandbox when no activity has
    /// occurred for this many minutes. `0` means "never reap".
    pub ttl_minutes: i32,
    /// Inactive TTL in days. The reaper auto-closes the task when no activity
    /// has occurred for this many days. `0` means "never auto-close".
    pub inactive_ttl_days: i32,
    /// RFC3339 timestamp of when the sandbox was last started. `None` if the
    /// task has never had a sandbox running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_started_at: Option<String>,
    /// Persisted target state for the task's sandbox. This is application
    /// intent, not live infrastructure status.
    pub sandbox_desired_state: SandboxDesiredState,
    /// Reason the sandbox was intentionally moved away from `running`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_stop_reason: Option<SandboxStopReason>,
    /// RFC3339 timestamp for the last desired-state transition.
    pub sandbox_desired_state_updated_at: String,
    /// RFC3339 timestamp of when the requesting user last read this task's
    /// conversation. `None` if they have never opened it. Compare against
    /// `last_message_at` to derive unread state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_read_at: Option<String>,
    /// Whether the requesting user follows this task. Followed tasks appear in
    /// the user's personal task list. `None` when the response was built
    /// without a requesting-user context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_following: Option<bool>,
}

/// Aggregate token counts for a task conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskTokenTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Cache-write plus cache-read input tokens.
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub total_tokens: i64,
}

/// Token usage attached to a single assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskMessageTokenUsage {
    pub message_uuid: String,
    pub timestamp: String,
    pub turn_number: Option<i32>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub total_tokens: i64,
}

/// Tool-call aggregate for a task conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskToolCallStats {
    pub tool_name: String,
    pub total_calls: i64,
    pub failed_calls: i64,
}

/// Skill usage aggregate for a task conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskSkillUsage {
    pub skill_name: String,
    pub total_calls: i64,
}

/// Conversation statistics derived from persisted task messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskStatisticsResponse {
    pub task_id: String,
    pub agent_type: AgentType,
    pub model: Option<String>,
    pub token_totals: TaskTokenTotals,
    pub message_token_usage: Vec<TaskMessageTokenUsage>,
    pub tool_calls: Vec<TaskToolCallStats>,
    pub skills: Vec<TaskSkillUsage>,
    /// Whether this Task has a tool-call record at all.
    ///
    /// A Task that ran before tool calls were stored as their own rows has none,
    /// and an empty `tool_calls` from that Task means "not recorded", not "made
    /// no calls". The client has to be able to tell those apart — reporting zero
    /// is a wrong answer, not a missing one.
    ///
    /// Defaults to `true` so a client reading an older response, or a response
    /// from a deployment that predates the field, is not told every Task is
    /// unrecorded.
    #[serde(default = "default_true")]
    pub tool_calls_available: bool,
}

fn default_true() -> bool {
    true
}

/// Response for follow/unfollow endpoints, reporting the resulting state.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FollowTaskResponse {
    /// Whether the requesting user follows the task after the operation.
    pub following: bool,
}

/// Tag as embedded in a task response (minimal: id + name only).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TagResponse {
    pub tag_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub status: TaskStatus,
    pub task: TaskResponse,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    pub items: Vec<TaskResponse>,
    pub pagination: PaginationResponse,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PaginationResponse {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_element_context() {
        let json = serde_json::json!({
            "type": "element",
            "referenceId": "ref-1",
            "data": {
                "componentId": "comp-1",
                "path": "src/App.tsx",
                "line": 10,
                "column": 5,
                "componentName": "App"
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).expect("deserialize");
        match ctx {
            MessageContextInput::Element { reference_id, data } => {
                assert_eq!(reference_id, "ref-1");
                assert_eq!(data.component_id, "comp-1");
                assert_eq!(data.path, "src/App.tsx");
                assert_eq!(data.line, 10);
                assert_eq!(data.column, 5);
                assert_eq!(data.component_name, "App");
            }
            _ => panic!("Expected Element variant"),
        }
    }

    #[test]
    fn deserialize_preview_url_context() {
        let json = serde_json::json!({
            "type": "previewUrl",
            "referenceId": "ref-3",
            "data": {
                "path": "/dashboard",
                "url": "https://example.com/dashboard"
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).expect("deserialize");
        match ctx {
            MessageContextInput::PreviewUrl { reference_id, data } => {
                assert_eq!(reference_id, "ref-3");
                assert_eq!(data.path, "/dashboard");
                assert_eq!(data.url.as_deref(), Some("https://example.com/dashboard"));
            }
            _ => panic!("Expected PreviewUrl variant"),
        }
    }

    #[test]
    fn deserialize_preview_url_context_without_url() {
        let json = serde_json::json!({
            "type": "previewUrl",
            "referenceId": "ref-4",
            "data": {
                "path": "/about"
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).expect("deserialize");
        match ctx {
            MessageContextInput::PreviewUrl { data, .. } => {
                assert_eq!(data.path, "/about");
                assert!(data.url.is_none());
            }
            _ => panic!("Expected PreviewUrl variant"),
        }
    }

    #[test]
    fn deserialize_file_context() {
        let json = serde_json::json!({
            "type": "file",
            "referenceId": "upload-1",
            "data": {
                "filename": "report.pdf",
                "mimeType": "application/pdf",
                "size": 1234
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).expect("deserialize");
        match ctx {
            MessageContextInput::File { reference_id, data } => {
                assert_eq!(reference_id, "upload-1");
                assert_eq!(data.filename, "report.pdf");
                assert_eq!(data.mime_type, "application/pdf");
                assert_eq!(data.size, 1234);
            }
            _ => panic!("Expected File variant"),
        }
    }

    #[test]
    fn deserialize_file_context_with_default_data() {
        let json = serde_json::json!({
            "type": "file",
            "referenceId": "upload-2"
        });

        let ctx: MessageContextInput = serde_json::from_value(json).expect("deserialize");
        match ctx {
            MessageContextInput::File { reference_id, data } => {
                assert_eq!(reference_id, "upload-2");
                assert_eq!(data.filename, "");
                assert_eq!(data.mime_type, "");
                assert_eq!(data.size, 0);
            }
            _ => panic!("Expected File variant"),
        }
    }

    #[test]
    fn deserialize_unknown_context_type_fails() {
        let json = serde_json::json!({
            "type": "unknown",
            "referenceId": "ref-5",
            "data": {}
        });

        let result = serde_json::from_value::<MessageContextInput>(json);
        assert!(result.is_err());
    }
}
