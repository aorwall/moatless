//! Loading and authorizing the adapter/loop/task entities the routes act on,
//! and resolving the scope a new adapter resource is created under.

use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::AdapterConnection;
use moatless_backend_core::access_control::{
    ResolvedScope, ensure_can_use_repository, resolve_scope_for_create,
};
use moatless_backend_core::task::{TaskDescriptor, TaskReader};
use moatless_backend_core::workspace::RepositoryReader;
use moatless_types::access_control::{Scope, ScopeContext, ScopedResource};
use moatless_types::adapter::AdapterKind;

use crate::loops::dao::{LoopDao, LoopRecord};

/// Verify a repository exists and the caller may use it, through the one port
/// it needs. Its sole caller (the Loop HTTP handlers) already holds the port,
/// so this names no `AppState` and carries no door.
pub(crate) async fn ensure_repository_exists(
    repositories: &dyn RepositoryReader,
    ctx: &ScopeContext,
    repository_id: &str,
) -> Result<(), AdapterError> {
    let repo = repositories
        .describe(ctx, repository_id)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::BadRequest(format!("Repository {repository_id} not found"))
        })?;
    ensure_can_use_repository(
        ctx,
        &ScopedResource::borrowed(
            repo.scope,
            repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        ),
    )?;
    Ok(())
}

/// Resolve the scope for a new adapter resource. Defaults to `User` owned by
/// the caller; requesting `Global` requires admin.
pub fn resolve_scope_for_adapter_create(
    ctx: &ScopeContext,
    requested_scope: Option<Scope>,
) -> Result<ResolvedScope, AdapterError> {
    Ok(resolve_scope_for_create(ctx, requested_scope)?)
}

/// Load and authorize an adapter connection for the calling user, through the
/// one port it needs — the `adapter_connections` DAO.
///
/// Uses the scoped `find_by_id(id, ctx)` method so only connections visible to
/// the caller are returned; `None` → `NotFound`. Callers hold the port and pass
/// it directly, so this names no `AppState`.
pub(crate) async fn load_adapter_connection_for_auth_with(
    adapter_connections: &dyn AdapterConnectionDao,
    ctx: &ScopeContext,
    id: &str,
) -> Result<AdapterConnection, AdapterError> {
    adapter_connections
        .find_by_id(id, ctx)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::NotFound(format!("Provider connection {id} not found")).into()
        })
}

/// Load and authorize a loop through the one port it needs — the `loops` DAO.
///
/// Uses the scoped `get(id, ctx)` store method so only loops visible to the
/// caller are returned; `None` → `NotFound`. Callers hold the port and pass it
/// directly, so this names no `AppState`.
pub(crate) async fn load_loop_for_auth_with(
    loops: &LoopDao,
    ctx: &ScopeContext,
    id: &str,
) -> Result<LoopRecord, AdapterError> {
    loops
        .get(id, ctx)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")).into())
}

/// Load and authorize a task for the calling user, through the one port it
/// needs — the published [`TaskReader`].
///
/// `None` → `NotFound`: a Task the caller may not read is a Task that is not
/// there. Callers hold the port and pass it directly, so this names no
/// `AppState`.
pub async fn ensure_task_access_with(
    tasks: &dyn TaskReader,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<TaskDescriptor, AdapterError> {
    tasks
        .describe(task_id, ctx)
        .await
        .map_err(|e| SharedAdapterError::Internal(e.to_string()))?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Task {task_id} not found")).into())
}

/// The same authorization, for a caller that discards the Task.
pub async fn ensure_task_access_only(
    tasks: &dyn TaskReader,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), AdapterError> {
    let allowed = tasks
        .may_access(task_id, ctx)
        .await
        .map_err(|e| SharedAdapterError::Internal(e.to_string()))?;
    if allowed {
        Ok(())
    } else {
        Err(SharedAdapterError::NotFound(format!("Task {task_id} not found")).into())
    }
}

/// Resolve an adapter connection for the caller.
///
/// - `Some(id)` → load and authorize that specific connection.
/// - `None` → find all `kind` connections accessible to `ctx`:
///   * 0 → `BadRequest` "no <kind> connection configured"
///   * 1 → return it
///   * 2+ → `BadRequest` with the list of accessible IDs and a hint
pub async fn resolve_connection(
    adapter_connections: &dyn AdapterConnectionDao,
    ctx: &ScopeContext,
    kind: AdapterKind,
    connection_id: Option<&str>,
) -> Result<AdapterConnection, AdapterError> {
    if let Some(id) = connection_id {
        let conn = load_adapter_connection_for_auth_with(adapter_connections, ctx, id).await?;
        if conn.adapter_kind != kind {
            return Err(SharedAdapterError::BadRequest(format!(
                "Connection {id} is for adapter {}, not {}",
                conn.adapter_kind,
                kind.as_str()
            ))
            .into());
        }
        return Ok(conn);
    }
    let rows = adapter_connections.list(ctx, Some(kind.as_str())).await?;
    let mut iter = rows.into_iter();
    match (iter.next(), iter.next()) {
        (None, _) => Err(SharedAdapterError::BadRequest(format!(
            "No {} connection configured. Create one with `moat adapters connections create --adapter {} ...` first.",
            kind.as_str(),
            kind.as_str()
        )).into()),
        (Some(only), None) => Ok(only),
        (Some(first), Some(second)) => {
            let mut ids: Vec<String> = vec![first.id, second.id];
            for row in iter {
                ids.push(row.id);
            }
            Err(SharedAdapterError::BadRequest(format!(
                "Multiple {} connections accessible: [{}]. Pass --connection <id> to choose.",
                kind.as_str(),
                ids.join(", ")
            )).into())
        }
    }
}
