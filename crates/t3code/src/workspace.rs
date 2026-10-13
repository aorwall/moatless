//! Which conversation's Sandbox a `cwd` names, and whether this viewer may
//! be told it exists.
//!
//! `cwd` is the only handle these requests carry, so the path is the address.
//!
//! **A `cwd` is a claim, not a capability**: it's re-read through the same
//! follow-and-access filter as the shell listing, so an unreadable Task is
//! indistinguishable from a nonexistent one.
//!
//! **A removed Sandbox's working copy is reclaimed**: a stale `cwd` answers
//! as a conversation with no worktree, not a failure.

use super::dao::ConversationDao;
use super::nominal_path;
use chrono::{DateTime, Utc};
use moatless_ports::sandbox::SandboxError;
use moatless_types::access_control::Scope;
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::SandboxDesiredState;
use serde_json::Value;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WorkspaceView {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<Value>,
    pub kind: String,
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<Value>,
    pub env_vars: Option<Value>,
    pub scripts: Option<Value>,
    pub resources: Option<Value>,
    pub network_profile: Option<String>,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub source: String,
    pub source_repository_id: Option<String>,
    pub source_key: Option<String>,
    pub source_config_path: Option<String>,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UiRpcListedWorkspace {
    #[sqlx(flatten)]
    pub workspace: WorkspaceView,
    pub readable: bool,
}

/// Why a conversation's workspace could not be read.
///
/// The split is by what the client should be told, not by where the failure
/// came from: the first three are answers about the request, the last two are
/// this deployment failing to produce one.
#[derive(Debug)]
pub enum WorkspaceAccessError {
    /// The `cwd` names a project rather than a conversation's worktree, which
    /// is what the client sends for a conversation reporting no worktree at
    /// all. A state, not a defect — the conversation has no Sandbox.
    NoWorktree,
    /// The `cwd` is not a path this module issues, or names a conversation this
    /// viewer cannot be shown. Deliberately one variant: telling the two apart
    /// on the wire would answer whether a Task id exists.
    Unaddressable,
    /// The viewer may be shown the conversation but may not operate it.
    ///
    /// Its own variant rather than [`Self::Unaddressable`]: here the person is
    /// looking at the thing they are refused, so hiding its existence reads as
    /// a defect.
    NoControl,
    /// The conversation's Sandbox was not ready to be called at all — stopped,
    /// still starting, or gone. This is the readiness gate refusing, before any
    /// request is made.
    NotRunning(SandboxError),
    /// The Sandbox was ready and the request to it still failed: it refused the
    /// request, or it never answered.
    ///
    /// Kept apart from [`Self::NotRunning`]: the two send the person to
    /// different places, and telling an unreachable-but-up sandbox "not
    /// running" sends them looking in the wrong direction.
    Sandbox(SandboxError),
    Database(sqlx::Error),
}

impl From<sqlx::Error> for WorkspaceAccessError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl WorkspaceAccessError {
    /// What the person in front of the client is told.
    ///
    /// The client renders `message` verbatim in the panel that asked, so this
    /// is user-facing text, not a log line; causes stay behind and are logged
    /// once, where the frame is built.
    pub fn message(&self) -> String {
        match self {
            Self::NoWorktree => {
                "This conversation has no sandbox, so it has no workspace yet.".into()
            }
            Self::Unaddressable => "This conversation's workspace is not available.".into(),
            Self::NoControl => {
                "You can read this conversation, but only its owner can run things in its \
                 sandbox."
                    .into()
            }
            Self::NotRunning(_) => "The sandbox for this conversation is not running.".into(),
            Self::Sandbox(SandboxError::NotReady(_)) => {
                "The sandbox for this conversation did not answer.".into()
            }
            Self::Sandbox(SandboxError::NotFound(detail)) => {
                format!("Not found in the sandbox workspace: {detail}")
            }
            Self::Sandbox(SandboxError::BadRequest(detail)) => {
                format!("The sandbox refused the request: {detail}")
            }
            Self::Sandbox(SandboxError::Unsupported { .. }) => {
                "This sandbox is running an image from before this feature. Redeploy it to use \
                 this here."
                    .into()
            }
            Self::Sandbox(_) => "The sandbox could not be reached.".into(),
            Self::Database(_) => "This conversation's workspace could not be read.".into(),
        }
    }
}

pub fn sandbox_access(error: SandboxError) -> WorkspaceAccessError {
    match error {
        SandboxError::NotReady(_) => WorkspaceAccessError::NotRunning(error),
        other => WorkspaceAccessError::Sandbox(other),
    }
}

/// The conversation a `cwd` names, once this viewer is allowed to be told it
/// exists.
///
/// Re-applies the access filter: being issued a `cwd` says nothing about who
/// holds it now. Following isn't re-checked — the shell paths gate on
/// control instead (see [`controlled_conversation`]).
pub(crate) async fn addressed_conversation(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<String, WorkspaceAccessError> {
    let task_id = addressable_task_id(cwd)?;
    let conversation = conversations
        .readable_by_id(viewer, &task_id)
        .await?
        .ok_or(WorkspaceAccessError::Unaddressable)?;
    match conversation.sandbox_desired_state() {
        Some(SandboxDesiredState::Running | SandboxDesiredState::Stopped) => {
            Ok(conversation.task_id)
        }
        Some(SandboxDesiredState::Removed) | None => Err(WorkspaceAccessError::NoWorktree),
    }
}

/// The conversation a `cwd` names, for a caller about to *operate* it: open a
/// terminal, run a script, write a file, commit.
///
/// [`addressed_conversation`] first, so an unreadable Task is unaddressable
/// rather than forbidden, then the control check — a public Task grants it to
/// any signed-in caller, a private one only to its owner.
pub(crate) async fn controlled_conversation(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<String, WorkspaceAccessError> {
    let task_id = addressed_conversation(conversations, viewer, cwd).await?;
    if !conversations
        .controls_conversation(viewer, &task_id)
        .await?
    {
        return Err(WorkspaceAccessError::NoControl);
    }
    Ok(task_id)
}

fn addressable_task_id(cwd: &str) -> Result<String, WorkspaceAccessError> {
    if nominal_path::is_project_root(cwd) {
        return Err(WorkspaceAccessError::NoWorktree);
    }
    nominal_path::task_id_from_cwd(cwd)
        .map(str::to_string)
        .ok_or(WorkspaceAccessError::Unaddressable)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both of these arrive as `NotReady`, and saying "not running" about the
    /// second one is a lie the person cannot get past: their sandbox *is* up,
    /// so waiting for it to start never resolves anything.
    #[test]
    fn a_sandbox_that_did_not_answer_is_not_a_sandbox_that_is_down() {
        let down = WorkspaceAccessError::NotRunning(SandboxError::NotReady("t-1".into()));
        let silent = WorkspaceAccessError::Sandbox(SandboxError::NotReady("list_files".into()));
        assert_ne!(down.message(), silent.message());
    }

    /// A sandbox from before a method existed answers the call and refuses it,
    /// so "could not be reached" describes the wrong thing and points at the
    /// wrong fix. The only thing that resolves it is redeploying the sandbox.
    #[test]
    fn a_sandbox_too_old_for_a_method_is_not_a_sandbox_that_is_unreachable() {
        let stale = WorkspaceAccessError::Sandbox(SandboxError::Unsupported {
            operation: "list_git_refs".into(),
            source: Box::new(tonic::Status::unimplemented("unknown method")),
        });
        let unreachable = WorkspaceAccessError::Sandbox(SandboxError::Grpc {
            operation: "list_git_refs".into(),
            source: Box::new(tonic::Status::internal("boom")),
        });
        assert_ne!(stale.message(), unreachable.message());
        assert!(stale.message().contains("Redeploy"), "{}", stale.message());
    }
}
