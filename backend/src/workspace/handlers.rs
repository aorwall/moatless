//! Workspace HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use moatless_backend_core::scope::ScopedResource;

use crate::access_control::{ensure_can_write_resource, resolve_scope_for_create};
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::dao::WorkspacePatch;
use super::error::WorkspaceError;
use super::models::{Workspace, WorkspaceRepo};
use super::schemas::{
    workspace_response, AddRepoRequest, CreateWorkspaceRequest, UpdateWorkspaceRequest,
    WorkspaceRepoInput, WorkspaceResponse,
};

/// Serialize a typed run-config value into the jsonb column shape, dropping it
/// on the (practically impossible) serialization failure rather than erroring.
fn to_json<T: Serialize>(value: Option<T>) -> Option<serde_json::Value> {
    value.and_then(|v| serde_json::to_value(v).ok())
}

/// `blank` / `single` / `multi` from a placement count.
fn kind_for(count: usize) -> String {
    match count {
        0 => "blank",
        1 => "single",
        _ => "multi",
    }
    .to_string()
}

/// Load a workspace (scoped) plus its placements into a response, or NotFound.
async fn load_response(
    state: &AppState,
    auth: &AuthUser,
    workspace_id: &str,
) -> Result<WorkspaceResponse, WorkspaceError> {
    let ws = state
        .workspaces
        .find_by_id_scoped(workspace_id, &auth.scope_context())
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))?;
    let repos = state.workspaces.find_repos(workspace_id).await?;
    Ok(workspace_response(ws, repos))
}

/// Verify the caller can see/use every referenced repository, rejecting unknown
/// or out-of-scope repos so a workspace can't compose repositories the creator
/// cannot access.
async fn ensure_repos_visible(
    state: &AppState,
    auth: &AuthUser,
    placements: &[WorkspaceRepoInput],
) -> Result<(), WorkspaceError> {
    let ctx = auth.scope_context();
    for p in placements {
        if state
            .repositories
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
        (status = 400, description = "Invalid request", body = crate::error::ErrorBody),
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody)
    )
)]
pub async fn create_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceResponse>), WorkspaceError> {
    ensure_unique_mount_names(&req.repos)?;
    ensure_repos_visible(&state, &auth, &req.repos).await?;

    let resolved = resolve_scope_for_create(&state, &auth, req.scope, req.team_id.clone()).await?;

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
        setup_commands: to_json(req.setup_commands),
        env_vars: to_json(req.env_vars),
        servers: to_json(req.servers),
        plugins: to_json(req.plugins),
        resources: to_json(req.resources),
        scope: resolved.scope,
        team_id: Some(resolved.team_id),
        owner_user_id: resolved.owner_user_id,
        created_by: Some(auth.user_id.clone()),
        deleted: Some(false),
        created_at: None,
        updated_at: None,
    };
    state.workspaces.insert_workspace(&row).await?;

    // Insert placements non-primary, then elect exactly one primary so the
    // partial unique index never sees two primaries mid-insert.
    let mut placement_ids: Vec<String> = Vec::with_capacity(req.repos.len());
    let mut primary_choice: Option<String> = None;
    for (i, p) in req.repos.iter().enumerate() {
        let placement_id = format!("wsr_{}", uuid::Uuid::new_v4());
        state
            .workspaces
            .add_repo(&WorkspaceRepo {
                id: placement_id.clone(),
                workspace_id: workspace_id.clone(),
                repository_id: p.repository_id.clone(),
                branch: p.branch.clone(),
                mount_name: p.mount_name.clone(),
                is_primary: false,
                position: p.position.unwrap_or(i as i32),
                created_at: None,
            })
            .await?;
        if primary_choice.is_none() && p.is_primary == Some(true) {
            primary_choice = Some(placement_id.clone());
        }
        placement_ids.push(placement_id);
    }
    // Default the first placement to primary when the caller flagged none.
    if let Some(target) = primary_choice.or_else(|| placement_ids.first().cloned()) {
        state.workspaces.set_primary(&workspace_id, &target).await?;
    }

    let response = load_response(&state, &auth, &workspace_id).await?;
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
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<WorkspaceResponse>>, WorkspaceError> {
    let workspaces = state.workspaces.list(&auth.scope_context()).await?;
    let ids: Vec<String> = workspaces.iter().map(|w| w.id.clone()).collect();
    let all_repos = state.workspaces.find_repos_for_workspaces(&ids).await?;

    let responses: Vec<WorkspaceResponse> = workspaces
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
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    let response = load_response(&state, &auth, &workspace_id).await?;
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
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    let existing = state
        .workspaces
        .find_by_id_scoped(&workspace_id, &auth.scope_context())
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;
    ensure_can_write_resource(
        &state,
        &auth,
        &ScopedResource::borrowed(
            existing.scope,
            existing.team_id.as_deref(),
            existing
                .owner_user_id
                .as_deref()
                .or(existing.created_by.as_deref()),
        ),
        "workspace",
    )
    .await?;

    // Re-resolve team_id/owner_user_id when the scope changes.
    let (scope, team_id, owner_user_id) = match req.scope {
        Some(new_scope) => {
            let resolved =
                resolve_scope_for_create(&state, &auth, Some(new_scope), req.team_id.clone())
                    .await?;
            (
                Some(resolved.scope),
                Some(Some(resolved.team_id)),
                Some(resolved.owner_user_id),
            )
        }
        None => (None, None, None),
    };

    let patch = WorkspacePatch {
        name: req.name,
        description: req.description.map(Some),
        scope,
        team_id,
        owner_user_id,
        docker_image: req.docker_image.map(Some),
        docker_access: req.docker_access.map(Some),
        sandbox_service_account: req.sandbox_service_account.map(Some),
        system_prompt: req.system_prompt.map(Some),
        setup_commands: req.setup_commands.map(|v| to_json(Some(v))),
        env_vars: req.env_vars.map(|v| to_json(Some(v))),
        servers: req.servers.map(|v| to_json(Some(v))),
        plugins: req.plugins.map(|v| to_json(Some(v))),
        resources: req.resources.map(|v| to_json(Some(v))),
        kind: None,
    };
    state.workspaces.update(&workspace_id, &patch).await?;

    let response = load_response(&state, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    delete,
    path = "/api/v1/workspaces/{workspace_id}",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 204, description = "Workspace deleted"),
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<StatusCode, WorkspaceError> {
    let existing = state
        .workspaces
        .find_by_id_scoped(&workspace_id, &auth.scope_context())
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;
    ensure_can_write_resource(
        &state,
        &auth,
        &ScopedResource::borrowed(
            existing.scope,
            existing.team_id.as_deref(),
            existing
                .owner_user_id
                .as_deref()
                .or(existing.created_by.as_deref()),
        ),
        "workspace",
    )
    .await?;

    state.workspaces.soft_delete(&workspace_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Look up a workspace (scoped) and assert the caller may mutate it.
async fn ensure_writable(
    state: &AppState,
    auth: &AuthUser,
    workspace_id: &str,
) -> Result<(), WorkspaceError> {
    let existing = state
        .workspaces
        .find_by_id_scoped(workspace_id, &auth.scope_context())
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))?;
    ensure_can_write_resource(
        state,
        auth,
        &ScopedResource::borrowed(
            existing.scope,
            existing.team_id.as_deref(),
            existing
                .owner_user_id
                .as_deref()
                .or(existing.created_by.as_deref()),
        ),
        "workspace",
    )
    .await?;
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
        (status = 400, description = "Invalid request", body = crate::error::ErrorBody),
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn add_workspace_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<AddRepoRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    ensure_writable(&state, &auth, &workspace_id).await?;

    if state
        .repositories
        .find_by_id(&req.repository_id, &auth.scope_context())
        .await?
        .is_none()
    {
        return Err(WorkspaceError::BadRequest(format!(
            "Repository '{}' not found or not accessible",
            req.repository_id
        )));
    }

    let existing = state.workspaces.find_repos(&workspace_id).await?;
    if let Some(mount) = req.mount_name.as_deref() {
        if existing.iter().any(|r| r.mount_name.as_deref() == Some(mount)) {
            return Err(WorkspaceError::BadRequest(format!(
                "Duplicate mount name '{mount}' in workspace"
            )));
        }
    }

    let placement_id = format!("wsr_{}", uuid::Uuid::new_v4());
    let position = req.position.unwrap_or(existing.len() as i32);
    state
        .workspaces
        .add_repo(&WorkspaceRepo {
            id: placement_id.clone(),
            workspace_id: workspace_id.clone(),
            repository_id: req.repository_id,
            branch: req.branch,
            mount_name: req.mount_name,
            is_primary: false,
            position,
            created_at: None,
        })
        .await?;

    // First repo in a previously-blank workspace, or an explicit request, makes
    // this placement primary.
    if req.is_primary == Some(true) || existing.is_empty() {
        state
            .workspaces
            .set_primary(&workspace_id, &placement_id)
            .await?;
    }
    state.workspaces.refresh_kind(&workspace_id).await?;

    let response = load_response(&state, &auth, &workspace_id).await?;
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
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn remove_workspace_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((workspace_id, placement_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    ensure_writable(&state, &auth, &workspace_id).await?;

    let removed = state
        .workspaces
        .remove_repo(&workspace_id, &placement_id)
        .await?;
    if !removed {
        return Err(WorkspaceError::NotFound(placement_id));
    }
    state.workspaces.refresh_kind(&workspace_id).await?;

    // Preserve the "exactly one primary when n>=1" invariant if we removed the
    // primary: promote the next-best remaining placement.
    let repos = state.workspaces.find_repos(&workspace_id).await?;
    if !repos.is_empty() && !repos.iter().any(|r| r.is_primary) {
        state
            .workspaces
            .set_primary(&workspace_id, &repos[0].id)
            .await?;
    }

    let response = load_response(&state, &auth, &workspace_id).await?;
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
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn set_workspace_primary_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((workspace_id, placement_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    ensure_writable(&state, &auth, &workspace_id).await?;

    let ok = state
        .workspaces
        .set_primary(&workspace_id, &placement_id)
        .await?;
    if !ok {
        return Err(WorkspaceError::NotFound(placement_id));
    }

    let response = load_response(&state, &auth, &workspace_id).await?;
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

    #[test]
    fn kind_for_derives_label_from_count() {
        assert_eq!(kind_for(0), "blank");
        assert_eq!(kind_for(1), "single");
        assert_eq!(kind_for(2), "multi");
        assert_eq!(kind_for(9), "multi");
    }

    #[test]
    fn unique_mount_names_accepts_distinct_and_nulls() {
        let placements = vec![input(Some("api")), input(Some("web")), input(None), input(None)];
        assert!(ensure_unique_mount_names(&placements).is_ok());
    }

    #[test]
    fn unique_mount_names_rejects_duplicates() {
        let placements = vec![input(Some("api")), input(Some("api"))];
        let err = ensure_unique_mount_names(&placements).unwrap_err();
        assert!(matches!(err, WorkspaceError::BadRequest(_)));
    }

    #[test]
    fn to_json_roundtrips_and_drops_none() {
        assert_eq!(to_json::<Vec<String>>(None), None);
        let v = to_json(Some(vec!["a".to_string(), "b".to_string()])).unwrap();
        assert_eq!(v, serde_json::json!(["a", "b"]));
    }
}
