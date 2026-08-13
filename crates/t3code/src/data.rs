use async_trait::async_trait;
use moatless_api_schemas::workspace::ScriptConfig;
use moatless_backend_core::models::TaskStatus;
use moatless_backend_core::scope::ScopeContext;

use crate::workspace::UiRpcListedWorkspace;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskView {
    pub status: TaskStatus,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3WorkspaceDao: Send + Sync {
    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<UiRpcListedWorkspace>, sqlx::Error>;

    /// The repository a Workspace's branch picker lists, or `None` when the
    /// viewer cannot be shown the Workspace, it is gone, or it declares no
    /// repository. One answer rather than an existence check and a read: the
    /// three cases are one answer to the caller — there is no list to show.
    async fn primary_repository(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;

    /// The Scripts declared by the Workspace a Task runs against, scoped to the
    /// viewer. `None` when the Task is not readable or does not exist — the same
    /// answer as everywhere else a scoped read finds nothing; `Some(vec)` (which
    /// may be empty) when it is readable.
    async fn scripts_for_task(
        &self,
        ctx: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Vec<ScriptConfig>>, sqlx::Error>;

    /// The Workspace a scripts write is about to land on, as far as that write
    /// cares: whether git owns it, and what it declares today. `None` when the
    /// viewer cannot be shown it or it does not exist.
    ///
    /// Read before the write rather than folded into it because the two "no"
    /// answers are different sentences — a Workspace nobody can find, and one
    /// that exists and is git's to change — and because the write needs the
    /// current Scripts to carry over what the client never sees
    /// ([`ScriptConfig::env_vars`]).
    async fn scripts_write_target(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<ScriptsWriteTarget>, sqlx::Error>;

    /// Replace a Workspace's declared Scripts wholesale, scoped to the viewer.
    ///
    /// Wholesale because that is the shape the client writes in: adding,
    /// editing and deleting one Script are all "here is the list now".
    /// `false` when no row was scoped to this viewer — the same answer
    /// [`Self::scripts_write_target`] gives, re-checked here because the two
    /// statements are not one transaction.
    async fn replace_scripts(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
        scripts: &[ScriptConfig],
    ) -> Result<bool, sqlx::Error>;
}

/// What the Scripts write path needs to know about the Workspace it targets.
#[derive(Debug, Clone)]
pub struct ScriptsWriteTarget {
    /// The Workspace's `source` column. `git` means a re-sync owns the run
    /// config and an edit here would be overwritten by it.
    pub source: String,
    /// The Scripts as stored today, unreadable ones included — the write folds
    /// its own list onto these to keep the fields the client never carries.
    pub scripts: Vec<ScriptConfig>,
}

impl ScriptsWriteTarget {
    /// Whether this Workspace's run config is git's to change rather than a
    /// person's. Mirrors the REST surface's `ensure_workspace_mutable`.
    #[must_use]
    pub fn is_git_synced(&self) -> bool {
        self.source == "git"
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3TaskDao: Send + Sync {
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error>;
}
