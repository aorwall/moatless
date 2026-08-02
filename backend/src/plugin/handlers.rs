//! Plugin HTTP handlers.

use axum::extract::{Path, Query, State};
use axum::Json;

use crate::extractors::AuthUser;
use crate::state::AppState;

use super::activation;
use super::error::PluginError;
use super::models::{ActivationRow, Plugin};
use super::schemas::{
    ActivationReach, ActivationResponse, CreatePluginRequest, EffectivePluginResponse,
    PluginResponse, PluginSkillResponse, SetActivationRequest,
};

/// A plugin is loadable code (hooks/commands run inside sandboxes), so
/// registering or removing one — and setting the default everybody gets — is a
/// platform-wide trust decision restricted to admins. Anybody may set their own
/// activation records; see [`activation_user_id`].
fn ensure_admin(auth: &AuthUser) -> Result<(), PluginError> {
    if auth.scope_context().is_admin {
        Ok(())
    } else {
        Err(PluginError::Forbidden(
            "Admin role required to manage the plugin registry".to_string(),
        ))
    }
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
        return Err(PluginError::NotFound(format!("Plugin '{id}' not found")));
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

/// The activation records that bear on this caller for one plugin: every global
/// record plus their own, and nobody else's.
///
/// This is the read half of the two write endpoints below. Without it a control
/// can show what somebody *gets* but not which record put it there, so "turn
/// off my personal override" and "the default is already off" look identical.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/{id}/activations",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    responses(
        (status = 200, description = "The records that apply to this caller", body = Vec<ActivationResponse>),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_plugin_activations(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<ActivationResponse>>, PluginError> {
    let rows = state
        .plugins
        .list_activations_for(Some(auth.user_id))
        .await?;

    Ok(Json(
        rows.into_iter()
            .filter(|r| r.plugin_id == id)
            .map(|r| ActivationResponse {
                reach: if r.user_id.is_some() {
                    ActivationReach::Personal
                } else {
                    ActivationReach::Everyone
                },
                plugin_id: r.plugin_id,
                skill_name: r.skill_name,
                enabled: r.enabled,
            })
            .collect(),
    ))
}

/// Turn a plugin, or one skill inside it, on or off.
///
/// Setting the default for everyone is an administrator's call; setting one's
/// own is anybody's. A personal record wins over the global one in both
/// directions, so this same endpoint is how somebody escapes a plugin that
/// arrived by default.
#[utoipa::path(
    put,
    path = "/api/v1/plugins/{id}/activation",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    request_body = SetActivationRequest,
    responses(
        (status = 200, description = "The stored record", body = ActivationResponse),
        (status = 403, description = "Admin required for the global default", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn set_plugin_activation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<SetActivationRequest>,
) -> Result<Json<ActivationResponse>, PluginError> {
    let user_id = activation_user_id(&auth, req.reach)?;

    state
        .plugins
        .find_by_id(&id)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Plugin '{id}' not found")))?;

    let row = ActivationRow {
        plugin_id: id,
        skill_name: req.skill_name,
        user_id,
        enabled: req.enabled,
    };
    state.plugins.set_activation(&row).await?;

    Ok(Json(ActivationResponse {
        plugin_id: row.plugin_id,
        skill_name: row.skill_name,
        reach: req.reach,
        enabled: row.enabled,
    }))
}

/// Remove an activation record.
///
/// Clearing a personal record is not the same as setting it to off: it hands
/// the person back to whatever the global default says.
#[utoipa::path(
    delete,
    path = "/api/v1/plugins/{id}/activation",
    tag = "plugins",
    params(
        ("id" = String, Path, description = "Plugin ID"),
        ("reach" = ActivationReach, Query, description = "Whose record to clear"),
        ("skillName" = Option<String>, Query, description = "Omit to clear the whole-plugin record"),
    ),
    responses(
        (status = 204, description = "Cleared"),
        (status = 403, description = "Admin required for the global default", body = crate::error::ErrorBody),
        (status = 404, description = "No such record", body = crate::error::ErrorBody),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn clear_plugin_activation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<ClearActivationParams>,
) -> Result<axum::http::StatusCode, PluginError> {
    let user_id = activation_user_id(&auth, params.reach)?;

    let cleared = state
        .plugins
        .clear_activation(&id, params.skill_name, user_id)
        .await?;
    if !cleared {
        return Err(PluginError::NotFound(
            "No activation record to clear".to_string(),
        ));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearActivationParams {
    pub reach: ActivationReach,
    #[serde(default)]
    pub skill_name: Option<String>,
}

/// What the calling user actually gets, once global defaults and their own
/// records have been resolved against each other.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/effective",
    tag = "plugins",
    responses(
        (status = 200, description = "Plugins and skills this user receives", body = Vec<EffectivePluginResponse>),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_effective_plugins(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<EffectivePluginResponse>>, PluginError> {
    let resolved = activation::resolve_for_user(state.plugins.as_ref(), Some(auth.user_id)).await?;

    Ok(Json(
        resolved
            .into_iter()
            .map(|(plugin, skill_names)| EffectivePluginResponse {
                plugin_id: plugin.id,
                plugin_name: plugin.name,
                skill_names,
            })
            .collect(),
    ))
}

/// The `user_id` an activation record carries: `None` for the global default,
/// the caller's own id for a personal one.
///
/// Nobody may write a personal record for somebody else — the reach says
/// "personal", and personal means the caller.
fn activation_user_id(
    auth: &AuthUser,
    reach: ActivationReach,
) -> Result<Option<uuid::Uuid>, PluginError> {
    match reach {
        ActivationReach::Everyone => {
            ensure_admin(auth)?;
            Ok(None)
        }
        ActivationReach::Personal => Ok(Some(auth.user_id)),
    }
}

fn to_response(r: Plugin) -> PluginResponse {
    PluginResponse {
        id: r.id,
        name: r.name,
        git_url: r.git_url,
        synced_version: r.synced_version,
    }
}
