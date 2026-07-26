//! Plugin HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use moatless_backend_core::scope::ScopedResource;

use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::PluginError;
use super::models::Plugin;
use super::schemas::{CreatePluginRequest, PluginResponse, PluginSkillResponse};

/// A plugin is loadable code (hooks/commands run inside sandboxes), so
/// registering or removing one is a platform-wide trust decision restricted to
/// admins. Linking an already-registered plugin to a repository is gated
/// separately by repository write access (see `ensure_repo_control`).
fn ensure_admin(auth: &AuthUser) -> Result<(), PluginError> {
    if auth.scope_context().is_admin {
        Ok(())
    } else {
        Err(PluginError::Forbidden(
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
) -> Result<(), PluginError> {
    let ctx = auth.scope_context();
    let repo = state
        .repositories
        .find_by_id(repo_id, &ctx)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Repository '{repo_id}' not found")))?;
    let resource = ScopedResource::borrowed(
        repo.scope,
        repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
    );
    if !ctx.can_control(&resource) {
        return Err(PluginError::Forbidden(format!(
            "No control access to repository '{repo_id}'"
        )));
    }
    Ok(())
}

/// List all plugins.
#[utoipa::path(
    get,
    path = "/api/v1/plugins",
    tag = "plugins",
    responses(
        (status = 200, description = "List of plugins", body = Vec<PluginResponse>),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_plugins(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<PluginResponse>>, PluginError> {
    let rows = state.plugins.list_all().await?;
    let items = rows.into_iter().map(to_response).collect();
    Ok(Json(items))
}

/// Create a new plugin.
#[utoipa::path(
    post,
    path = "/api/v1/plugins",
    tag = "plugins",
    request_body = CreatePluginRequest,
    responses(
        (status = 200, description = "Created plugin", body = PluginResponse),
        (status = 409, description = "Name already exists", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn create_plugin(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreatePluginRequest>,
) -> Result<Json<PluginResponse>, PluginError> {
    ensure_admin(&auth)?;
    let id = uuid::Uuid::new_v4().to_string();
    let row = Plugin {
        id: id.clone(),
        name: req.name,
        git_url: req.git_url,
        synced_version: None,
        synced_at: None,
        created_at: None,
        updated_at: None,
    };

    if let Err(e) = state.plugins.insert(&row).await {
        // Check for unique constraint violation on name
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.constraint().is_some() {
                return Err(PluginError::Conflict(
                    "A plugin with that name already exists".to_string(),
                ));
            }
        }
        return Err(e.into());
    }

    Ok(Json(to_response(row)))
}

/// Delete a plugin.
#[utoipa::path(
    delete,
    path = "/api/v1/plugins/{id}",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_plugin(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, PluginError> {
    ensure_admin(&auth)?;
    let deleted = state.plugins.delete(&id).await?;
    if !deleted {
        return Err(PluginError::NotFound(format!(
            "Plugin '{id}' not found"
        )));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// List plugins linked to a repository.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repo_id}/plugins",
    tag = "plugins",
    params(("repo_id" = String, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Linked plugins", body = Vec<PluginResponse>),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_repo_plugins(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<String>,
) -> Result<Json<Vec<PluginResponse>>, PluginError> {
    // Only expose a repository's linked plugins to callers who can access it.
    state
        .repositories
        .find_by_id(&repo_id, &auth.scope_context())
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Repository '{repo_id}' not found")))?;
    let rows = state.plugins.find_by_repository_id(&repo_id).await?;
    let items = rows.into_iter().map(to_response).collect();
    Ok(Json(items))
}

/// Link a plugin to a repository.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{repo_id}/plugins/{plugin_id}",
    tag = "plugins",
    params(
        ("repo_id" = String, Path, description = "Repository ID"),
        ("plugin_id" = String, Path, description = "Plugin ID"),
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
) -> Result<axum::http::StatusCode, PluginError> {
    // Linking loadable plugin code to a repository requires control of that
    // repository (its sandboxes will clone and execute the plugin's hooks).
    ensure_repo_control(&state, &auth, &repo_id).await?;

    // Verify plugin exists
    state
        .plugins
        .find_by_id(&plugin_id)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Plugin '{plugin_id}' not found")))?;

    state.plugins.link(&repo_id, &plugin_id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Unlink a plugin from a repository.
#[utoipa::path(
    delete,
    path = "/api/v1/repositories/{repo_id}/plugins/{plugin_id}",
    tag = "plugins",
    params(
        ("repo_id" = String, Path, description = "Repository ID"),
        ("plugin_id" = String, Path, description = "Plugin ID"),
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
) -> Result<axum::http::StatusCode, PluginError> {
    ensure_repo_control(&state, &auth, &repo_id).await?;
    let removed = state.plugins.unlink(&repo_id, &plugin_id).await?;
    if !removed {
        return Err(PluginError::NotFound("Link not found".to_string()));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// List a plugin's skills, syncing from its source first.
///
/// This is the admin view's read path, and it goes through the same sync as
/// task provisioning — so opening the page both shows current skills and
/// leaves them stored for the next task that needs them.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/{id}/skills",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    responses(
        (status = 200, description = "The plugin's skills", body = Vec<PluginSkillResponse>),
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_plugin_skills(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<PluginSkillResponse>>, PluginError> {
    let plugin = state
        .plugins
        .find_by_id(&id)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Plugin '{id}' not found")))?;

    let skills = state.plugin_sync.skills(&plugin).await?;
    Ok(Json(
        skills
            .into_iter()
            .map(|s| PluginSkillResponse {
                name: s.name,
                description: s.description,
            })
            .collect(),
    ))
}

fn to_response(r: Plugin) -> PluginResponse {
    PluginResponse {
        id: r.id,
        name: r.name,
        git_url: r.git_url,
        synced_version: r.synced_version,
    }
}
