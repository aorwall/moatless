//! Workspace DAO — queries for workspace + workspace_repo persistence.

use async_trait::async_trait;

use super::models::{Workspace, WorkspaceRepo};
use crate::db::DbPool;

/// Repository trait for workspace persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    /// Fetch a single workspace by id.
    async fn find_by_id(&self, id: &str) -> Result<Option<Workspace>, sqlx::Error>;

    /// List a workspace's repo placements, primary first then by `position`.
    async fn find_repos(&self, workspace_id: &str) -> Result<Vec<WorkspaceRepo>, sqlx::Error>;

    /// The workspace's primary repo placement (the diff/PR/CLAUDE.md target),
    /// falling back to the lowest-`position` repo when none is flagged primary.
    async fn find_primary_repo(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRepo>, sqlx::Error>;

    /// Insert a workspace row.
    async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error>;

    /// Insert a workspace_repo placement.
    async fn add_repo(&self, row: &WorkspaceRepo) -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`WorkspaceRepository`].
pub struct PgWorkspaceRepository(pub DbPool);

#[async_trait]
impl WorkspaceRepository for PgWorkspaceRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<Workspace>, sqlx::Error> {
        sqlx::query_as::<_, Workspace>("SELECT * FROM workspaces WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    async fn find_repos(&self, workspace_id: &str) -> Result<Vec<WorkspaceRepo>, sqlx::Error> {
        sqlx::query_as::<_, WorkspaceRepo>(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = $1
               ORDER BY is_primary DESC, position ASC, id ASC"#,
        )
        .bind(workspace_id)
        .fetch_all(&self.0)
        .await
    }

    async fn find_primary_repo(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRepo>, sqlx::Error> {
        sqlx::query_as::<_, WorkspaceRepo>(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = $1
               ORDER BY is_primary DESC, position ASC, id ASC
               LIMIT 1"#,
        )
        .bind(workspace_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO workspaces (
                   id, name, description, kind,
                   docker_image, docker_access, sandbox_service_account,
                   system_prompt, setup_commands, env_vars, servers, plugins, resources,
                   scope, team_id, owner_user_id, created_by, deleted
               ) VALUES (
                   $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18
               )"#,
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.kind)
        .bind(&row.docker_image)
        .bind(row.docker_access)
        .bind(&row.sandbox_service_account)
        .bind(&row.system_prompt)
        .bind(&row.setup_commands)
        .bind(&row.env_vars)
        .bind(&row.servers)
        .bind(&row.plugins)
        .bind(&row.resources)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(&row.owner_user_id)
        .bind(&row.created_by)
        .bind(row.deleted)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn add_repo(&self, row: &WorkspaceRepo) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO workspace_repos (
                   id, workspace_id, repository_id, branch, mount_name, is_primary, position
               ) VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(&row.id)
        .bind(&row.workspace_id)
        .bind(&row.repository_id)
        .bind(&row.branch)
        .bind(&row.mount_name)
        .bind(row.is_primary)
        .bind(row.position)
        .execute(&self.0)
        .await?;
        Ok(())
    }
}
