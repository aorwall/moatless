//! Workspace HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use tracing::warn;

use moatless_types::access_control::{ScopeContext, ScopedResource};

use crate::repository::dao::RepositoryDao;
use moatless_backend_core::access_control::resolve_scope_for_create;
use moatless_backend_core::identity::UserResolver;
use moatless_core::http_auth::AuthUser;

use super::dao::{WorkspaceDao, WorkspacePatch};
use super::error::WorkspaceError;
use super::git_sync::{GitSyncService, SyncForce};
use super::models::{Workspace, WorkspaceKind, WorkspaceRepo, WorkspaceSource};
use super::schemas::{AddRepoRequest, CreateWorkspaceRequest, ResetGitWorkspaceResponse, UpdateWorkspaceRequest, WorkspaceRepoInput, WorkspaceResponse, resource_config_from_wire, script_configs_from_wire, server_configs_from_wire, workspace_response};

/// Serialize a typed run-config value into the jsonb column shape, dropping it
/// on the (practically impossible) serialization failure rather than erroring.
/// The `(source_repository_id, source_key)` a reset-git re-syncs from.
///
/// Without both, there is no git config to reset to: marking such a workspace
/// `source = "git"` would strand it — no sync would ever reach it (reconcile is
/// keyed on the source repo) and the write API refuses to edit or delete a
/// git-sourced workspace.
fn reset_source(ws: &Workspace) -> Result<(&str, &str), WorkspaceError> {
    match (ws.source_repository_id.as_deref(), ws.source_key.as_deref()) {
        (Some(repository_id), Some(source_key)) => Ok((repository_id, source_key)),
        _ => Err(WorkspaceError::BadRequest(
            "Workspace was not synced from Git; there is no config to reset to".to_string(),
        )),
    }
}

/// Reject mutation of a git-synced workspace — only a re-sync from git changes
/// it. The sync service writes through the DAO directly, so it bypasses this
/// guard by design.
fn ensure_workspace_mutable(ws: &Workspace) -> Result<(), WorkspaceError> {
    if ws.deleted {
        return Err(WorkspaceError::Conflict(
            "Workspace has been deleted; reset it from Git before editing it".to_string(),
        ));
    }
    if ws.source == WorkspaceSource::Git {
        return Err(WorkspaceError::Conflict(
            "Workspace is synced from Git and is read-only; edit .moatless/workspaces.json to change it".to_string(),
        ));
    }
    Ok(())
}

/// `blank` / `single` / `multi` from a placement count.
fn kind_for(count: usize) -> WorkspaceKind {
    match count {
        0 => WorkspaceKind::Blank,
        1 => WorkspaceKind::Single,
        _ => WorkspaceKind::Multi,
    }
}

/// Load a workspace (scoped) plus its placements into a response, or NotFound.
async fn load_response(
    workspaces: &Arc<dyn WorkspaceDao>,
    auth: &AuthUser,
    workspace_id: &str,
) -> Result<WorkspaceResponse, WorkspaceError> {
    let ws = workspaces
        .find_by_id(workspace_id, &auth.scope_context())
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))?;
    let repos = workspaces.find_repos(workspace_id).await?;
    Ok(workspace_response(ws, repos))
}

/// Verify the caller can see/use every referenced repository, rejecting unknown
/// or out-of-scope repos so a workspace can't compose repositories the creator
/// cannot access.
async fn ensure_repos_visible(
    repositories: &Arc<dyn RepositoryDao>,
    auth: &AuthUser,
    placements: &[WorkspaceRepoInput],
) -> Result<(), WorkspaceError> {
    let ctx = auth.scope_context();
    for p in placements {
        if repositories
            .find_by_id(&p.repository_id, &ctx)
            .await?
            .is_none()
        {
            return Err(WorkspaceError::BadRequest(format!(
                "Repository '{}' not found or not accessible",
                p.repository_id
            )));
        }
    }
    Ok(())
}

/// Restrict the two fields that select a sandbox's Kubernetes privileges to
/// admins.
///
/// `sandbox_service_account` picks the pod's ServiceAccount and
/// `network_profile` picks the NetworkPolicy that reaches the kube-apiserver.
/// Together they are what opts a workspace into a chart `sandboxAccess` profile
/// — including `namespace-admin`, which can read the app namespace's Secrets
/// (database credentials, provider API keys, session signing keys) and delete
/// workloads there. The same pair also selects the `shadowDev` identity, whose
/// bound Role already carries `secrets`.
///
/// Whether those identities *exist* is an operator decision made in Helm
/// values, but without this check any authenticated user could name one on a
/// workspace they own and inherit it, so "point exactly one workspace at it"
/// would not be an enforceable control.
///
/// Only rejects when a request actually carries the field: an ordinary
/// non-admin update (renaming, changing servers) leaves both `None` and is
/// unaffected, as is a workspace that already has them set.
fn ensure_privileged_sandbox_fields(
    ctx: &ScopeContext,
    sandbox_service_account: Option<&String>,
    network_profile: Option<&String>,
) -> Result<(), WorkspaceError> {
    if ctx.is_admin {
        return Ok(());
    }
    let attempted = match (sandbox_service_account, network_profile) {
        (Some(_), Some(_)) => "sandboxServiceAccount and networkProfile",
        (Some(_), None) => "sandboxServiceAccount",
        (None, Some(_)) => "networkProfile",
        (None, None) => return Ok(()),
    };
    Err(WorkspaceError::Forbidden(format!(
        "{attempted} may only be set by an admin — these fields grant a sandbox \
         elevated Kubernetes access"
    )))
}

/// Restrict naming the identity a Workspace runs as to admins, and require
/// that the named user exists.
///
/// Naming one makes every Task on the Workspace reach a git host with that
/// user's credential, so without this check anyone who can edit a Workspace
/// could act as anybody. This is the rule Loop activation already applies to
/// its own `run_as_user_id` (`loops/http/handlers.rs`).
///
/// An empty string clears the field and needs no user to exist, but still
/// needs admin — un-naming an identity changes which credential the Workspace
/// acts with just as much as naming one.
async fn resolve_run_as_user(
    ctx: &ScopeContext,
    users: &dyn UserResolver,
    requested: Option<&String>,
) -> Result<Option<Option<String>>, WorkspaceError> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    if !ctx.is_admin {
        return Err(WorkspaceError::Forbidden(
            "runAsUserId may only be set by an admin — it makes every task on this \
             workspace act with another account's git-host credential"
                .to_string(),
        ));
    }
    let requested = requested.trim();
    if requested.is_empty() {
        return Ok(Some(None));
    }
    users
        .canonical_id(requested)
        .await
        .map_err(|e| WorkspaceError::Db(sqlx::Error::Protocol(e.to_string())))?
        .map(|id| Some(Some(id.to_string())))
        .ok_or_else(|| {
            WorkspaceError::BadRequest(format!("Run-as user {requested} does not exist"))
        })
}

/// Reject duplicate non-null mount names within a single request payload (the
/// DB also enforces per-workspace uniqueness).
fn ensure_unique_mount_names(placements: &[WorkspaceRepoInput]) -> Result<(), WorkspaceError> {
    let mut seen = std::collections::HashSet::new();
    for p in placements {
        if let Some(mount) = p.mount_name.as_deref() {
            if !seen.insert(mount) {
                return Err(WorkspaceError::BadRequest(format!(
                    "Duplicate mount name '{mount}' in workspace"
                )));
            }
        }
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces",
    tag = "workspaces",
    request_body = CreateWorkspaceRequest,
    responses(
        (status = 201, description = "Workspace created", body = WorkspaceResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn create_workspace(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    State(repositories): State<Arc<dyn RepositoryDao>>,
    State(users): State<Arc<dyn UserResolver>>,
    auth: AuthUser,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceResponse>), WorkspaceError> {
    ensure_unique_mount_names(&req.repos)?;
    ensure_repos_visible(&repositories, &auth, &req.repos).await?;
    ensure_privileged_sandbox_fields(
        &auth.scope_context(),
        req.sandbox_service_account.as_ref(),
        req.network_profile.as_ref(),
    )?;
    let run_as_user_id = resolve_run_as_user(
        &auth.scope_context(),
        users.as_ref(),
        req.run_as_user_id.as_ref(),
    )
    .await?
    .flatten();

    let resolved = resolve_scope_for_create(&auth.scope_context(), req.scope)?;

    let workspace_id = format!("ws_{}", uuid::Uuid::new_v4());
    let row = Workspace {
        id: workspace_id.clone(),
        name: req.name,
        description: req.description,
        kind: kind_for(req.repos.len()),
        docker_image: req.docker_image,
        docker_access: req.docker_access,
        sandbox_service_account: req.sandbox_service_account,
        system_prompt: req.system_prompt,
        setup_commands: req.setup_commands,
        env_vars: req.env_vars,
        servers: req.servers.map(server_configs_from_wire).transpose()?,
        scripts: req.scripts.map(script_configs_from_wire),
        resources: req.resources.map(resource_config_from_wire).transpose()?,
        network_profile: req.network_profile,
        source: WorkspaceSource::Manual,
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
        scope: resolved.scope,
        owner_user_id: resolved.owner_user_id,
        run_as_user_id,
        created_by: Some(auth.user_id.clone().to_string()),
        deleted: false,
        created_at: None,
        updated_at: None,
    };
    workspaces.insert_workspace(&row).await?;

    // Insert placements non-primary, then elect exactly one primary so the
    // partial unique index never sees two primaries mid-insert.
    let ctx = auth.scope_context();
    let mut placement_ids: Vec<String> = Vec::with_capacity(req.repos.len());
    let mut primary_choice: Option<String> = None;
    for (i, p) in req.repos.iter().enumerate() {
        let placement_id = format!("wsr_{}", uuid::Uuid::new_v4());
        workspaces
            .add_repo(
                &WorkspaceRepo {
                    id: placement_id.clone(),
                    workspace_id: workspace_id.clone(),
                    repository_id: p.repository_id.clone(),
                    branch: p.branch.clone(),
                    mount_name: p.mount_name.clone(),
                    is_primary: false,
                    position: p.position.unwrap_or(i as i32),
                    created_at: None,
                },
                &ctx,
            )
            .await?;
        if primary_choice.is_none() && p.is_primary == Some(true) {
            primary_choice = Some(placement_id.clone());
        }
        placement_ids.push(placement_id);
    }
    // Default the first placement to primary when the caller flagged none.
    if let Some(target) = primary_choice.or_else(|| placement_ids.first().cloned()) {
        workspaces.set_primary(&workspace_id, &target, &ctx).await?;
    }

    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok((StatusCode::CREATED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces",
    tag = "workspaces",
    responses(
        (status = 200, description = "Workspaces", body = Vec<WorkspaceResponse>)
    )
)]
pub async fn list_workspaces(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
) -> Result<Json<Vec<WorkspaceResponse>>, WorkspaceError> {
    let rows = workspaces.list(&auth.scope_context()).await?;
    let ids: Vec<String> = rows.iter().map(|w| w.id.clone()).collect();
    let all_repos = workspaces.find_repos_for_workspaces(&ids).await?;

    let responses: Vec<WorkspaceResponse> = rows
        .into_iter()
        .map(|w| {
            let repos: Vec<WorkspaceRepo> = all_repos
                .iter()
                .filter(|r| r.workspace_id == w.id)
                .cloned()
                .collect();
            workspace_response(w, repos)
        })
        .collect();
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 200, description = "Workspace", body = WorkspaceResponse),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_workspace(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    patch,
    path = "/api/v1/workspaces/{workspace_id}",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    request_body = UpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Workspace updated", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_workspace(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    State(users): State<Arc<dyn UserResolver>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    let ctx = auth.scope_context();
    let existing = workspaces
        .find_by_id(&workspace_id, &ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;
    ensure_workspace_mutable(&existing)?;
    ensure_privileged_sandbox_fields(
        &ctx,
        req.sandbox_service_account.as_ref(),
        req.network_profile.as_ref(),
    )?;
    let run_as_user_id =
        resolve_run_as_user(&ctx, users.as_ref(), req.run_as_user_id.as_ref()).await?;

    // Re-resolve owner_user_id when the scope changes.
    let (scope, owner_user_id) = match req.scope {
        Some(new_scope) => {
            let resolved = resolve_scope_for_create(&auth.scope_context(), Some(new_scope))?;
            (Some(resolved.scope), Some(resolved.owner_user_id))
        }
        None => (None, None),
    };

    let patch = WorkspacePatch {
        name: req.name,
        description: req.description.map(Some),
        scope,
        owner_user_id,
        docker_image: req.docker_image.map(Some),
        docker_access: req.docker_access.map(Some),
        sandbox_service_account: req.sandbox_service_account.map(Some),
        system_prompt: req.system_prompt.map(Some),
        setup_commands: req.setup_commands.map(Some),
        env_vars: req.env_vars.map(Some),
        servers: req
            .servers
            .map(|configs| server_configs_from_wire(configs).map(Some))
            .transpose()?,
        scripts: req
            .scripts
            .map(|configs| Some(script_configs_from_wire(configs))),
        resources: req
            .resources
            .map(|config| resource_config_from_wire(config).map(Some))
            .transpose()?,
        network_profile: req.network_profile.map(Some),
        run_as_user_id,
        kind: None,
        // Provenance not editable via the write API.
        source: None,
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
    };
    workspaces.update(&workspace_id, &patch, &ctx).await?;

    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    delete,
    path = "/api/v1/workspaces/{workspace_id}",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 204, description = "Workspace deleted"),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_workspace(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<StatusCode, WorkspaceError> {
    let ctx = auth.scope_context();
    let existing = workspaces
        .find_by_id(&workspace_id, &ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;

    let deleted = if existing.source_repository_id.is_some() && existing.source_key.is_some() {
        workspaces
            .soft_delete_git_origin(&workspace_id, &ctx)
            .await?
    } else {
        ensure_workspace_mutable(&existing)?;
        workspaces.soft_delete(&workspace_id, &ctx).await?
    };
    if !deleted {
        return Err(WorkspaceError::NotFound(workspace_id));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/api/v1/workspaces/{workspace_id}/override",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    request_body = UpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Workspace overridden; source set to manual", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
/// Override a git-synced workspace. Sets `source = "manual"` so that git sync
/// skips this workspace going forward. Provenance fields are preserved.
/// Use `POST /{id}/reset-git` to re-enable git sync.
pub async fn override_workspace(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    State(users): State<Arc<dyn UserResolver>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    let ctx = auth.scope_context();
    let existing = workspaces
        .find_by_id(&workspace_id, &ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;
    if existing.deleted {
        return Err(WorkspaceError::Conflict(
            "Workspace has been deleted; reset it from Git before overriding it".to_string(),
        ));
    }
    // Intentionally skipping `ensure_workspace_mutable` — this endpoint is the
    // explicit way to take ownership of a git-synced workspace. The DAO
    // control_filter ensures only the owner/admin can do this.
    //
    // The privileged-field gate is NOT skipped: taking ownership of a workspace
    // is not the same as being allowed to elevate it.
    ensure_privileged_sandbox_fields(
        &ctx,
        req.sandbox_service_account.as_ref(),
        req.network_profile.as_ref(),
    )?;
    let run_as_user_id =
        resolve_run_as_user(&ctx, users.as_ref(), req.run_as_user_id.as_ref()).await?;

    let patch = WorkspacePatch {
        name: req.name,
        description: req.description.map(Some),
        docker_image: req.docker_image.map(Some),
        docker_access: req.docker_access.map(Some),
        sandbox_service_account: req.sandbox_service_account.map(Some),
        system_prompt: req.system_prompt.map(Some),
        setup_commands: req.setup_commands.map(Some),
        env_vars: req.env_vars.map(Some),
        servers: req
            .servers
            .map(|configs| server_configs_from_wire(configs).map(Some))
            .transpose()?,
        scripts: req
            .scripts
            .map(|configs| Some(script_configs_from_wire(configs))),
        resources: req
            .resources
            .map(|config| resource_config_from_wire(config).map(Some))
            .transpose()?,
        network_profile: req.network_profile.map(Some),
        run_as_user_id,
        // Mark as manually managed; provenance columns stay unchanged.
        source: Some(WorkspaceSource::Manual),
        kind: None,
        scope: None,
        owner_user_id: None,
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
    };
    workspaces.update(&workspace_id, &patch, &ctx).await?;

    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/reset-git",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 200, description = "Re-sync attempted; `resynced` reports whether it took effect", body = ResetGitWorkspaceResponse),
        (status = 400, description = "Workspace was never synced from git", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
/// Discard a workspace's local override and take its run-config from git again.
///
/// Implemented as a re-sync of just this workspace rather than a flag flip, so
/// `source` only returns to `"git"` when the repository still declares the
/// entry — a workspace can never end up claiming git provenance while holding
/// config git never produced.
pub async fn reset_git_workspace(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    State(repositories): State<Arc<dyn RepositoryDao>>,
    State(git_sync): State<Arc<GitSyncService>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<Json<ResetGitWorkspaceResponse>, WorkspaceError> {
    let ctx = auth.scope_context();
    let existing = workspaces
        .find_by_id(&workspace_id, &ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;

    // The sync writes with a system context, so control is checked here.
    let resource = ScopedResource::borrowed(
        existing.scope,
        existing
            .owner_user_id
            .as_deref()
            .or(existing.created_by.as_deref()),
    );
    if !ctx.can_control(&resource) {
        return Err(WorkspaceError::Forbidden(format!(
            "No control access to workspace '{workspace_id}'"
        )));
    }

    let (source_repository_id, source_key) = reset_source(&existing)?;
    // An already-git workspace has nothing to reclaim — for it the re-sync is
    // just a refresh, and success means the sync ran.
    let was_overridden = existing.source == WorkspaceSource::Manual;

    let repo = repositories
        .find_by_id(source_repository_id, &ctx)
        .await
        .map_err(WorkspaceError::Db)?;
    let error = match repo {
        None => Some(format!(
            "Source repository '{source_repository_id}' not found or not accessible"
        )),
        Some(repo) => {
            let force = SyncForce::Only(workspace_id.clone());
            match git_sync.sync_repo_workspaces(&repo, &auth, &force).await {
                // `Only` reclaims at most this one workspace, so a non-zero
                // count is proof that *this* workspace was rewritten from git —
                // `updated` would also count the repo's other workspaces.
                Ok(summary) if !was_overridden || summary.reclaimed > 0 => None,
                Ok(_) => Some(format!(
                    "'{source_key}' is no longer declared in {}",
                    existing
                        .source_config_path
                        .as_deref()
                        .unwrap_or(".moatless/workspaces.json")
                )),
                Err(e) => {
                    warn!(%workspace_id, error = %e, "Reset-git re-sync failed");
                    Some(format!("Failed to read the config from Git: {e}"))
                }
            }
        }
    };

    let workspace = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(ResetGitWorkspaceResponse {
        resynced: error.is_none(),
        error,
        workspace,
    }))
}

/// Look up a workspace (scoped) and assert the caller may mutate it.
async fn ensure_writable(
    workspaces: &Arc<dyn WorkspaceDao>,
    auth: &AuthUser,
    workspace_id: &str,
) -> Result<(), WorkspaceError> {
    let existing = workspaces
        .find_by_id(workspace_id, &auth.scope_context())
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))?;
    ensure_workspace_mutable(&existing)?;
    // Control is enforced in the DAO via control_filter; the read above just
    // provides an early NotFound / git-sync guard.
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/repos",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    request_body = AddRepoRequest,
    responses(
        (status = 200, description = "Repo added", body = WorkspaceResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn add_workspace_repo(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    State(repositories): State<Arc<dyn RepositoryDao>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<AddRepoRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    ensure_writable(&workspaces, &auth, &workspace_id).await?;

    if repositories
        .find_by_id(&req.repository_id, &auth.scope_context())
        .await?
        .is_none()
    {
        return Err(WorkspaceError::BadRequest(format!(
            "Repository '{}' not found or not accessible",
            req.repository_id
        )));
    }

    let existing = workspaces.find_repos(&workspace_id).await?;
    if let Some(mount) = req.mount_name.as_deref() {
        if existing
            .iter()
            .any(|r| r.mount_name.as_deref() == Some(mount))
        {
            return Err(WorkspaceError::BadRequest(format!(
                "Duplicate mount name '{mount}' in workspace"
            )));
        }
    }

    let placement_id = format!("wsr_{}", uuid::Uuid::new_v4());
    let position = req.position.unwrap_or(existing.len() as i32);
    let ctx = auth.scope_context();
    workspaces
        .add_repo(
            &WorkspaceRepo {
                id: placement_id.clone(),
                workspace_id: workspace_id.clone(),
                repository_id: req.repository_id,
                branch: req.branch,
                mount_name: req.mount_name,
                is_primary: false,
                position,
                created_at: None,
            },
            &ctx,
        )
        .await?;

    // First repo in a previously-blank workspace, or an explicit request, makes
    // this placement primary.
    if req.is_primary == Some(true) || existing.is_empty() {
        workspaces
            .set_primary(&workspace_id, &placement_id, &ctx)
            .await?;
    }
    workspaces.refresh_kind(&workspace_id, &ctx).await?;

    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    delete,
    path = "/api/v1/workspaces/{workspace_id}/repos/{placement_id}",
    tag = "workspaces",
    params(
        ("workspace_id" = String, Path, description = "Workspace id"),
        ("placement_id" = String, Path, description = "Placement id")
    ),
    responses(
        (status = 200, description = "Repo removed", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn remove_workspace_repo(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Path((workspace_id, placement_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    ensure_writable(&workspaces, &auth, &workspace_id).await?;

    let ctx = auth.scope_context();
    let removed = workspaces
        .remove_repo(&workspace_id, &placement_id, &ctx)
        .await?;
    if !removed {
        return Err(WorkspaceError::NotFound(placement_id));
    }
    workspaces.refresh_kind(&workspace_id, &ctx).await?;

    // Preserve the "exactly one primary when n>=1" invariant if we removed the
    // primary: promote the next-best remaining placement.
    let repos = workspaces.find_repos(&workspace_id).await?;
    if !repos.is_empty() && !repos.iter().any(|r| r.is_primary) {
        workspaces
            .set_primary(&workspace_id, &repos[0].id, &ctx)
            .await?;
    }

    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/repos/{placement_id}/primary",
    tag = "workspaces",
    params(
        ("workspace_id" = String, Path, description = "Workspace id"),
        ("placement_id" = String, Path, description = "Placement id")
    ),
    responses(
        (status = 200, description = "Primary set", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn set_workspace_primary_repo(
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Path((workspace_id, placement_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    ensure_writable(&workspaces, &auth, &workspace_id).await?;

    let ctx = auth.scope_context();
    let ok = workspaces
        .set_primary(&workspace_id, &placement_id, &ctx)
        .await?;
    if !ok {
        return Err(WorkspaceError::NotFound(placement_id));
    }

    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(mount: Option<&str>) -> WorkspaceRepoInput {
        WorkspaceRepoInput {
            repository_id: "repo".to_string(),
            branch: None,
            mount_name: mount.map(str::to_string),
            is_primary: None,
            position: None,
        }
    }

    fn ctx(is_admin: bool) -> ScopeContext {
        ScopeContext {
            user_id: "u1".to_string(),
            is_admin,
        }
    }

    /// The two fields that opt a workspace into an elevated `sandboxAccess`
    /// profile are admin-only, so enabling a profile in Helm values stays an
    /// operator decision rather than becoming self-serve.
    #[test]
    fn privileged_sandbox_fields_are_admin_only() {
        let sa = "moatless-admin".to_string();
        let profile = "moatless-admin".to_string();

        for (account, network) in [
            (Some(&sa), None),
            (None, Some(&profile)),
            (Some(&sa), Some(&profile)),
        ] {
            let err = ensure_privileged_sandbox_fields(&ctx(false), account, network).unwrap_err();
            assert!(
                matches!(err, WorkspaceError::Forbidden(_)),
                "non-admin must not select a privileged sandbox identity"
            );
            assert!(
                ensure_privileged_sandbox_fields(&ctx(true), account, network).is_ok(),
                "admin may select a privileged sandbox identity"
            );
        }
    }

    /// An ordinary edit that never mentions the fields is untouched — otherwise
    /// a non-admin could not rename their own workspace.
    #[test]
    fn absent_privileged_fields_are_allowed_for_anyone() {
        assert!(ensure_privileged_sandbox_fields(&ctx(false), None, None).is_ok());
    }

    #[test]
    fn kind_for_derives_label_from_count() {
        assert_eq!(kind_for(0), WorkspaceKind::Blank);
        assert_eq!(kind_for(1), WorkspaceKind::Single);
        assert_eq!(kind_for(2), WorkspaceKind::Multi);
        assert_eq!(kind_for(9), WorkspaceKind::Multi);
    }

    fn workspace_with_source(source: WorkspaceSource) -> Workspace {
        Workspace {
            id: "ws_1".to_string(),
            name: "w".to_string(),
            description: None,
            kind: WorkspaceKind::Single,
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            scripts: None,
            resources: None,
            network_profile: None,
            source,
            source_repository_id: None,
            source_key: None,
            source_config_path: None,
            scope: moatless_types::access_control::Scope::User,
            owner_user_id: None,
            run_as_user_id: None,
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn git_workspace_is_read_only() {
        let err = ensure_workspace_mutable(&workspace_with_source(WorkspaceSource::Git)).unwrap_err();
        assert!(matches!(err, WorkspaceError::Conflict(_)));
    }

    #[test]
    fn manual_workspace_is_mutable() {
        assert!(ensure_workspace_mutable(&workspace_with_source(WorkspaceSource::Manual)).is_ok());
    }

    #[test]
    fn deleted_workspace_is_not_mutable() {
        let mut workspace = workspace_with_source(WorkspaceSource::Manual);
        workspace.deleted = true;

        let err = ensure_workspace_mutable(&workspace).unwrap_err();
        assert!(matches!(err, WorkspaceError::Conflict(_)));
    }

    #[test]
    fn reset_source_requires_full_provenance() {
        // A workspace that was never git-synced has nothing to reset to.
        let err = reset_source(&workspace_with_source(WorkspaceSource::Manual)).unwrap_err();
        assert!(matches!(err, WorkspaceError::BadRequest(_)));

        // A half-stamped row is just as unreachable for the sync.
        let mut half = workspace_with_source(WorkspaceSource::Manual);
        half.source_repository_id = Some("repo-1".to_string());
        assert!(reset_source(&half).is_err());

        let mut overridden = workspace_with_source(WorkspaceSource::Manual);
        overridden.source_repository_id = Some("repo-1".to_string());
        overridden.source_key = Some("api".to_string());
        assert_eq!(reset_source(&overridden).unwrap(), ("repo-1", "api"));
    }

    #[test]
    fn unique_mount_names_accepts_distinct_and_nulls() {
        let placements = vec![
            input(Some("api")),
            input(Some("web")),
            input(None),
            input(None),
        ];
        assert!(ensure_unique_mount_names(&placements).is_ok());
    }

    #[test]
    fn unique_mount_names_rejects_duplicates() {
        let placements = vec![input(Some("api")), input(Some("api"))];
        let err = ensure_unique_mount_names(&placements).unwrap_err();
        assert!(matches!(err, WorkspaceError::BadRequest(_)));
    }
}
