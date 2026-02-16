//! CRUD operations for channel_configs table.

use async_trait::async_trait;

use super::models::{ChannelConfig, RoutingMode};
use crate::db::DbPool;

/// Updateable fields for a channel config.
#[derive(Default)]
pub struct ChannelConfigPatch {
    pub source_name: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<Option<String>>,
}

/// Repository trait for channel config persistence.
///
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` in `list`.
#[async_trait]
pub trait ChannelConfigRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<ChannelConfig>, sqlx::Error>;
    async fn find_by_provider_source(&self, provider: &str, source_id: &str) -> Result<Option<ChannelConfig>, sqlx::Error>;
    async fn list(&self, provider: Option<&str>, repository_id: Option<&str>, active_only: bool) -> Result<Vec<ChannelConfig>, sqlx::Error>;
    async fn insert(&self, row: &ChannelConfig) -> Result<(), sqlx::Error>;
    async fn update(&self, id: &str, patch: &ChannelConfigPatch) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`ChannelConfigRepository`].
pub struct PgChannelConfigRepository(pub DbPool);

#[async_trait]
impl ChannelConfigRepository for PgChannelConfigRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<ChannelConfig>, sqlx::Error> {
        sqlx::query_as::<_, ChannelConfig>(
            r#"SELECT id, provider, source_id, source_name, repository_id, routing_mode,
                      prompt_template, active, provider_config, ongoing_task_id,
                      created_by, created_at, updated_at
               FROM channel_configs WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_provider_source(&self, provider: &str, source_id: &str) -> Result<Option<ChannelConfig>, sqlx::Error> {
        sqlx::query_as::<_, ChannelConfig>(
            r#"SELECT id, provider, source_id, source_name, repository_id, routing_mode,
                      prompt_template, active, provider_config, ongoing_task_id,
                      created_by, created_at, updated_at
               FROM channel_configs WHERE provider = $1 AND source_id = $2"#,
        )
        .bind(provider)
        .bind(source_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn list(&self, provider: Option<&str>, repository_id: Option<&str>, active_only: bool) -> Result<Vec<ChannelConfig>, sqlx::Error> {
        let mut sql = String::from(
            r#"SELECT id, provider, source_id, source_name, repository_id, routing_mode,
                      prompt_template, active, provider_config, ongoing_task_id,
                      created_by, created_at, updated_at
               FROM channel_configs WHERE 1=1"#,
        );
        let mut param_idx = 0u32;
        let mut binds: Vec<String> = Vec::new();

        if let Some(p) = provider {
            param_idx += 1;
            use std::fmt::Write;
            let _ = write!(sql, " AND provider = ${param_idx}");
            binds.push(p.to_string());
        }
        if let Some(r) = repository_id {
            param_idx += 1;
            use std::fmt::Write;
            let _ = write!(sql, " AND repository_id = ${param_idx}");
            binds.push(r.to_string());
        }
        if active_only {
            sql.push_str(" AND active = true");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let mut query = sqlx::query_as::<_, ChannelConfig>(&sql);
        for b in &binds {
            query = query.bind(b);
        }
        query.fetch_all(&self.0).await
    }

    async fn insert(&self, row: &ChannelConfig) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO channel_configs
               (id, provider, source_id, source_name, repository_id, routing_mode,
                prompt_template, active, provider_config, ongoing_task_id, created_by)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        )
        .bind(&row.id)
        .bind(&row.provider)
        .bind(&row.source_id)
        .bind(&row.source_name)
        .bind(&row.repository_id)
        .bind(row.routing_mode)
        .bind(&row.prompt_template)
        .bind(row.active)
        .bind(&row.provider_config)
        .bind(&row.ongoing_task_id)
        .bind(&row.created_by)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn update(&self, id: &str, patch: &ChannelConfigPatch) -> Result<(), sqlx::Error> {
        let mut sets = Vec::new();
        let mut param_idx = 1u32;

        if patch.source_name.is_some() {
            param_idx += 1;
            sets.push(format!("source_name = ${param_idx}"));
        }
        if patch.routing_mode.is_some() {
            param_idx += 1;
            sets.push(format!("routing_mode = ${param_idx}"));
        }
        if patch.prompt_template.is_some() {
            param_idx += 1;
            sets.push(format!("prompt_template = ${param_idx}"));
        }
        if patch.active.is_some() {
            param_idx += 1;
            sets.push(format!("active = ${param_idx}"));
        }
        if patch.provider_config.is_some() {
            param_idx += 1;
            sets.push(format!("provider_config = ${param_idx}"));
        }
        if patch.ongoing_task_id.is_some() {
            param_idx += 1;
            sets.push(format!("ongoing_task_id = ${param_idx}"));
        }

        if sets.is_empty() {
            return Ok(());
        }

        sets.push("updated_at = now()".to_string());

        let sql = format!("UPDATE channel_configs SET {} WHERE id = $1", sets.join(", "));

        let mut query = sqlx::query(&sql).bind(id);

        if let Some(ref v) = patch.source_name {
            query = query.bind(v);
        }
        if let Some(v) = patch.routing_mode {
            query = query.bind(v.as_str());
        }
        if let Some(ref v) = patch.prompt_template {
            query = query.bind(v.as_deref());
        }
        if let Some(v) = patch.active {
            query = query.bind(v);
        }
        if let Some(ref v) = patch.provider_config {
            query = query.bind(v);
        }
        if let Some(ref v) = patch.ongoing_task_id {
            query = query.bind(v.as_deref());
        }

        query.execute(&self.0).await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM channel_configs WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}
