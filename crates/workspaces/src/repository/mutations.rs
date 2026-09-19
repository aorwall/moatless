use super::dao::RepositoryDao;
use super::error::RepositoryError;
use super::models::Repository;
use super::models::RepositoryPatch;
use super::schemas::{CreateRepositoryRequest, UpdateRepositoryRequest};
use crate::repository::remote::{ensure_no_provider_mismatch, normalize_remote_url};
use crate::workspace::git_sync::{GitSyncService, SyncForce, log_workspace_sync_for};
use moatless_core::db::DbPool;
use moatless_types::access_control::{Scope, ScopeContext, resolve_scope_for_create};
use moatless_workspace_types::repository::RepositoryKind;
use std::collections::HashSet;
use tracing::warn;

async fn ensure_unique_remote_url(
    repositories: &dyn super::dao::RepositoryDao,
    repository_id: Option<&str>,
    remote_url: Option<&str>,
    scope: Scope,
    owner_user_id: Option<String>,
) -> Result<(), RepositoryError> {
    let Some(remote_url) = remote_url else {
        return Ok(());
    };
    let url_key = normalize_remote_url(remote_url);
    let existing = repositories
        .find_by_normalized_remote_url(&url_key, scope, owner_user_id)
        .await?;
    if existing
        .as_ref()
        .is_some_and(|existing| Some(existing.id.as_str()) != repository_id)
    {
        return Err(RepositoryError::Conflict(remote_url.to_string()));
    }
    Ok(())
}

pub(super) fn preserved_user_scope(existing: &Repository) -> Result<String, RepositoryError> {
    existing.owner_user_id.clone().ok_or_else(|| {
        RepositoryError::BadRequest("Cannot move an ownerless repository to user scope".to_string())
    })
}

/// Apply the repository mutation with caller scope and existing write policy.
pub async fn create_repository(
    repositories: &dyn RepositoryDao,
    git_sync: &GitSyncService,
    ctx: &ScopeContext,
    req: CreateRepositoryRequest,
) -> Result<Repository, RepositoryError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let user_id = &ctx.user_id;

    let resolved_scope = resolve_scope_for_create(ctx, req.scope)?;
    let scope_ctx = ctx;

    if let Some(remote_url) = req.remote_url.as_deref() {
        ensure_no_provider_mismatch(req.provider, remote_url)?;
    }

    ensure_unique_remote_url(
        repositories,
        None,
        req.remote_url.as_deref(),
        resolved_scope.scope,
        resolved_scope.owner_user_id.clone(),
    )
    .await?;

    let row = Repository {
        id: id.clone(),
        name: req.name,
        kind: req.kind,
        remote: req
            .remote_url
            .map(|url| crate::repository::remote::resolve_remote(req.provider, url)),
        default_branch: req.default_branch,
        description: req.description,
        provider: req.provider,
        scope: resolved_scope.scope,
        owner_user_id: resolved_scope.owner_user_id,
        created_by: Some(user_id.to_string()),
        deleted: false,
        created_at: Some(now),
        updated_at: Some(now),
    };

    repositories.insert(&row).await.map_err(|error| {
        RepositoryError::from_write_error(
            error,
            row.remote.as_ref().map(|remote| remote.url.as_str()),
        )
    })?;

    // A repository is only a git connection; its run-config flows to the
    // workspace via the git sync below, not into repo columns.
    let final_row = repositories
        .find_by_id(&id, scope_ctx)
        .await?
        .unwrap_or(row);

    if final_row.remote.is_some() {
        // Sync git-declared workspaces (`.moatless/workspaces.json`).
        // Loop sync is deliberately never automatic:
        // an administrator must invoke the explicit sync endpoint.
        log_workspace_sync_for(
            git_sync,
            &final_row,
            ctx,
            "create",
            &SyncForce::RespectOverrides,
        )
        .await;
    }

    Ok(final_row)
}

/// Apply the repository mutation with caller scope and existing write policy.
pub async fn update_repository(
    repositories: &dyn RepositoryDao,
    ctx: &ScopeContext,
    repository_id: String,
    req: UpdateRepositoryRequest,
) -> Result<Repository, RepositoryError> {
    let existing = repositories
        .find_by_id(&repository_id, ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    // User ownership is immutable here: re-submitting `scope: user` preserves
    // the existing owner even when an admin is editing the row.
    // Global moves require admin (enforced by resolve_scope_for_create).
    let (scope, owner_user_id) = match req.scope {
        Some(Scope::User) => {
            let existing_owner_user_id = preserved_user_scope(&existing)?;
            (Some(Scope::User), Some(Some(existing_owner_user_id)))
        }
        Some(new_scope) => {
            let resolved = resolve_scope_for_create(ctx, Some(new_scope))?;
            (Some(resolved.scope), Some(resolved.owner_user_id))
        }
        None => (None, None),
    };

    let target_scope = scope.unwrap_or(existing.scope);
    let target_owner_user_id = owner_user_id
        .clone()
        .unwrap_or_else(|| existing.owner_user_id.clone());
    let target_remote_url = req
        .remote_url
        .clone()
        .or_else(|| existing.remote.as_ref().map(|remote| remote.url.clone()));

    // Either half may be patched alone, so the check reads the pair the row
    // ends up holding rather than the one the request carried. A patch sending
    // neither is exempt: a row registered before this check must stay editable
    // by a rename, and repairing it needs `provider` anyway.
    if req.provider.is_some() || req.remote_url.is_some() {
        if let Some(remote_url) = target_remote_url.as_deref() {
            ensure_no_provider_mismatch(req.provider.or(existing.provider), remote_url)?;
        }
    }

    ensure_unique_remote_url(
        repositories,
        Some(&repository_id),
        target_remote_url.as_deref(),
        target_scope,
        target_owner_user_id,
    )
    .await?;

    let patch = RepositoryPatch {
        scope,
        owner_user_id,
        name: req.name,
        provider: req.provider.map(Some),
        remote_url: req.remote_url.map(Some),
        default_branch: req.default_branch.map(Some),
        description: req.description.map(Some),
        ..Default::default()
    };

    let applied = repositories
        .update(&repository_id, &patch, ctx)
        .await
        .map_err(|error| RepositoryError::from_write_error(error, target_remote_url.as_deref()))?;
    if !applied {
        // write_filter matched no row: the caller may read this repository
        // but not control it (e.g. a global repo owned by someone else). Surface
        // NotFound so a denied write confirms no more than the read already did.
        return Err(RepositoryError::NotFound(repository_id));
    }

    // Re-fetch updated row
    let updated = repositories
        .find_by_id(&repository_id, ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(updated)
}

/// Move each repository to [`Scope::Global`], dropping its owner.
///
/// One-directional: a Workspace moving back to user scope leaves its
/// repositories global, since promotion already dropped the owner to hand them
/// back to. A repository the caller cannot read (soft-deleted) is skipped
/// rather than rejected.
pub(crate) async fn promote_repositories_to_global(
    repositories: &dyn RepositoryDao,
    ctx: &ScopeContext,
    repository_ids: &[String],
) -> Result<(), RepositoryError> {
    // Check every repository before writing any: a promotion drops the owner and
    // cannot be undone, so a collision on a later one must not leave an earlier
    // one global. Two passes are still not a transaction — a DB error on the
    // write pass, or a row inserted between the passes, splits it anyway.
    let mut pending = Vec::new();
    let mut claimed: HashSet<String> = HashSet::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for repository_id in repository_ids {
        // One repository placed twice — at two branches, under two mount names —
        // is one row to promote, and its second visit would claim its own URL.
        if !seen.insert(repository_id.as_str()) {
            continue;
        }

        let Some(existing) = repositories.find_by_id(repository_id, ctx).await? else {
            continue;
        };
        if existing.scope == Scope::Global {
            continue;
        }

        ensure_unique_remote_url(
            repositories,
            Some(repository_id),
            existing.remote.as_ref().map(|remote| remote.url.as_str()),
            Scope::Global,
            None,
        )
        .await?;

        // The batch collides with itself too, and the query above cannot see it:
        // two user-scoped rows may hold one URL under different owners, which
        // `repositories_active_owner_remote_url_uq` allows and dropping both
        // owners does not.
        if let Some(remote_url) = existing.remote.as_ref().map(|remote| remote.url.as_str()) {
            if !claimed.insert(normalize_remote_url(remote_url)) {
                return Err(RepositoryError::Conflict(remote_url.to_string()));
            }
        }

        pending.push((repository_id, existing));
    }

    for (repository_id, existing) in pending {
        let patch = RepositoryPatch {
            scope: Some(Scope::Global),
            owner_user_id: Some(None),
            ..Default::default()
        };
        let applied = repositories
            .update(repository_id, &patch, ctx)
            .await
            .map_err(|error| {
                RepositoryError::from_write_error(
                    error,
                    existing.remote.as_ref().map(|remote| remote.url.as_str()),
                )
            })?;
        if !applied {
            return Err(RepositoryError::NotFound(repository_id.clone()));
        }
    }
    Ok(())
}

/// Apply the repository mutation with caller scope and existing write policy.
pub async fn convert_to_template(
    repositories: &dyn RepositoryDao,
    ctx: &ScopeContext,
    repository_id: String,
) -> Result<Repository, RepositoryError> {
    let existing = repositories
        .find_by_id(&repository_id, ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if existing.kind != RepositoryKind::Remote {
        return Err(RepositoryError::BadRequest(format!(
            "Only remote repositories can be converted to templates (current kind: {})",
            existing.kind
        )));
    }

    let patch = RepositoryPatch {
        kind: Some(RepositoryKind::Template),
        ..Default::default()
    };

    repositories.update(&repository_id, &patch, ctx).await?;

    let updated = repositories
        .find_by_id(&repository_id, ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(updated)
}

/// Apply the repository mutation with caller scope and existing write policy.
pub async fn delete_repository(
    repositories: &dyn RepositoryDao,
    db: &DbPool,
    ctx: &ScopeContext,
    repository_id: String,
) -> Result<(), RepositoryError> {
    repositories
        .find_by_id(&repository_id, ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if !repositories.soft_delete(&repository_id, ctx).await? {
        return Err(RepositoryError::NotFound(repository_id.clone()));
    }

    // Soft-delete workspaces this repo declared in git — once the source repo is
    // gone no future sync will prune them. Best-effort; the repo is already deleted.
    if let Err(e) = crate::workspace::dao::delete_declared_workspaces(db, &repository_id).await {
        warn!(repository_id = %repository_id, error = %e, "Failed to soft-delete git workspaces for deleted repo");
    }

    Ok(())
}
