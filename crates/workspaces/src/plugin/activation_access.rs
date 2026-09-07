use super::activation;
use super::dao::PluginDao;
use super::error::PluginError;
use super::models::{ActivationRow, ActivationWrite, Plugin};
use super::registry::ensure_admin;
use super::schemas::{
    ActivationReach, ActivationSubjectParams, ClearActivationParams, SetActivationRequest,
};
use crate::workspace::dao::WorkspaceDao;
use moatless_types::access_control::{ScopeContext, ScopedResource};

type Subject = (Option<uuid::Uuid>, Option<String>);

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

fn activation_columns(
    auth: &ScopeContext,
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

async fn write_subject(
    workspaces: &dyn WorkspaceDao,
    auth: &ScopeContext,
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

async fn ensure_workspace_control(
    workspaces: &dyn WorkspaceDao,
    auth: &ScopeContext,
    workspace_id: &str,
) -> Result<(), PluginError> {
    let ctx = auth.clone();
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
    if !ctx.can_write(&resource) {
        return Err(PluginError::Forbidden(format!(
            "No control access to workspace '{workspace_id}'"
        )));
    }
    Ok(())
}

async fn visible_workspace(
    workspaces: &dyn WorkspaceDao,
    auth: &ScopeContext,
    workspace_id: Option<String>,
) -> Result<Option<String>, PluginError> {
    let Some(id) = workspace_id else {
        return Ok(None);
    };
    workspaces
        .find_by_id(&id, &auth.clone())
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("Workspace '{id}' not found")))?;
    Ok(Some(id))
}

fn activation_subject(
    auth: &ScopeContext,
    user_id: Option<uuid::Uuid>,
) -> Result<uuid::Uuid, PluginError> {
    let caller = uuid::Uuid::parse_str(&auth.user_id)
        .map_err(|_| PluginError::Forbidden("Invalid caller identity".to_string()))?;
    match user_id {
        None => Ok(caller),
        Some(id) if id == caller => Ok(id),
        Some(id) => {
            ensure_admin(auth, "act on another user's activation records")?;
            Ok(id)
        }
    }
}

/// Apply activation subject access before reading or writing records.
pub async fn set_plugin_activation(
    plugins: &dyn PluginDao,
    workspaces: &dyn WorkspaceDao,
    auth: &ScopeContext,
    id: String,
    req: SetActivationRequest,
) -> Result<ActivationRow, PluginError> {
    let (user_id, workspace_id) =
        write_subject(workspaces, auth, req.reach, req.user_id, req.workspace_id).await?;

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

    Ok(stored)
}

/// Apply activation subject access before reading or writing records.
pub async fn clear_plugin_activation(
    plugins: &dyn PluginDao,
    workspaces: &dyn WorkspaceDao,
    auth: &ScopeContext,
    id: String,
    params: ClearActivationParams,
) -> Result<(), PluginError> {
    let (user_id, workspace_id) = write_subject(
        workspaces,
        auth,
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
    Ok(())
}

/// Apply activation subject access before reading or writing records.
pub async fn list_plugin_activations(
    plugins: &dyn PluginDao,
    workspaces: &dyn WorkspaceDao,
    auth: &ScopeContext,
    id: String,
    params: ActivationSubjectParams,
) -> Result<Vec<ActivationRow>, PluginError> {
    let subject = activation_subject(auth, params.user_id)?;
    let workspace_id = visible_workspace(workspaces, auth, params.workspace_id).await?;
    let rows = plugins
        .list_activations_for(Some(subject), workspace_id.as_deref())
        .await?;

    Ok(rows.into_iter().filter(|r| r.plugin_id == id).collect())
}

/// Apply activation subject access before reading or writing records.
pub async fn list_effective_plugins(
    plugins: &dyn PluginDao,
    workspaces: &dyn WorkspaceDao,
    auth: &ScopeContext,
    params: ActivationSubjectParams,
) -> Result<Vec<(Plugin, activation::PickedSkills)>, PluginError> {
    let subject = activation_subject(auth, params.user_id)?;
    let workspace_id = visible_workspace(workspaces, auth, params.workspace_id).await?;
    let resolved = activation::resolve_for(plugins, Some(subject), workspace_id.as_deref()).await?;

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    use moatless_types::access_control::Scope;

    use crate::workspace::dao::MockWorkspaceDao;
    use crate::workspace::models::{Workspace, WorkspaceKind, WorkspaceSource};

    const CALLER: uuid::Uuid = uuid::Uuid::from_u128(1);
    const BOT: uuid::Uuid = uuid::Uuid::from_u128(2);

    fn auth(role: &str) -> ScopeContext {
        ScopeContext {
            user_id: CALLER.to_string(),
            is_admin: role == "admin",
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
        let stored =
            write_subject(&mock, &auth("user"), ActivationReach::Personal, None, None).await;
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
