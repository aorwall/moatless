//! Plugin HTTP handlers.

use axum::Json;
use axum::extract::{Path, Query, State};

use std::sync::Arc;

use moatless_core::http_auth::AuthUser;
use moatless_types::access_control::{Actor, ScopedResource};

use super::activation;
use super::dao::PluginDao;
use super::error::PluginError;
use super::models::{ActivationRow, ActivationWrite, Plugin};
use super::schemas::{
    ActivationReach, ActivationResponse, CreatePluginRequest, EffectivePluginResponse,
    PluginResponse, PluginSkillResponse, SetActivationRequest,
};
use super::sync::PluginSyncService;
use crate::workspace::dao::WorkspaceDao;

/// A plugin is loadable code (hooks/commands run inside sandboxes), so
/// registering or removing one — and setting the default everybody gets — is a
/// platform-wide trust decision restricted to admins. Anybody may set their own
/// activation records; see [`activation_subject`].
fn ensure_admin(auth: &AuthUser, action: &str) -> Result<(), PluginError> {
    if auth.scope_context().is_admin {
        Ok(())
    } else {
        Err(PluginError::Forbidden(format!(
            "Admin role required to {action}"
        )))
    }
}

/// Whose records a write lands on: the `user_id` and `workspace_id` columns an
/// activation row carries, exactly one of which is filled in at most.
type Subject = (Option<uuid::Uuid>, Option<String>);

/// List all plugins.
#[utoipa::path(
    get,
    path = "/api/v1/plugins",
    tag = "plugins",
    responses(
        (status = 200, description = "List of plugins", body = Vec<PluginResponse>),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
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
        (status = 409, description = "Name already exists", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
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
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
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
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_plugin_skills(
    State(plugins): State<Arc<dyn PluginDao>>,
    State(plugin_sync): State<Arc<PluginSyncService>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<PluginSkillResponse>>, PluginError> {
    let plugin = plugins
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
            })
            .collect(),
    ))
}

/// The activation records that bear on one person for one plugin: every global
/// record plus theirs, and nobody else's — plus one Workspace's when `workspaceId`
/// names one. The person is the caller unless `userId` names another, which is
/// an administrator's call.
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
        ("workspaceId" = Option<String>, Query, description = "Also include this Workspace's records"),
    ),
    responses(
        (status = 200, description = "The records that apply to that person", body = Vec<ActivationResponse>),
        (status = 403, description = "Admin required for somebody else's records", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No such workspace", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_plugin_activations(
    State(plugins): State<Arc<dyn PluginDao>>,
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<ActivationSubjectParams>,
) -> Result<Json<Vec<ActivationResponse>>, PluginError> {
    let subject = activation_subject(&auth, params.user_id)?;
    let workspace_id = visible_workspace(workspaces.as_ref(), &auth, params.workspace_id).await?;
    let rows = plugins
        .list_activations_for(Some(subject), workspace_id.as_deref())
        .await?;

    Ok(Json(
        rows.into_iter()
            .filter(|r| r.plugin_id == id)
            .map(to_activation_response)
            .collect(),
    ))
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
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationSubjectParams {
    #[serde(default)]
    pub user_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// Turn a plugin, or one skill inside it, on or off.
///
/// Setting the default for everyone is an administrator's call; setting one's
/// own is anybody's, and setting somebody else's — a bot user, which has no way
/// to sign in and set its own — is an administrator's again. Setting a
/// Workspace's is whoever the Workspace resolves for, the same gate that
/// governs its run configuration.
///
/// Each reach wins over the one below it in both directions, so this same
/// endpoint is how somebody escapes a plugin that arrived by default, and how a
/// Workspace adds one nobody turned on.
#[utoipa::path(
    put,
    path = "/api/v1/plugins/{id}/activation",
    tag = "plugins",
    params(("id" = String, Path, description = "Plugin ID")),
    request_body = SetActivationRequest,
    responses(
        (status = 200, description = "The stored record", body = ActivationResponse),
        (status = 400, description = "A subject was named with a reach it does not belong to", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin required for the global default or somebody else's record; control of the Workspace required for a workspace one", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn set_plugin_activation(
    State(plugins): State<Arc<dyn PluginDao>>,
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<SetActivationRequest>,
) -> Result<Json<ActivationResponse>, PluginError> {
    let (user_id, workspace_id) = write_subject(
        workspaces.as_ref(),
        &auth,
        req.reach,
        req.user_id,
        req.workspace_id,
    )
    .await?;

    plugins
        .find_by_id(&id)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Plugin '{id}' not found")))?;

    let write = ActivationWrite {
        plugin_id: id,
        skill_name: req.skill_name,
        user_id,
        workspace_id,
        enabled: req.enabled,
        always: req.always,
        frontload: req.frontload,
    };
    // What is read back, not what was asked for: a flag the request left unsaid
    // keeps the value it had, and the caller is owed the record that resulted.
    let stored = match plugins.set_activation(&write).await {
        Ok(stored) => stored,
        Err(e) => return Err(unknown_user_or(e, user_id)),
    };

    Ok(Json(to_activation_response(stored)))
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
/// Clearing a record is not the same as setting it to off: it hands the subject
/// back to whatever the next reach down says.
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
        (status = 400, description = "A subject was named with a reach it does not belong to", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin required for the global default or somebody else's record; control of the Workspace required for a workspace one", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No such record", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn clear_plugin_activation(
    State(plugins): State<Arc<dyn PluginDao>>,
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<ClearActivationParams>,
) -> Result<axum::http::StatusCode, PluginError> {
    let (user_id, workspace_id) = write_subject(
        workspaces.as_ref(),
        &auth,
        params.reach,
        params.user_id,
        params.workspace_id,
    )
    .await?;

    let cleared = plugins
        .clear_activation(&id, params.skill_name, user_id, workspace_id.as_deref())
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
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// What one person actually gets, once the global defaults, their own records
/// and — when `workspaceId` names one — that Workspace's have been resolved
/// against each other. The caller unless `userId` names another, which is an
/// administrator's call.
///
/// The same read whoever it is asked about: an administrator setting a bot
/// user's records needs to see what those records add up to for the bot, not
/// for themselves, and resolution is the server's to do either way.
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
        (status = 403, description = "Admin required for somebody else's delivery", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No such workspace", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_effective_plugins(
    State(plugins): State<Arc<dyn PluginDao>>,
    State(workspaces): State<Arc<dyn WorkspaceDao>>,
    auth: AuthUser,
    Query(params): Query<ActivationSubjectParams>,
) -> Result<Json<Vec<EffectivePluginResponse>>, PluginError> {
    let subject = activation_subject(&auth, params.user_id)?;
    let workspace_id = visible_workspace(workspaces.as_ref(), &auth, params.workspace_id).await?;
    let resolved =
        activation::resolve_for(plugins.as_ref(), Some(subject), workspace_id.as_deref()).await?;

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

/// The subject columns an activation record carries — `(None, None)` for the
/// global default, somebody's id for a personal one, a Workspace's for a
/// Workspace one — and the permission checks that need nothing read.
///
/// A subject named alongside a reach it does not belong to is refused rather
/// than ignored: every one of those reads as a narrow record and would write a
/// broader one, and the broadest changes what everybody gets.
fn activation_columns(
    auth: &AuthUser,
    reach: ActivationReach,
    user_id: Option<uuid::Uuid>,
    workspace_id: Option<String>,
) -> Result<Subject, PluginError> {
    match reach {
        ActivationReach::Everyone => {
            if user_id.is_some() || workspace_id.is_some() {
                return Err(PluginError::BadRequest(
                    "userId and workspaceId cannot be combined with the everyone reach".to_string(),
                ));
            }
            ensure_admin(auth, "set the default for everyone")?;
            Ok((None, None))
        }
        ActivationReach::Personal => {
            if workspace_id.is_some() {
                return Err(PluginError::BadRequest(
                    "workspaceId cannot be combined with the personal reach".to_string(),
                ));
            }
            Ok((Some(activation_subject(auth, user_id)?), None))
        }
        ActivationReach::Workspace => {
            if user_id.is_some() {
                return Err(PluginError::BadRequest(
                    "userId cannot be combined with the workspace reach".to_string(),
                ));
            }
            let id = workspace_id.ok_or_else(|| {
                PluginError::BadRequest(
                    "workspaceId is required with the workspace reach".to_string(),
                )
            })?;
            Ok((None, Some(id)))
        }
    }
}

/// The subject columns for a write, with the Workspace half checked against the
/// store.
///
/// A Workspace record decides what loads for everybody who works in that
/// Workspace, so the gate is the one every other change to that Workspace
/// passes: control, not read. Reading is the wider grant — a global Workspace
/// is readable by everyone signed in — and a reader who could write these
/// records would be setting execution policy for other people's tasks.
///
/// Being git-declared does not close that gate — `.moatless/workspaces.json`
/// describes no activation records, so there is nothing here for a git sync to
/// reconcile away.
async fn write_subject(
    workspaces: &dyn WorkspaceDao,
    auth: &AuthUser,
    reach: ActivationReach,
    user_id: Option<uuid::Uuid>,
    workspace_id: Option<String>,
) -> Result<Subject, PluginError> {
    let subject = activation_columns(auth, reach, user_id, workspace_id)?;
    if let Some(id) = &subject.1 {
        ensure_workspace_control(workspaces, auth, id).await?;
    }
    Ok(subject)
}

/// A Workspace named by a write, resolved through the caller's own scope and
/// then through control.
///
/// The two answers are deliberately different: one the caller cannot see is
/// absent, and one they can see but not control is refused. Control is the same
/// admin-or-owner rule `WorkspaceDao`'s own mutations embed as
/// [`ScopeContext::control_filter`]; it is applied here rather than in the
/// statement because the row being written belongs to another table.
async fn ensure_workspace_control(
    workspaces: &dyn WorkspaceDao,
    auth: &AuthUser,
    workspace_id: &str,
) -> Result<(), PluginError> {
    let ctx = auth.scope_context();
    let workspace = workspaces
        .find_by_id(workspace_id, &ctx)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Workspace '{workspace_id}' not found")))?;
    let resource = ScopedResource::borrowed(
        workspace.scope,
        workspace
            .owner_user_id
            .as_deref()
            .or(workspace.created_by.as_deref()),
    );
    if !ctx.can_control(&resource) {
        return Err(PluginError::Forbidden(format!(
            "No control access to workspace '{workspace_id}'"
        )));
    }
    Ok(())
}

/// A Workspace named by a read, resolved through the caller's own scope so one
/// they cannot see reads as absent rather than as somebody else's settings.
async fn visible_workspace(
    workspaces: &dyn WorkspaceDao,
    auth: &AuthUser,
    workspace_id: Option<String>,
) -> Result<Option<String>, PluginError> {
    let Some(id) = workspace_id else {
        return Ok(None);
    };
    workspaces
        .find_by_id(&id, &auth.scope_context())
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Workspace '{id}' not found")))?;
    Ok(Some(id))
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

    use moatless_types::access_control::Scope;

    use crate::workspace::dao::MockWorkspaceDao;
    use crate::workspace::models::{Workspace, WorkspaceKind, WorkspaceSource};

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

    fn forbidden(result: &Result<Subject, PluginError>) -> bool {
        matches!(result, Err(PluginError::Forbidden(_)))
    }

    fn bad_request(result: &Result<Subject, PluginError>) -> bool {
        matches!(result, Err(PluginError::BadRequest(_)))
    }

    /// The subject columns for a reach that names no workspace.
    fn columns(
        role: &str,
        reach: ActivationReach,
        user_id: Option<uuid::Uuid>,
    ) -> Result<Subject, PluginError> {
        activation_columns(&auth(role), reach, user_id, None)
    }

    #[test]
    fn a_personal_record_without_a_user_is_the_callers_own() {
        assert_eq!(
            columns("user", ActivationReach::Personal, None).ok(),
            Some((Some(CALLER), None))
        );
    }

    #[test]
    fn naming_yourself_is_the_same_as_naming_nobody() {
        assert_eq!(
            columns("user", ActivationReach::Personal, Some(CALLER)).ok(),
            Some((Some(CALLER), None)),
            "a client that fills the field in from the session must not need the admin role"
        );
    }

    #[test]
    fn an_admin_may_write_a_record_for_a_bot_user() {
        assert_eq!(
            columns("admin", ActivationReach::Personal, Some(BOT)).ok(),
            Some((Some(BOT), None)),
            "a bot never signs in, so its own records can only be written on its behalf"
        );
    }

    #[test]
    fn a_non_admin_may_not_write_a_record_for_somebody_else() {
        assert!(forbidden(&columns(
            "user",
            ActivationReach::Personal,
            Some(BOT)
        )));
    }

    #[test]
    fn a_non_admin_may_not_set_the_default_for_everyone() {
        assert!(forbidden(&columns("user", ActivationReach::Everyone, None)));
    }

    #[test]
    fn naming_a_user_with_the_everyone_reach_is_refused_rather_than_ignored() {
        assert!(
            bad_request(&columns("admin", ActivationReach::Everyone, Some(BOT))),
            "silently writing the global default would change what everybody gets"
        );
    }

    // ---- Workspace reach ----

    #[test]
    fn a_workspace_record_names_the_workspace_and_nobody() {
        let stored = activation_columns(
            &auth("user"),
            ActivationReach::Workspace,
            None,
            Some("ws1".to_string()),
        );
        assert_eq!(stored.ok(), Some((None, Some("ws1".to_string()))));
    }

    #[test]
    fn setting_a_workspace_record_does_not_need_the_admin_role() {
        let stored = activation_columns(
            &auth("user"),
            ActivationReach::Workspace,
            None,
            Some("ws1".to_string()),
        );
        assert!(
            stored.is_ok(),
            "the gate is control of that one workspace, checked against the \
             store by write_subject, not a platform-wide role"
        );
    }

    #[test]
    fn a_workspace_record_without_a_workspace_is_refused() {
        assert!(
            bad_request(&activation_columns(
                &auth("admin"),
                ActivationReach::Workspace,
                None,
                None
            )),
            "storing it with no workspace would write the deployment-wide default"
        );
    }

    #[test]
    fn a_subject_named_with_the_wrong_reach_is_refused() {
        // A user with the workspace reach: reads as "for that person here",
        // would store a record for the whole workspace.
        assert!(bad_request(&activation_columns(
            &auth("admin"),
            ActivationReach::Workspace,
            Some(BOT),
            Some("ws1".to_string())
        )));
        // A workspace with the personal reach: reads as "for me there", would
        // store one that follows the person everywhere.
        assert!(bad_request(&activation_columns(
            &auth("user"),
            ActivationReach::Personal,
            None,
            Some("ws1".to_string())
        )));
        // A workspace with the everyone reach: would store the global default.
        assert!(bad_request(&activation_columns(
            &auth("admin"),
            ActivationReach::Everyone,
            None,
            Some("ws1".to_string())
        )));
    }

    // ---- Control over the workspace a record names ----

    /// A `WorkspaceDao` serving one row, to whoever the read rule admits.
    fn workspaces_holding(ws: Workspace) -> MockWorkspaceDao {
        let mut mock = MockWorkspaceDao::new();
        mock.expect_find_by_id().returning(move |id, ctx| {
            if id != ws.id {
                return Ok(None);
            }
            let resource = ScopedResource::borrowed(
                ws.scope,
                ws.owner_user_id.as_deref().or(ws.created_by.as_deref()),
            );
            Ok(ctx.can_read(&resource).then(|| ws.clone()))
        });
        mock
    }

    fn workspace(scope: Scope, owner: Option<uuid::Uuid>) -> Workspace {
        Workspace {
            id: "ws1".to_string(),
            name: "ws".to_string(),
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
            source: WorkspaceSource::Manual,
            source_repository_id: None,
            source_key: None,
            source_config_path: None,
            scope,
            run_as_user_id: None,
            owner_user_id: owner.map(|id| id.to_string()),
            created_by: owner.map(|id| id.to_string()),
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    /// The write path for a record naming `ws1`, against a store holding `ws`.
    async fn write_for(role: &str, ws: Workspace) -> Result<Subject, PluginError> {
        write_subject(
            &workspaces_holding(ws),
            &auth(role),
            ActivationReach::Workspace,
            None,
            Some("ws1".to_string()),
        )
        .await
    }

    #[tokio::test]
    async fn the_owner_of_a_workspace_may_write_its_records() {
        let stored = write_for("user", workspace(Scope::User, Some(CALLER))).await;
        assert_eq!(stored.ok(), Some((None, Some("ws1".to_string()))));
    }

    #[tokio::test]
    async fn merely_being_able_to_read_a_workspace_does_not_admit_a_write() {
        let stored = write_for("user", workspace(Scope::Global, None)).await;
        assert!(
            forbidden(&stored),
            "a global workspace reads to everybody signed in, and these records \
             decide what loads for everybody working in it"
        );
    }

    #[tokio::test]
    async fn one_persons_workspace_is_not_anothers_to_configure() {
        let stored = write_for("user", workspace(Scope::User, Some(BOT))).await;
        assert!(
            matches!(stored, Err(PluginError::NotFound(_))),
            "the read refuses it first, so it reads as absent rather than as \
             somebody else's workspace"
        );
    }

    #[tokio::test]
    async fn an_admin_may_write_a_global_workspaces_records() {
        let stored = write_for("admin", workspace(Scope::Global, None)).await;
        assert_eq!(stored.ok(), Some((None, Some("ws1".to_string()))));
    }

    #[tokio::test]
    async fn a_workspace_that_is_not_there_is_not_written_to() {
        let stored = write_subject(
            &workspaces_holding(workspace(Scope::User, Some(CALLER))),
            &auth("admin"),
            ActivationReach::Workspace,
            None,
            Some("ws-gone".to_string()),
        )
        .await;
        assert!(matches!(stored, Err(PluginError::NotFound(_))));
    }

    #[tokio::test]
    async fn the_reaches_that_name_no_workspace_never_reach_the_store() {
        let mut mock = MockWorkspaceDao::new();
        mock.expect_find_by_id().never();
        let stored = write_subject(&mock, &auth("user"), ActivationReach::Personal, None, None).await;
        assert_eq!(stored.ok(), Some((Some(CALLER), None)));
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
