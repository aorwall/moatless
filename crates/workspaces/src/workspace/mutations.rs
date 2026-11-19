use super::dao::WorkspaceDao;
use super::error::WorkspaceError;
use super::models::WorkspaceSource;
use moatless_types::access_control::ScopeContext;

/// Soft-delete a legacy `manual` Workspace the caller controls.
///
/// A git-synced Workspace is refused: the next sync would restore it, so its
/// entry in `.moatless/workspaces.json` is the only thing that removes it.
pub async fn delete_workspace(
    workspaces: &dyn WorkspaceDao,
    ctx: &ScopeContext,
    workspace_id: String,
) -> Result<(), WorkspaceError> {
    let existing = workspaces
        .find_by_id(&workspace_id, ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;
    if existing.source == WorkspaceSource::Git {
        return Err(WorkspaceError::Conflict(
            "Workspace is synced from Git; remove its entry from .moatless/workspaces.json, \
             or delete the repository"
                .to_string(),
        ));
    }
    if !workspaces.soft_delete(&workspace_id, ctx).await? {
        return Err(WorkspaceError::NotFound(workspace_id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::dao::MockWorkspaceDao;
    use super::super::models::{Workspace, WorkspaceKind};
    use super::*;

    fn workspace_with_source(source: WorkspaceSource) -> Workspace {
        Workspace {
            id: "ws_1".to_string(),
            name: "w".to_string(),
            description: None,
            icon: None,
            default_model: None,
            default_effort: None,
            linear_team: None,
            kind: WorkspaceKind::Single,
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            scripts: None,
            resources: None,
            language_servers: None,
            network_profile: None,
            node_selector: None,
            services: None,
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

    fn dao_holding(source: WorkspaceSource) -> MockWorkspaceDao {
        let mut dao = MockWorkspaceDao::new();
        dao.expect_find_by_id()
            .returning(move |_, _| Ok(Some(workspace_with_source(source))));
        dao
    }

    #[tokio::test]
    async fn a_git_workspace_is_not_deleted() {
        let mut dao = dao_holding(WorkspaceSource::Git);
        dao.expect_soft_delete().never();

        let err = delete_workspace(&dao, &ScopeContext::admin("a"), "ws_1".into())
            .await
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::Conflict(_)));
    }

    #[tokio::test]
    async fn a_manual_workspace_is_soft_deleted() {
        let mut dao = dao_holding(WorkspaceSource::Manual);
        dao.expect_soft_delete().times(1).returning(|_, _| Ok(true));

        delete_workspace(&dao, &ScopeContext::admin("a"), "ws_1".into())
            .await
            .unwrap();
    }
}
