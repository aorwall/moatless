//! Plugin DAO — queries for plugin data access.

use async_trait::async_trait;

use super::models::Plugin;
use crate::db::DbPool;

/// Repository trait for plugin persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PluginRepository: Send + Sync {
    async fn list_all(&self) -> Result<Vec<Plugin>, sqlx::Error>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Plugin>, sqlx::Error>;
    async fn insert(&self, row: &Plugin) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error>;
    async fn find_by_repository_id(&self, repo_id: &str) -> Result<Vec<Plugin>, sqlx::Error>;
    async fn link(&self, repository_id: &str, plugin_id: &str) -> Result<(), sqlx::Error>;
    async fn unlink(&self, repository_id: &str, plugin_id: &str) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`PluginRepository`].
pub struct PgPluginRepository(pub DbPool);

#[async_trait]
impl PluginRepository for PgPluginRepository {
    async fn list_all(&self) -> Result<Vec<Plugin>, sqlx::Error> {
        sqlx::query_as::<_, Plugin>("SELECT * FROM plugins ORDER BY name ASC")
            .fetch_all(&self.0)
            .await
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Plugin>, sqlx::Error> {
        sqlx::query_as::<_, Plugin>("SELECT * FROM plugins WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    async fn insert(&self, row: &Plugin) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO plugins (id, name, git_url) VALUES ($1, $2, $3)")
            .bind(&row.id)
            .bind(&row.name)
            .bind(&row.git_url)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM plugins WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn find_by_repository_id(&self, repo_id: &str) -> Result<Vec<Plugin>, sqlx::Error> {
        sqlx::query_as::<_, Plugin>(
            r#"SELECT pr.id, pr.name, pr.git_url, pr.created_at, pr.updated_at
               FROM plugins pr
               JOIN repository_plugins rpr ON pr.id = rpr.plugin_id
               WHERE rpr.repository_id = $1
               ORDER BY pr.name ASC"#,
        )
        .bind(repo_id)
        .fetch_all(&self.0)
        .await
    }

    async fn link(&self, repository_id: &str, plugin_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO repository_plugins (repository_id, plugin_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(repository_id)
        .bind(plugin_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn unlink(&self, repository_id: &str, plugin_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM repository_plugins WHERE repository_id = $1 AND plugin_id = $2",
        )
        .bind(repository_id)
        .bind(plugin_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
