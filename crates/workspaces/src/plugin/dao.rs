//! Plugin DAO — queries for plugin data access.

use async_trait::async_trait;
use uuid::Uuid;

use super::models::{ActivationRow, ActivationWrite, Plugin, PluginSkill};
use moatless_core::db::DbPool;

/// DAO trait for plugin persistence.
#[async_trait]
pub(crate) trait PluginDao: Send + Sync {
    async fn list_all(&self) -> Result<Vec<Plugin>, sqlx::Error>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Plugin>, sqlx::Error>;
    async fn insert(&self, row: &Plugin) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error>;

    /// The skills stored for a plugin, by name.
    async fn list_skills(&self, plugin_id: &str) -> Result<Vec<PluginSkill>, sqlx::Error>;

    /// Replace a plugin's stored skills and record the version they were read
    /// at, in one transaction. A sync that got as far as parsing skills but
    /// failed to store them must not leave the version claiming otherwise.
    async fn replace_skills(
        &self,
        plugin_id: &str,
        version: Option<String>,
        skills: &[PluginSkill],
    ) -> Result<(), sqlx::Error>;

    /// Every plugin's stored skills in one query, so resolution does not fan
    /// out per plugin.
    async fn list_all_skills(&self) -> Result<Vec<PluginSkill>, sqlx::Error>;

    /// The records resolution needs for one person in one Workspace.
    ///
    /// Every global record, plus that person's own, plus that Workspace's —
    /// nobody else's. `None` for either omits its records; `(None, None)` is
    /// the global set alone.
    async fn list_activations_for(
        &self,
        user_id: Option<Uuid>,
        workspace_id: Option<&str>,
    ) -> Result<Vec<ActivationRow>, sqlx::Error>;

    /// Set one activation record, replacing whatever it said before, and read
    /// back what now stands. A flag the write left unsaid keeps its stored
    /// value, so the caller is told the record rather than its own request.
    async fn set_activation(&self, write: &ActivationWrite) -> Result<ActivationRow, sqlx::Error>;

    /// Remove one activation record. Clearing a personal or Workspace record
    /// hands it back to whatever the next reach down says, which is not the
    /// same as setting it to off.
    async fn clear_activation(
        &self,
        plugin_id: &str,
        skill_name: Option<String>,
        user_id: Option<Uuid>,
        workspace_id: Option<&str>,
    ) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`PluginDao`].
pub(crate) struct PgPluginDao(pub(crate) DbPool);

#[async_trait]
impl PluginDao for PgPluginDao {
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

    async fn list_skills(&self, plugin_id: &str) -> Result<Vec<PluginSkill>, sqlx::Error> {
        sqlx::query_as::<_, PluginSkill>(
            "SELECT plugin_id, name, description, path FROM plugin_skills WHERE plugin_id = $1 ORDER BY name ASC",
        )
        .bind(plugin_id)
        .fetch_all(&self.0)
        .await
    }

    async fn replace_skills(
        &self,
        plugin_id: &str,
        version: Option<String>,
        skills: &[PluginSkill],
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;

        sqlx::query("DELETE FROM plugin_skills WHERE plugin_id = $1")
            .bind(plugin_id)
            .execute(&mut *tx)
            .await?;

        for skill in skills {
            sqlx::query(
                "INSERT INTO plugin_skills (plugin_id, name, description, path) VALUES ($1, $2, $3, $4)",
            )
            .bind(plugin_id)
            .bind(&skill.name)
            .bind(&skill.description)
            .bind(&skill.path)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query("UPDATE plugins SET synced_version = $2, synced_at = now() WHERE id = $1")
            .bind(plugin_id)
            .bind(&version)
            .execute(&mut *tx)
            .await?;

        tx.commit().await
    }

    async fn list_all_skills(&self) -> Result<Vec<PluginSkill>, sqlx::Error> {
        sqlx::query_as::<_, PluginSkill>(
            "SELECT plugin_id, name, description, path FROM plugin_skills ORDER BY plugin_id, name",
        )
        .fetch_all(&self.0)
        .await
    }

    async fn list_activations_for(
        &self,
        user_id: Option<Uuid>,
        workspace_id: Option<&str>,
    ) -> Result<Vec<ActivationRow>, sqlx::Error> {
        // Three subjects, each admitted on its own terms: the global records
        // always, the named person's own, the named Workspace's. A row belongs
        // to exactly one of them, so these cannot overlap.
        sqlx::query_as::<_, ActivationRow>(
            r#"SELECT plugin_id, skill_name, user_id, workspace_id, enabled,
                      "always", frontload
               FROM plugin_activations
               WHERE (user_id IS NULL AND workspace_id IS NULL)
                  OR user_id = $1
                  OR workspace_id = $2"#,
        )
        .bind(user_id)
        .bind(workspace_id)
        .fetch_all(&self.0)
        .await
    }

    async fn set_activation(&self, write: &ActivationWrite) -> Result<ActivationRow, sqlx::Error> {
        // `COALESCE($6, plugin_activations."always")` is what makes an unsaid
        // flag leave the stored one alone; on the insert half there is no
        // stored one, so it falls to the column default.
        sqlx::query_as::<_, ActivationRow>(
            r#"INSERT INTO plugin_activations (plugin_id, skill_name, user_id, workspace_id, enabled, "always", frontload)
               VALUES ($1, $2, $3, $4, $5, COALESCE($6, false), COALESCE($7, false))
               ON CONFLICT (plugin_id, skill_name, user_id, workspace_id)
               DO UPDATE SET enabled = EXCLUDED.enabled,
                             "always" = COALESCE($6, plugin_activations."always"),
                             frontload = COALESCE($7, plugin_activations.frontload),
                             updated_at = now()
               RETURNING plugin_id, skill_name, user_id, workspace_id, enabled, "always", frontload"#,
        )
        .bind(&write.plugin_id)
        .bind(&write.skill_name)
        .bind(write.user_id)
        .bind(&write.workspace_id)
        .bind(write.enabled)
        .bind(write.always)
        .bind(write.frontload)
        .fetch_one(&self.0)
        .await
    }

    async fn clear_activation(
        &self,
        plugin_id: &str,
        skill_name: Option<String>,
        user_id: Option<Uuid>,
        workspace_id: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        // `IS NOT DISTINCT FROM` so a NULL argument matches the NULL row —
        // plain `=` would never match the global or whole-plugin records.
        let result = sqlx::query(
            r#"DELETE FROM plugin_activations
               WHERE plugin_id = $1
                 AND skill_name IS NOT DISTINCT FROM $2
                 AND user_id IS NOT DISTINCT FROM $3
                 AND workspace_id IS NOT DISTINCT FROM $4"#,
        )
        .bind(plugin_id)
        .bind(&skill_name)
        .bind(user_id)
        .bind(workspace_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod db_tests;
