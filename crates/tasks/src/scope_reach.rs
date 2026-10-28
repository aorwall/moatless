//! Which scopes a Task reaches, and who may write or read what a scope holds.
//! Memory entries and record collections share these rules.

use moatless_types::access_control::{Scope, ScopeContext, ScopedResource};
use moatless_types::memory::MemoryScope;
use moatless_types::store::StoreError;
use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};

use crate::task::models::Task;

/// One scope a Task reaches, with the id the Task supplies for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeRef {
    pub scope: MemoryScope,
    pub id: String,
}

/// Why a scope cannot be reached or written.
#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("{0}")]
    Forbidden(String),

    /// The Task has no id for the scope, such as `loop` on a manual Task.
    #[error("{0}")]
    Missing(String),

    /// Reading the Workspace or repository a scope belongs to failed.
    #[error("Store error")]
    Store(#[from] StoreError),
}

/// The owners of the Workspaces and repositories a scope belongs to.
#[derive(Clone, Copy)]
pub struct ScopeOwners<'a> {
    pub workspaces: &'a dyn WorkspaceResolver,
    pub repositories: &'a dyn RepositoryReader,
}

/// `requested` as `task` reaches it.
///
/// A request naming no scope reaches the Task's Loop, else the Task itself.
/// `what` names the store in an error, as in "memory". A request reaches one
/// scope only, deliberately: a read merged across scopes made two scopes'
/// `state` keys collide.
pub fn scope_ref(
    ctx: &ScopeContext,
    task: &Task,
    requested: Option<MemoryScope>,
    what: &str,
) -> Result<ScopeRef, ScopeError> {
    let requested = requested.unwrap_or_else(|| default_scope(task));
    // The owner's own scope: a caller who can merely read someone else's
    // Task does not reach it.
    if requested == MemoryScope::User && owner(task).is_some_and(|o| o != ctx.user_id) {
        return Err(ScopeError::Forbidden(format!(
            "only the owner of task {} reaches its user {what}",
            task.task_id
        )));
    }
    let id = match requested {
        MemoryScope::Task => Some(task.task_id.as_str()),
        MemoryScope::Loop => task.loop_id.as_deref(),
        MemoryScope::Workspace => task.workspace_id.as_deref(),
        MemoryScope::Repository => task.repository_id.as_deref(),
        MemoryScope::User => owner(task),
        MemoryScope::Global => Some(""),
    };
    id.filter(|id| !id.is_empty() || requested == MemoryScope::Global)
        .map(|id| ScopeRef {
            scope: requested,
            id: id.to_string(),
        })
        .ok_or_else(|| {
            ScopeError::Missing(format!("task {} has no {requested} scope", task.task_id))
        })
}

/// The Loop scope of `task`, which a Loop's `memory.preload` reads.
pub fn loop_scope(task: &Task) -> Option<ScopeRef> {
    task.loop_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(|id| ScopeRef {
            scope: MemoryScope::Loop,
            id: id.to_string(),
        })
}

/// The private scope an unscoped request reaches, deliberately rather than
/// `global`: an unscoped put on a private repository must not publish its
/// text to every user.
fn default_scope(task: &Task) -> MemoryScope {
    if loop_scope(task).is_some() {
        MemoryScope::Loop
    } else {
        MemoryScope::Task
    }
}

/// The user a Task belongs to, whose `user` scope it reaches.
pub fn owner(task: &Task) -> Option<&str> {
    task.owner_user_id.as_deref().or(task.created_by.as_deref())
}

/// Refuse a write to a scope other users' Tasks read unless `ctx` may write
/// what it belongs to.
///
/// That is the Workspace or repository itself, and for `global` an admin or
/// a global Task. Writing the Task is not enough: on a shared repository it
/// would let any user put text in another user's scope.
pub async fn check_scope_write(
    owners: ScopeOwners<'_>,
    ctx: &ScopeContext,
    task: &Task,
    scope: &ScopeRef,
    what: &str,
) -> Result<(), ScopeError> {
    let allowed = match scope.scope {
        MemoryScope::Task | MemoryScope::Loop | MemoryScope::User => true,
        MemoryScope::Global => ctx.is_admin || task.scope == Scope::Global,
        MemoryScope::Workspace => owners
            .workspaces
            .describe(ctx, &scope.id)
            .await?
            .is_some_and(|w| ctx.can_write(&ScopedResource::new(w.scope, w.owner_user_id))),
        MemoryScope::Repository => owners
            .repositories
            .describe(ctx, &scope.id)
            .await?
            .is_some_and(|r| ctx.can_write(&ScopedResource::new(r.scope, r.owner_user_id))),
    };
    if allowed {
        Ok(())
    } else {
        Err(ScopeError::Forbidden(format!(
            "cannot write {} {what} for task {}",
            scope.scope, task.task_id
        )))
    }
}

/// Whether `ctx`, a reader rather than the Task's agent, may read what
/// `scope` holds.
///
/// `reads_task` says whether `ctx` can read the Task the scope was resolved
/// for: a `task` or `loop` scope holds no more than what that Task's Loop
/// could have shown them.
pub async fn can_read_scope(
    owners: ScopeOwners<'_>,
    ctx: &ScopeContext,
    scope: &ScopeRef,
    reads_task: bool,
) -> Result<bool, StoreError> {
    Ok(match scope.scope {
        MemoryScope::Task | MemoryScope::Loop => reads_task,
        MemoryScope::User => ctx.is_admin || ctx.user_id == scope.id,
        MemoryScope::Workspace => owners.workspaces.describe(ctx, &scope.id).await?.is_some(),
        MemoryScope::Repository => owners
            .repositories
            .describe(ctx, &scope.id)
            .await?
            .is_some(),
        MemoryScope::Global => true,
    })
}
