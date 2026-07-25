//! Plugin repo HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use moatless_backend_core::scope::ScopedResource;

use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::PluginRepoError;
use super::models::PluginRepo;
use super::schemas::{CreatePluginRepoRequest, PluginRepoResponse};

/// A plugin repo is loadable code (hooks/commands run inside sandboxes), so
/// registering or removing one is a platform-wide trust decision restricted to
/// admins. Linking an already-registered plugin to a repository is gated
/// separately by repository write access (see `ensure_repo_control`).
fn ensure_admin(auth: &AuthUser) -> Result<(), PluginRepoError> {
    if auth.scope_context().is_admin {
        Ok(())
    } else {
        Err(PluginRepoError::Forbidden(
            "Admin role required to manage the plugin registry".to_string(),
        ))
    }
}

/// Ensure the caller may write/control the target repository before allowing a
/// plugin link/unlink — otherwise any user could attach loadable plugin code to
/// a repository (and its sandboxes) they don't own.
async fn ensure_repo_control(
    state: &AppState,
    auth: &AuthUser,
    repo_id: &str,
) -> Result<(), PluginRepoError> {
    let ctx = auth.scope_context();
    let repo = state
        .repositories
        .find_by_id(repo_id, &ctx)
        .await?
        .ok_or_else(|| PluginRepoError::NotFound(format!("Repository '{repo_id}' not found")))?;
    let resource = ScopedResource::borrowed(
        repo.scope,
        repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
    );
    if !ctx.can_control(&resource) {
        return Err(PluginRepoError::Forbidden(format!(
            "No control access to repository '{repo_id}'"
        )));
    }
    Ok(())
}

/// List all plugin repos.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-repos",
    tag = "plugin-repos",
    responses(
        (status = 200, description = "List of plugin repos", body = Vec<PluginRepoResponse>),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_plugin_repos(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<PluginRepoResponse>>, PluginRepoError> {
    let rows = state.plugin_repos.list_all().await?;
    let items = rows.into_iter().map(to_response).collect();
    Ok(Json(items))
}

/// Create a new plugin repo.
#[utoipa::path(
    post,
    path = "/api/v1/plugin-repos",
    tag = "plugin-repos",
    request_body = CreatePluginRepoRequest,
    responses(
        (status = 200, description = "Created plugin repo", body = PluginRepoResponse),
        (status = 409, description = "Name already exists", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn create_plugin_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreatePluginRepoRequest>,
) -> Result<Json<PluginRepoResponse>, PluginRepoError> {
    ensure_admin(&auth)?;
    let id = uuid::Uuid::new_v4().to_string();
    let row = PluginRepo {
        id: id.clone(),
        name: req.name,
        git_url: req.git_url,
        created_at: None,
        updated_at: None,
    };

    if let Err(e) = state.plugin_repos.insert(&row).await {
        // Check for unique constraint violation on name
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.constraint().is_some() {
                return Err(PluginRepoError::Conflict(
                    "A plugin repo with that name already exists".to_string(),
                ));
            }
        }
        return Err(e.into());
    }

    Ok(Json(to_response(row)))
}

/// Delete a plugin repo.
#[utoipa::path(
    delete,
    path = "/api/v1/plugin-repos/{id}",
    tag = "plugin-repos",
    params(("id" = String, Path, description = "Plugin repo ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_plugin_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, PluginRepoError> {
    ensure_admin(&auth)?;
    let deleted = state.plugin_repos.delete(&id).await?;
    if !deleted {
        return Err(PluginRepoError::NotFound(format!(
            "Plugin repo '{id}' not found"
        )));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// List plugins linked to a repository.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repo_id}/plugins",
    tag = "plugin-repos",
    params(("repo_id" = String, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Linked plugins", body = Vec<PluginRepoResponse>),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_repo_plugins(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<String>,
) -> Result<Json<Vec<PluginRepoResponse>>, PluginRepoError> {
    // Only expose a repository's linked plugins to callers who can access it.
    state
        .repositories
        .find_by_id(&repo_id, &auth.scope_context())
        .await?
        .ok_or_else(|| PluginRepoError::NotFound(format!("Repository '{repo_id}' not found")))?;
    let rows = state.plugin_repos.find_by_repository_id(&repo_id).await?;
    let items = rows.into_iter().map(to_response).collect();
    Ok(Json(items))
}

/// Link a plugin to a repository.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{repo_id}/plugins/{plugin_id}",
    tag = "plugin-repos",
    params(
        ("repo_id" = String, Path, description = "Repository ID"),
        ("plugin_id" = String, Path, description = "Plugin repo ID"),
    ),
    responses(
        (status = 204, description = "Linked"),
        (status = 404, description = "Plugin not found", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn link_plugin(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, plugin_id)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, PluginRepoError> {
    // Linking loadable plugin code to a repository requires control of that
    // repository (its sandboxes will clone and execute the plugin's hooks).
    ensure_repo_control(&state, &auth, &repo_id).await?;

    // Verify plugin exists
    state
        .plugin_repos
        .find_by_id(&plugin_id)
        .await?
        .ok_or_else(|| PluginRepoError::NotFound(format!("Plugin repo '{plugin_id}' not found")))?;

    state.plugin_repos.link(&repo_id, &plugin_id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Unlink a plugin from a repository.
#[utoipa::path(
    delete,
    path = "/api/v1/repositories/{repo_id}/plugins/{plugin_id}",
    tag = "plugin-repos",
    params(
        ("repo_id" = String, Path, description = "Repository ID"),
        ("plugin_id" = String, Path, description = "Plugin repo ID"),
    ),
    responses(
        (status = 204, description = "Unlinked"),
        (status = 404, description = "Link not found", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn unlink_plugin(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, plugin_id)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, PluginRepoError> {
    ensure_repo_control(&state, &auth, &repo_id).await?;
    let removed = state.plugin_repos.unlink(&repo_id, &plugin_id).await?;
    if !removed {
        return Err(PluginRepoError::NotFound("Link not found".to_string()));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn to_response(r: PluginRepo) -> PluginRepoResponse {
    PluginRepoResponse {
        id: r.id,
        name: r.name,
        git_url: r.git_url,
    }
}
