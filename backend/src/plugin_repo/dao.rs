//! Plugin repo DAO — queries for plugin repo data access.

use async_trait::async_trait;

use super::models::PluginRepo;
use crate::db::DbPool;

/// Repository trait for plugin repo persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PluginRepoRepository: Send + Sync {
    async fn list_all(&self) -> Result<Vec<PluginRepo>, sqlx::Error>;
    async fn find_by_id(&self, id: &str) -> Result<Option<PluginRepo>, sqlx::Error>;
    async fn insert(&self, row: &PluginRepo) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error>;
    async fn find_by_repository_id(&self, repo_id: &str) -> Result<Vec<PluginRepo>, sqlx::Error>;
    async fn link(&self, repository_id: &str, plugin_repo_id: &str) -> Result<(), sqlx::Error>;
    async fn unlink(&self, repository_id: &str, plugin_repo_id: &str) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`PluginRepoRepository`].
pub struct PgPluginRepoRepository(pub DbPool);

#[async_trait]
impl PluginRepoRepository for PgPluginRepoRepository {
    async fn list_all(&self) -> Result<Vec<PluginRepo>, sqlx::Error> {
        sqlx::query_as::<_, PluginRepo>(
            "SELECT id, name, git_url, created_at, updated_at FROM plugin_repos ORDER BY name ASC",
        )
        .fetch_all(&self.0)
        .await
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<PluginRepo>, sqlx::Error> {
        sqlx::query_as::<_, PluginRepo>(
            "SELECT id, name, git_url, created_at, updated_at FROM plugin_repos WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert(&self, row: &PluginRepo) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO plugin_repos (id, name, git_url) VALUES ($1, $2, $3)",
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.git_url)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM plugin_repos WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn find_by_repository_id(&self, repo_id: &str) -> Result<Vec<PluginRepo>, sqlx::Error> {
        sqlx::query_as::<_, PluginRepo>(
            r#"SELECT pr.id, pr.name, pr.git_url, pr.created_at, pr.updated_at
               FROM plugin_repos pr
               JOIN repository_plugin_repos rpr ON pr.id = rpr.plugin_repo_id
               WHERE rpr.repository_id = $1
               ORDER BY pr.name ASC"#,
        )
        .bind(repo_id)
        .fetch_all(&self.0)
        .await
    }

    async fn link(&self, repository_id: &str, plugin_repo_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO repository_plugin_repos (repository_id, plugin_repo_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(repository_id)
        .bind(plugin_repo_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn unlink(&self, repository_id: &str, plugin_repo_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM repository_plugin_repos WHERE repository_id = $1 AND plugin_repo_id = $2",
        )
        .bind(repository_id)
        .bind(plugin_repo_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
