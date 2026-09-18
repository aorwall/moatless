//! Plugin HTTP handlers.

use axum::Json;
use axum::extract::{Path, Query, State};

use std::sync::Arc;

use moatless_core::http_auth::AuthUser;
use moatless_types::access_control::Actor;

use super::activation;
use super::error::PluginError;
use super::models::{ActivationRow, Plugin};
use super::operations::Plugins;
use super::schemas::{
    ActivationReach, ActivationResponse, CreatePluginRequest, EffectivePluginResponse,
    PluginResponse, PluginSkillResponse, SetActivationRequest,
};
use super::sync::PluginSyncService;
use crate::workspace::Workspaces;

/// List all plugins.
#[utoipa::path(
    get,
    path = "/api/v1/plugins",
    tag = "plugins",
    responses(
        (status = 200, description = "List of plugins", body = Vec<PluginResponse>),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn list_plugins(
    State(plugins): State<Arc<Plugins>>,
    _auth: AuthUser,
) -> Result<Json<Vec<PluginResponse>>, PluginError> {
    let rows = plugins.store().list_all().await?;
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
        (status = 409, description = "Name already exists", body = moatless_types::http::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn create_plugin(
    State(plugins): State<Arc<Plugins>>,
    auth: AuthUser,
    Json(req): Json<CreatePluginRequest>,
) -> Result<Json<PluginResponse>, PluginError> {
    let row = super::registry::create_plugin(plugins.store(), &auth.scope_context(), req).await?;
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
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn delete_plugin(
    State(plugins): State<Arc<Plugins>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, PluginError> {
    super::registry::delete_plugin(plugins.store(), &auth.scope_context(), id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// List a plugin's skills, syncing from its source first.
///
/// Goes through the same sync as task provisioning, so opening this admin view
/// both shows current skills and leaves them stored for the next task.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/{id}/skills",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    responses(
        (status = 200, description = "The plugin's skills", body = Vec<PluginSkillResponse>),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn list_plugin_skills(
    State(plugins): State<Arc<Plugins>>,
    State(plugin_sync): State<Arc<PluginSyncService>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<PluginSkillResponse>>, PluginError> {
    let plugin = plugins
        .store()
        .find_by_id(&id)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Plugin '{id}' not found")))?;

    let skills = plugin_sync
        .skills(&Actor::caller(&auth.scope_context()), &plugin)
        .await?;
    Ok(Json(
        skills
            .into_iter()
            .map(|s| PluginSkillResponse {
                name: s.name,
                description: s.description,
                path: s.path,
            })
            .collect(),
    ))
}

/// The activation records that bear on one person for one plugin — global,
/// theirs, and one Workspace's when `workspaceId` names one.
///
/// The read half of the write endpoints below: without it, "turn off my
/// override" and "the default is already off" look identical.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/{id}/activations",
    tag = "plugins",
    params(
        ("id" = String, Path, description = "Plugin ID"),
        ("userId" = Option<uuid::Uuid>, Query, description = "Whose records; omit for your own"),
        ("workspaceId" = Option<String>, Query, description = "Also include this Workspace's records"),
    ),
    responses(
        (status = 200, description = "The records that apply to that person", body = Vec<ActivationResponse>),
        (status = 403, description = "Admin required for somebody else's records", body = moatless_types::http::ErrorBody),
        (status = 404, description = "No such workspace", body = moatless_types::http::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn list_plugin_activations(
    State(plugins): State<Arc<Plugins>>,
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<ActivationSubjectParams>,
) -> Result<Json<Vec<ActivationResponse>>, PluginError> {
    let rows = super::activation_access::list_plugin_activations(
        plugins.store(),
        workspaces.store(),
        &auth.scope_context(),
        id,
        params,
    )
    .await?;
    Ok(Json(rows.into_iter().map(to_activation_response).collect()))
}

/// A stored record on the wire, reach read back off the subject it names.
fn to_activation_response(r: ActivationRow) -> ActivationResponse {
    ActivationResponse {
        reach: wire_reach(activation::reach_of(&r)),
        plugin_id: r.plugin_id,
        skill_name: r.skill_name,
        user_id: r.user_id,
        workspace_id: r.workspace_id,
        enabled: r.enabled,
        always: r.always,
        frontload: r.frontload,
    }
}

/// The domain's reach on the wire. Kept as a total match so a reach added to
/// one of the two enums cannot be quietly mapped onto the wrong neighbour.
fn wire_reach(reach: activation::Reach) -> ActivationReach {
    match reach {
        activation::Reach::Everyone => ActivationReach::Everyone,
        activation::Reach::Personal => ActivationReach::Personal,
        activation::Reach::Workspace => ActivationReach::Workspace,
    }
}

/// Whose records a request is about, for the reads and writes that can be made
/// on somebody else's behalf, and in which Workspace.
pub use super::schemas::ActivationSubjectParams;

/// Turn a plugin, or one skill inside it, on or off.
///
/// The global default and somebody else's record need an administrator; one's
/// own is anybody's; a Workspace's needs whoever the Workspace resolves for.
/// Each reach wins over the one below it either direction, so this is how
/// somebody escapes a default and how a Workspace adds one nobody turned on.
#[utoipa::path(
    put,
    path = "/api/v1/plugins/{id}/activation",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    request_body = SetActivationRequest,
    responses(
        (status = 200, description = "The stored record", body = ActivationResponse),
        (status = 400, description = "A subject was named with a reach it does not belong to", body = moatless_types::http::ErrorBody),
        (status = 403, description = "Admin required for the global default or somebody else's record; control of the Workspace required for a workspace one", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn set_plugin_activation(
    State(plugins): State<Arc<Plugins>>,
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<SetActivationRequest>,
) -> Result<Json<ActivationResponse>, PluginError> {
    let stored = super::activation_access::set_plugin_activation(
        plugins.store(),
        workspaces.store(),
        &auth.scope_context(),
        id,
        req,
    )
    .await?;
    Ok(Json(to_activation_response(stored)))
}

/// Remove an activation record.
///
/// Not the same as setting it to off: clearing hands the subject back to
/// whatever the next reach down says.
#[utoipa::path(
    delete,
    path = "/api/v1/plugins/{id}/activation",
    tag = "plugins",
    params(
        ("id" = String, Path, description = "Plugin ID"),
        ("reach" = ActivationReach, Query, description = "Whose record to clear"),
        ("skillName" = Option<String>, Query, description = "Omit to clear the whole-plugin record"),
        ("userId" = Option<uuid::Uuid>, Query, description = "Whose personal record; omit for your own"),
        ("workspaceId" = Option<String>, Query, description = "Which Workspace's record; required with the workspace reach"),
    ),
    responses(
        (status = 204, description = "Cleared"),
        (status = 400, description = "A subject was named with a reach it does not belong to", body = moatless_types::http::ErrorBody),
        (status = 403, description = "Admin required for the global default or somebody else's record; control of the Workspace required for a workspace one", body = moatless_types::http::ErrorBody),
        (status = 404, description = "No such record", body = moatless_types::http::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn clear_plugin_activation(
    State(plugins): State<Arc<Plugins>>,
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<ClearActivationParams>,
) -> Result<axum::http::StatusCode, PluginError> {
    super::activation_access::clear_plugin_activation(
        plugins.store(),
        workspaces.store(),
        &auth.scope_context(),
        id,
        params,
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub use super::schemas::ClearActivationParams;

/// What one person actually gets, once global defaults, their own records, and
/// — when `workspaceId` names one — that Workspace's are resolved against each
/// other.
///
/// The same read whoever it's asked about, so an administrator sees what a bot
/// user's records add up to for the bot, not for themselves.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/effective",
    tag = "plugins",
    params(
        ("userId" = Option<uuid::Uuid>, Query, description = "Whose delivery; omit for your own"),
        ("workspaceId" = Option<String>, Query, description = "Which Workspace they are working in; omit for none in particular"),
    ),
    responses(
        (status = 200, description = "Plugins and skills that user receives", body = Vec<EffectivePluginResponse>),
        (status = 403, description = "Admin required for somebody else's delivery", body = moatless_types::http::ErrorBody),
        (status = 404, description = "No such workspace", body = moatless_types::http::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn list_effective_plugins(
    State(plugins): State<Arc<Plugins>>,
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Query(params): Query<ActivationSubjectParams>,
) -> Result<Json<Vec<EffectivePluginResponse>>, PluginError> {
    let resolved = super::activation_access::list_effective_plugins(
        &plugins,
        workspaces.store(),
        &auth.scope_context(),
        params,
    )
    .await?;
    Ok(Json(
        resolved
            .into_iter()
            .map(|(plugin, skills)| EffectivePluginResponse {
                plugin_id: plugin.id,
                plugin_name: plugin.name,
                skill_names: skills.names,
                frontload_skill_names: skills.frontloaded,
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
