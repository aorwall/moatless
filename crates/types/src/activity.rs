//! Shared activity values and their read-side interpretation.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// The closed set. A value outside it is a bug, not an extension point.
///
/// Grows only by a code change that fails to compile, which is why every read
/// narrows to known kinds in SQL (`dao::known_kinds`): a newer row is skipped,
/// not a decode failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    ToolCall,
    Thinking,
    SandboxStatus,
    /// A Sandbox stall the run recovers from — a slow startup, blocked
    /// scheduling, an eviction. The severity between an ordinary status and a
    /// fatal error, and the one the T3 work log colours amber.
    SandboxWarning,
    SandboxError,
    /// The agent handed the conversation back to a person: an
    /// `AskUserQuestion`, or the synthetic approve/reject question in front of
    /// a proposed plan.
    UserInputRequested,
    /// That question answered — the row that closes the panel the one above
    /// opened.
    UserInputResolved,
    /// The plan the agent is working through, whole, as of this update.
    PlanUpdated,
    /// What the harness reported about its own context window.
    ContextWindowUpdated,
    /// The platform moved the Task to another agent, and why.
    AgentSwitched,
    /// The harness compacted its context; the payload's `summary` is what it
    /// kept.
    Compaction,
    /// An Artifact version published with `--inline`, shown in the thread as
    /// the page itself; the payload is an [`ArtifactShown`].
    ArtifactShown,
}

// `ALL` is used by the SQL narrowing in `dao::known_kinds`, which a test
// asserts names every kind.
crate::string_enum_mapping!(
    ActivityKind {
        ToolCall => "tool_call",
        Thinking => "thinking",
        SandboxStatus => "sandbox_status",
        SandboxWarning => "sandbox_warning",
        SandboxError => "sandbox_error",
        UserInputRequested => "user_input_requested",
        UserInputResolved => "user_input_resolved",
        PlanUpdated => "plan_updated",
        ContextWindowUpdated => "context_window_updated",
        AgentSwitched => "agent_switched",
        Compaction => "compaction",
        ArtifactShown => "artifact_shown",
    }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// arch-allow(floor-type-admission): Status is part of the shared `Activity`
/// and `ActivityView` contract.
pub enum ActivityStatus {
    InProgress,
    Completed,
    Failed,
}

impl ActivityStatus {
    /// How the column stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    /// How the T3 contract spells it. Two spellings because the column is this
    /// codebase's vocabulary and the payload is the client's — `projection.rs`
    /// has always written `inProgress` into the payload it sends.
    pub fn wire(self) -> &'static str {
        match self {
            Self::InProgress => "inProgress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// One stored activity.
#[derive(Debug, Clone)]
pub struct Activity {
    pub activity_id: String,
    pub task_id: String,
    pub position: i64,
    /// Where this row sits inside its change. See
    /// [`ActivityRow::ordinal`](ActivityRow::ordinal).
    pub ordinal: i32,
    pub message_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub kind: ActivityKind,
    pub status: ActivityStatus,
    pub tool_use_id: Option<String>,
    pub summary: String,
    pub payload: Value,
    /// What a settled call shows before it is expanded. `None` until the call
    /// comes back, and on every activity that is not a tool call.
    pub detail: Option<String>,
    pub is_error: bool,
    pub created_at: DateTime<Utc>,
}

impl Activity {
    /// The tool's input as the call carried it, or `Null` for an activity that
    /// is not a tool call.
    pub fn tool_input(&self) -> &Value {
        self.payload.get("input").unwrap_or(&Value::Null)
    }

    /// The result whole, or `None` while the call is still in progress.
    pub fn result(&self) -> Option<&Value> {
        self.payload.get("result")
    }

    /// The transport's structured view of the outcome, where it reported one.
    pub fn typed_result(&self) -> Option<&Value> {
        self.payload.get("typedResult")
    }

    /// Whether the call this activity records has come back.
    pub fn is_settled(&self) -> bool {
        self.status != ActivityStatus::InProgress
    }
}

/// The payload of an `ArtifactShown` activity and of the `artifact_shown` task
/// event it is projected from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactShown {
    pub artifact_id: String,
    pub version: i32,
    pub title: String,
    /// The frame height in CSS pixels, which caps the height the page reports.
    pub height: i32,
}

/// The tool calls that hand the conversation back to a person. Keep in sync
/// with `message::dao::INTERACTIVE_TOOLS`, which is the same pair asked about
/// from the write side.
pub const QUESTION_TOOL: &str = "AskUserQuestion";
pub const PLAN_TOOL: &str = "ExitPlanMode";

/// The Claude CLI's plan tracker; [`crate::codex::CODEX_TOOL_TODO_LIST`] is
/// what the Codex transport's `turn/plan/updated` notification is normalised into.
pub const TODO_TOOL: &str = "TodoWrite";

/// The question in front of a proposed plan.
///
/// A plan parks the run on a call only an answer releases, so it is *asked*
/// rather than shown: the card renders the plan, this renders the two things a
/// person can do.
pub const PLAN_QUESTION: &str = "Approve this plan?";
pub const PLAN_APPROVE: &str = "Approve";
pub const PLAN_REJECT: &str = "Reject";

/// Which of the contract's closed set of tool shapes a Moatless tool call is.
///
/// Names how the client *renders* a call; guessing from a free-text tool name
/// is how a search tool ends up rendered as a file diff.
pub fn item_type(tool_name: &str) -> &'static str {
    match tool_name {
        "Bash" | "BashOutput" => "command_execution",
        "Edit" | "Write" | "NotebookEdit" => "file_change",
        _ => "dynamic_tool_call",
    }
}

/// A tool result's visible output.
///
/// `content` is a string for most tools, or a block list for structured output
/// — only the first text block of the latter is shown.
pub fn result_text(result: &Value) -> Option<&str> {
    if let Some(text) = result.as_str() {
        return Some(text);
    }
    result
        .as_array()?
        .iter()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|part| part.get("text").and_then(Value::as_str))
}

/// Canonical read-side meaning of one stored activity.
///
/// A client adapter translates this view into its own wire shape instead of
/// re-reading the storage payload rules locally.
pub struct ActivityView<'a> {
    activity: &'a Activity,
}

impl<'a> ActivityView<'a> {
    pub fn new(activity: &'a Activity) -> Self {
        Self { activity }
    }

    pub fn activity_id(&self) -> &str {
        &self.activity.activity_id
    }

    pub fn kind(&self) -> ActivityKind {
        self.activity.kind
    }

    pub fn status(&self) -> ActivityStatus {
        self.activity.status
    }

    pub fn summary(&self) -> &str {
        &self.activity.summary
    }

    pub fn tool_use_id(&self) -> Option<&str> {
        self.activity.tool_use_id.as_deref()
    }

    pub fn input(&self) -> &Value {
        self.activity.tool_input()
    }

    pub fn payload(&self) -> &Value {
        &self.activity.payload
    }

    pub fn result_text(&self) -> Option<&str> {
        self.activity.result().and_then(result_text)
    }

    pub fn typed_result(&self) -> Option<&Value> {
        self.activity.typed_result()
    }

    pub fn detail(&self) -> Option<&str> {
        self.activity.detail.as_deref()
    }

    pub fn is_error(&self) -> bool {
        self.activity.is_error
    }

    pub fn is_settled(&self) -> bool {
        self.activity.is_settled()
    }
}
