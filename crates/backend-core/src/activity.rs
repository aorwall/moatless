//! One activity a Task recorded, and the seam a read model reads them through.
//!
//! `task_activities` is the Task band's table; `moatless-t3code` projects it
//! into the T3 transcript. The vocabulary both sides speak sits here so the read
//! model names the Task band's store through [`ActivitySource`] and its rows
//! through the types below, rather than reaching into the crate that owns them.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::store::StoreError;

/// The closed set. A value outside it is a bug, not an extension point.
///
/// It grows by a code change that fails to compile, never by a migration —
/// which is also why every read narrows to the known kinds in SQL (see
/// `dao::known_kinds`): a row written by a newer backend is skipped rather than
/// failing the decode of the whole conversation around it.
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
    /// The agent handed the conversation back to a person: an `AskUserQuestion`,
    /// or the synthetic approve/reject question in front of a proposed plan.
    UserInputRequested,
    /// That question answered — the row that closes the panel the one above
    /// opened.
    UserInputResolved,
    /// The plan the agent is working through, whole, as of this update.
    PlanUpdated,
    /// What the harness reported about its own context window.
    ContextWindowUpdated,
}

impl ActivityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::Thinking => "thinking",
            Self::SandboxStatus => "sandbox_status",
            Self::SandboxWarning => "sandbox_warning",
            Self::SandboxError => "sandbox_error",
            Self::UserInputRequested => "user_input_requested",
            Self::UserInputResolved => "user_input_resolved",
            Self::PlanUpdated => "plan_updated",
            Self::ContextWindowUpdated => "context_window_updated",
        }
    }

    /// Every value, so a test can assert the SQL narrowing names all of them.
    pub const ALL: &'static [Self] = &[
        Self::ToolCall,
        Self::Thinking,
        Self::SandboxStatus,
        Self::SandboxWarning,
        Self::SandboxError,
        Self::UserInputRequested,
        Self::UserInputResolved,
        Self::PlanUpdated,
        Self::ContextWindowUpdated,
    ];
}

impl std::str::FromStr for ActivityKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "tool_call" => Ok(Self::ToolCall),
            "thinking" => Ok(Self::Thinking),
            "sandbox_status" => Ok(Self::SandboxStatus),
            "sandbox_warning" => Ok(Self::SandboxWarning),
            "sandbox_error" => Ok(Self::SandboxError),
            "user_input_requested" => Ok(Self::UserInputRequested),
            "user_input_resolved" => Ok(Self::UserInputResolved),
            "plan_updated" => Ok(Self::PlanUpdated),
            "context_window_updated" => Ok(Self::ContextWindowUpdated),
            other => Err(format!("unknown activity kind: {other:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The tool calls that hand the conversation back to a person. Keep in sync with
/// `message::dao::INTERACTIVE_TOOLS`, which is the same pair asked about from
/// the write side.
pub const QUESTION_TOOL: &str = "AskUserQuestion";
pub const PLAN_TOOL: &str = "ExitPlanMode";

/// The question in front of a proposed plan.
///
/// A plan proposal parks the run on a call only an answer releases, and the
/// answer panel is the one surface that can send one. So the plan is *asked*
/// rather than merely shown: the card renders the plan, this renders the two
/// things a person can do about it.
pub const PLAN_QUESTION: &str = "Approve this plan?";
pub const PLAN_APPROVE: &str = "Approve";
pub const PLAN_REJECT: &str = "Reject";

/// Which of the contract's closed set of tool shapes a Moatless tool call is.
///
/// The set names how the client *renders* a call, so only the two shapes with a
/// dedicated renderer are worth claiming. Guessing from a free-text tool name is
/// how a search tool ends up rendered as a file diff.
pub fn item_type(tool_name: &str) -> &'static str {
    match tool_name {
        "Bash" | "BashOutput" => "command_execution",
        "Edit" | "Write" | "NotebookEdit" => "file_change",
        _ => "dynamic_tool_call",
    }
}

/// A tool result's visible output.
///
/// The block's `content` is a string for most tools and a list of content blocks
/// for the ones that return structured output; only the first text block of the
/// latter is shown, which is what the work log has room for.
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

/// The activities a Task recorded, for a read model that projects them.
///
/// Four reads, because the transcript is assembled four ways: the whole tail
/// since a cursor, the rows one batch of Messages produced, the rows a set of
/// ids names, and the single call a `tool_use_id` names.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait ActivitySource: Send + Sync {
    /// Top-level activities for a Task, oldest first, from `since` onward.
    async fn since(
        &self,
        task_id: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Activity>, StoreError>;

    /// The activities these Messages produced. An empty slice reads nothing.
    async fn for_messages(
        &self,
        task_id: &str,
        message_uuids: &[String],
    ) -> Result<Vec<Activity>, StoreError>;

    /// The activities these ids name. An empty slice reads nothing.
    async fn for_ids(
        &self,
        task_id: &str,
        activity_ids: &[String],
    ) -> Result<Vec<Activity>, StoreError>;

    /// The one tool call this `tool_use_id` names, whatever its state.
    async fn tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<Activity>, StoreError>;
}

/// How the row decodes. Behind `database` with the rest of the store surface.
#[cfg(feature = "database")]
mod row {
    use sqlx::{Row, postgres::PgRow};

    use super::{Activity, ActivityStatus};

    impl sqlx::FromRow<'_, PgRow> for Activity {
        fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
            let kind: String = row.try_get("kind")?;
            let status: String = row.try_get("status")?;
            Ok(Self {
                activity_id: row.try_get("activity_id")?,
                task_id: row.try_get("task_id")?,
                position: row.try_get("position")?,
                ordinal: row.try_get("ordinal")?,
                message_uuid: row.try_get("message_uuid")?,
                parent_uuid: row.try_get("parent_uuid")?,
                kind: kind
                    .parse()
                    .map_err(|e: String| sqlx::Error::ColumnDecode {
                        index: "kind".to_string(),
                        source: e.into(),
                    })?,
                status: match status.as_str() {
                    "in_progress" => ActivityStatus::InProgress,
                    "completed" => ActivityStatus::Completed,
                    "failed" => ActivityStatus::Failed,
                    other => {
                        return Err(sqlx::Error::ColumnDecode {
                            index: "status".to_string(),
                            source: format!("unknown activity status: {other:?}").into(),
                        });
                    }
                },
                tool_use_id: row.try_get("tool_use_id")?,
                summary: row.try_get("summary")?,
                payload: row.try_get("payload")?,
                detail: row.try_get("detail")?,
                is_error: row.try_get("is_error")?,
                created_at: row.try_get("created_at")?,
            })
        }
    }
}
