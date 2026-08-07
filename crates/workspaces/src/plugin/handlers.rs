//! Plugin HTTP handlers.

use axum::Json;
use axum::extract::{Path, Query, State};

use std::sync::Arc;

use moatless_identity::extractors::AuthUser;

use super::activation;
use super::dao::PluginDao;
use super::error::PluginError;
use super::models::{ActivationRow, Plugin};
use super::schemas::{
    ActivationReach, ActivationResponse, CreatePluginRequest, EffectivePluginResponse,
    PluginResponse, PluginSkillResponse, SetActivationRequest,
};
use super::sync::PluginSyncService;

/// A plugin is loadable code (hooks/commands run inside sandboxes), so
/// registering or removing one — and setting the default everybody gets — is a
/// platform-wide trust decision restricted to admins. Anybody may set their own
/// activation records; see [`activation_user_id`].
fn ensure_admin(auth: &AuthUser, action: &str) -> Result<(), PluginError> {
    if auth.scope_context().is_admin {
        Ok(())
    } else {
        Err(PluginError::Forbidden(format!(
            "Admin role required to {action}"
        )))
    }
}

/// List all plugins.
#[utoipa::path(
    get,
    path = "/api/v1/plugins",
    tag = "plugins",
    responses(
        (status = 200, description = "List of plugins", body = Vec<PluginResponse>),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn list_plugins(
    State(plugins): State<Arc<dyn PluginDao>>,
    _auth: AuthUser,
) -> Result<Json<Vec<PluginResponse>>, PluginError> {
    let rows = plugins.list_all().await?;
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
        (status = 409, description = "Name already exists", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn create_plugin(
    State(plugins): State<Arc<dyn PluginDao>>,
    auth: AuthUser,
    Json(req): Json<CreatePluginRequest>,
) -> Result<Json<PluginResponse>, PluginError> {
    ensure_admin(&auth, "manage the plugin registry")?;
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

    if let Err(e) = plugins.insert(&row).await {
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
        (status = 404, description = "Not found", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn delete_plugin(
    State(plugins): State<Arc<dyn PluginDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, PluginError> {
    ensure_admin(&auth, "manage the plugin registry")?;
    let deleted = plugins.delete(&id).await?;
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
        (status = 404, description = "Not found", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn list_plugin_skills(
    State(plugins): State<Arc<dyn PluginDao>>,
    State(plugin_sync): State<Arc<PluginSyncService>>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<PluginSkillResponse>>, PluginError> {
    let plugin = plugins
        .find_by_id(&id)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Plugin '{id}' not found")))?;

    let skills = plugin_sync.skills(&plugin).await?;
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

/// The activation records that bear on one person for one plugin: every global
/// record plus theirs, and nobody else's. The person is the caller unless
/// `userId` names another, which is an administrator's call.
///
/// This is the read half of the two write endpoints below. Without it a control
/// can show what somebody *gets* but not which record put it there, so "turn
/// off my personal override" and "the default is already off" look identical.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/{id}/activations",
    tag = "plugins",
    params(
        ("id" = String, Path, description = "Plugin ID"),
        ("userId" = Option<uuid::Uuid>, Query, description = "Whose records; omit for your own"),
    ),
    responses(
        (status = 200, description = "The records that apply to that person", body = Vec<ActivationResponse>),
        (status = 403, description = "Admin required for somebody else's records", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn list_plugin_activations(
    State(plugins): State<Arc<dyn PluginDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<ActivationSubjectParams>,
) -> Result<Json<Vec<ActivationResponse>>, PluginError> {
    let subject = activation_subject(&auth, params.user_id)?;
    let rows = plugins.list_activations_for(Some(subject)).await?;

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
                user_id: r.user_id,
                enabled: r.enabled,
            })
            .collect(),
    ))
}

/// Whose records a request is about, for the reads and writes that can be made
/// on somebody else's behalf.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationSubjectParams {
    #[serde(default)]
    pub user_id: Option<uuid::Uuid>,
}

/// Turn a plugin, or one skill inside it, on or off.
///
/// Setting the default for everyone is an administrator's call; setting one's
/// own is anybody's, and setting somebody else's — a bot user, which has no way
/// to sign in and set its own — is an administrator's again. A personal record
/// wins over the global one in both directions, so this same endpoint is how
/// somebody escapes a plugin that arrived by default.
#[utoipa::path(
    put,
    path = "/api/v1/plugins/{id}/activation",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    request_body = SetActivationRequest,
    responses(
        (status = 200, description = "The stored record", body = ActivationResponse),
        (status = 400, description = "A user was named with the everyone reach", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin required for the global default or somebody else's record", body = moatless_core::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn set_plugin_activation(
    State(plugins): State<Arc<dyn PluginDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<SetActivationRequest>,
) -> Result<Json<ActivationResponse>, PluginError> {
    let user_id = activation_user_id(&auth, req.reach, req.user_id)?;

    plugins
        .find_by_id(&id)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Plugin '{id}' not found")))?;

    let row = ActivationRow {
        plugin_id: id,
        skill_name: req.skill_name,
        user_id,
        enabled: req.enabled,
    };
    if let Err(e) = plugins.set_activation(&row).await {
        return Err(unknown_user_or(e, user_id));
    }

    Ok(Json(ActivationResponse {
        plugin_id: row.plugin_id,
        skill_name: row.skill_name,
        reach: req.reach,
        user_id: row.user_id,
        enabled: row.enabled,
    }))
}

/// A write naming a user who is not there, told apart from any other database
/// failure by the foreign key it violated — Postgres's default name for the
/// unnamed `user_id` reference in migration `0102_plugin_activations`.
///
/// Nothing else reaches the users table on this path, so the alternative is a
/// 500 on a request whose only fault is a stale id — which is what an
/// administrator holds after the bot user they were configuring was deleted.
fn unknown_user_or(e: sqlx::Error, user_id: Option<uuid::Uuid>) -> PluginError {
    let violated_user_fk = matches!(
        &e,
        sqlx::Error::Database(db) if db.constraint() == Some("plugin_activations_user_id_fkey")
    );
    match (violated_user_fk, user_id) {
        (true, Some(id)) => PluginError::NotFound(format!("User '{id}' not found")),
        _ => e.into(),
    }
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
        ("userId" = Option<uuid::Uuid>, Query, description = "Whose personal record; omit for your own"),
    ),
    responses(
        (status = 204, description = "Cleared"),
        (status = 400, description = "A user was named with the everyone reach", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin required for the global default or somebody else's record", body = moatless_core::error::ErrorBody),
        (status = 404, description = "No such record", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn clear_plugin_activation(
    State(plugins): State<Arc<dyn PluginDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<ClearActivationParams>,
) -> Result<axum::http::StatusCode, PluginError> {
    let user_id = activation_user_id(&auth, params.reach, params.user_id)?;

    let cleared = plugins
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
    #[serde(default)]
    pub user_id: Option<uuid::Uuid>,
}

/// What one person actually gets, once global defaults and their own records
/// have been resolved against each other. The caller unless `userId` names
/// another, which is an administrator's call.
///
/// The same read whoever it is asked about: an administrator setting a bot
/// user's records needs to see what those records add up to for the bot, not
/// for themselves, and resolution is the server's to do either way.
#[utoipa::path(
    get,
    path = "/api/v1/plugins/effective",
    tag = "plugins",
    params(("userId" = Option<uuid::Uuid>, Query, description = "Whose delivery; omit for your own")),
    responses(
        (status = 200, description = "Plugins and skills that user receives", body = Vec<EffectivePluginResponse>),
        (status = 403, description = "Admin required for somebody else's delivery", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn list_effective_plugins(
    State(plugins): State<Arc<dyn PluginDao>>,
    auth: AuthUser,
    Query(params): Query<ActivationSubjectParams>,
) -> Result<Json<Vec<EffectivePluginResponse>>, PluginError> {
    let subject = activation_subject(&auth, params.user_id)?;
    let resolved = activation::resolve_for_user(plugins.as_ref(), Some(subject)).await?;

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
/// somebody's id for a personal one.
///
/// Naming a user alongside the `everyone` reach is refused rather than ignored:
/// it reads as a record for that person and would write the deployment-wide
/// default, which is the one mistake here that changes what everybody gets.
fn activation_user_id(
    auth: &AuthUser,
    reach: ActivationReach,
    user_id: Option<uuid::Uuid>,
) -> Result<Option<uuid::Uuid>, PluginError> {
    match reach {
        ActivationReach::Everyone => {
            if user_id.is_some() {
                return Err(PluginError::BadRequest(
                    "userId cannot be combined with the everyone reach".to_string(),
                ));
            }
            ensure_admin(auth, "set the default for everyone")?;
            Ok(None)
        }
        ActivationReach::Personal => Ok(Some(activation_subject(auth, user_id)?)),
    }
}

/// Whose records a request is about: the caller, unless it names somebody else.
///
/// Naming somebody else is an administrator's call. That is what makes a bot
/// user configurable at all — it never signs in, so its own records can only be
/// written on its behalf — and it is equally what keeps one person out of
/// another's.
fn activation_subject(
    auth: &AuthUser,
    user_id: Option<uuid::Uuid>,
) -> Result<uuid::Uuid, PluginError> {
    match user_id {
        None => Ok(auth.user_id),
        Some(id) if id == auth.user_id => Ok(id),
        Some(id) => {
            ensure_admin(auth, "act on another user's activation records")?;
            Ok(id)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    const CALLER: uuid::Uuid = uuid::Uuid::from_u128(1);
    const BOT: uuid::Uuid = uuid::Uuid::from_u128(2);

    fn auth(role: &str) -> AuthUser {
        AuthUser {
            user_id: CALLER,
            login: "caller".to_string(),
            role: Some(role.to_string()),
            provider: None,
            group_id: None,
            task_id: None,
            is_agent_context: false,
        }
    }

    fn forbidden(result: &Result<Option<uuid::Uuid>, PluginError>) -> bool {
        matches!(result, Err(PluginError::Forbidden(_)))
    }

    #[test]
    fn a_personal_record_without_a_user_is_the_callers_own() {
        let stored = activation_user_id(&auth("user"), ActivationReach::Personal, None);
        assert_eq!(stored.ok().flatten(), Some(CALLER));
    }

    #[test]
    fn naming_yourself_is_the_same_as_naming_nobody() {
        let stored = activation_user_id(&auth("user"), ActivationReach::Personal, Some(CALLER));
        assert_eq!(
            stored.ok().flatten(),
            Some(CALLER),
            "a client that fills the field in from the session must not need the admin role"
        );
    }

    #[test]
    fn an_admin_may_write_a_record_for_a_bot_user() {
        let stored = activation_user_id(&auth("admin"), ActivationReach::Personal, Some(BOT));
        assert_eq!(
            stored.ok().flatten(),
            Some(BOT),
            "a bot never signs in, so its own records can only be written on its behalf"
        );
    }

    #[test]
    fn a_non_admin_may_not_write_a_record_for_somebody_else() {
        let stored = activation_user_id(&auth("user"), ActivationReach::Personal, Some(BOT));
        assert!(forbidden(&stored));
    }

    #[test]
    fn a_non_admin_may_not_set_the_default_for_everyone() {
        let stored = activation_user_id(&auth("user"), ActivationReach::Everyone, None);
        assert!(forbidden(&stored));
    }

    #[test]
    fn naming_a_user_with_the_everyone_reach_is_refused_rather_than_ignored() {
        let stored = activation_user_id(&auth("admin"), ActivationReach::Everyone, Some(BOT));
        assert!(
            matches!(stored, Err(PluginError::BadRequest(_))),
            "silently writing the global default would change what everybody gets"
        );
    }

    #[test]
    fn reading_somebody_elses_records_needs_the_admin_role() {
        assert!(activation_subject(&auth("user"), Some(BOT)).is_err());
        assert_eq!(
            activation_subject(&auth("admin"), Some(BOT)).ok(),
            Some(BOT)
        );
        assert_eq!(activation_subject(&auth("user"), None).ok(), Some(CALLER));
    }
}
