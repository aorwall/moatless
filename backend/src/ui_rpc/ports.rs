//! What this module needs from the rest of the backend in order to write.
//!
//! Everything else here is a database read, which is why the module's other
//! dependencies are DAOs. Sending a message is not: it persists a row, follows
//! a Task on someone's behalf, and wakes a Sandbox. `SandboxConnector`
//! (`crate::sandbox::access`) is the precedent — a narrow port for the one thing
//! behind this boundary that is not a query — and these follow it, so the
//! compatibility layer keeps translating rather than learning how Moatless
//! works.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_backend_core::scope::ScopeContext;

use crate::shared_models::AgentMode;

/// Why a command was not carried out.
///
/// Every variant is an *anticipated* outcome and reaches the client as a typed
/// failure it can render. A defect is reserved for what it means — a bug in this
/// server — so a person is never told the server is broken because they clicked
/// a control it does not implement.
#[derive(Debug, Clone)]
pub enum CommandRefusal {
    /// The command exists in T3's contract and this surface does not implement
    /// it.
    Unsupported { command: String, reason: String },
    /// The Task is gone, closed, or not one this viewer may write to. The three
    /// are deliberately not distinguished: telling somebody a Task they cannot
    /// read exists is itself a disclosure.
    NotWritable { task_id: String },
    /// The request was understood and is wrong — a model we never published, a
    /// provider the Thread cannot run, a Workspace with no repository.
    Rejected { message: String },
    /// The write was attempted and failed. The cause is logged, not sent.
    Failed { message: String },
}

impl CommandRefusal {
    pub fn message(&self) -> String {
        match self {
            Self::Unsupported { command, reason } => {
                format!("`{command}` is not supported by this server. {reason}")
            }
            Self::NotWritable { task_id } => {
                format!("Thread {task_id} cannot be written to from here.")
            }
            Self::Rejected { message } | Self::Failed { message } => message.clone(),
        }
    }
}

/// One turn, as this surface understood it.
pub struct ThreadTurn {
    pub task_id: String,
    pub text: String,
    /// The id the client minted. Persisted verbatim so the client can retire
    /// its optimistic copy.
    pub message_uuid: String,
    /// A catalog model id, already translated out of T3's published slug.
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
}

/// A new Thread, as this surface understood it.
pub struct NewThread {
    /// The id the client minted and has already navigated to.
    pub task_id: String,
    /// The T3 Project, which is a Moatless Workspace.
    pub workspace_id: String,
    /// The provider instance the person picked, which decides which agent runs
    /// the thread. The transport within that kind is not selectable from T3.
    pub instance: String,
    pub title: Option<String>,
    /// A catalog model id.
    pub model: Option<String>,
    /// The first message, when the command carried one. Absent means the Task
    /// is created without a Sandbox.
    pub first_message: Option<ThreadTurn>,
}

/// A title and model change, as this surface understood it.
pub struct ThreadMeta {
    pub task_id: String,
    pub title: Option<String>,
    pub model: Option<String>,
}

/// An answer to a question the agent asked.
pub struct QuestionAnswer {
    pub task_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub payload: Option<serde_json::Value>,
}

/// The agent catalog, as this viewer can use it.
#[async_trait]
pub trait AgentCatalogSource: Send + Sync {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor>;
}

/// Everything a T3 command can ask Moatless to do.
#[async_trait]
pub trait ThreadCommandSink: Send + Sync {
    async fn send_turn(
        &self,
        viewer: &ScopeContext,
        turn: ThreadTurn,
    ) -> Result<(), CommandRefusal>;

    async fn create_thread(
        &self,
        viewer: &ScopeContext,
        thread: NewThread,
    ) -> Result<(), CommandRefusal>;

    async fn update_meta(
        &self,
        viewer: &ScopeContext,
        meta: ThreadMeta,
    ) -> Result<(), CommandRefusal>;

    /// Stop whatever the agent is doing. T3 distinguishes interrupting a turn
    /// from stopping a session; Moatless does not, and both arrive here.
    async fn stop(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn settle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn unsettle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn snooze(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        snoozed_until: DateTime<Utc>,
    ) -> Result<(), CommandRefusal>;

    async fn unsnooze(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal>;

    async fn answer_question(
        &self,
        viewer: &ScopeContext,
        answer: QuestionAnswer,
    ) -> Result<(), CommandRefusal>;
}
