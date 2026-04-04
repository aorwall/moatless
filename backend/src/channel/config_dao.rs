//! CRUD operations for channel_configs table.

use async_trait::async_trait;

use super::models::{ChannelConfig, RoutingMode};
use crate::db::DbPool;
use crate::scope::{ScopeContext, ScopedEntity};

const CHANNEL_CONFIG_COLUMNS: &str = r#"id, provider, source_id, source_name, repository_id, routing_mode,
                      prompt_template, active, allow_direct_send, provider_config, ongoing_task_id,
                      response_config_id, scope, team_id, task_name, tag_ids,
                      run_as_user_id, created_by, created_at, updated_at"#;

/// Updateable fields for a channel config.
#[derive(Default)]
pub struct ChannelConfigPatch {
    pub source_name: Option<String>,
    pub repository_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<Option<String>>,
    pub response_config_id: Option<Option<String>>,
    pub task_name: Option<Option<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub run_as_user_id: Option<Option<String>>,
    pub allow_direct_send: Option<bool>,
}

/// Repository trait for channel config persistence.
///
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` in `list`.
#[async_trait]
pub trait ChannelConfigRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<ChannelConfig>, sqlx::Error>;
    async fn find_by_id_scoped(&self, id: &str, ctx: &ScopeContext) -> Result<Option<ChannelConfig>, sqlx::Error>;
    async fn find_by_provider_source(&self, provider: &str, source_id: &str) -> Result<Option<ChannelConfig>, sqlx::Error>;
    async fn list(&self, provider: Option<&str>, repository_id: Option<&str>, active_only: bool) -> Result<Vec<ChannelConfig>, sqlx::Error>;
    async fn list_scoped(&self, provider: Option<&str>, repository_id: Option<&str>, active_only: bool, ctx: &ScopeContext) -> Result<Vec<ChannelConfig>, sqlx::Error>;
    async fn insert(&self, row: &ChannelConfig) -> Result<(), sqlx::Error>;
    async fn update(&self, id: &str, patch: &ChannelConfigPatch) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<(), sqlx::Error>;

    /// Find a wildcard (`source_id="*"`) config for the given provider.
    ///
    /// Lookup order: team-scoped wildcard first (if `team_id` is provided),
    /// then global wildcard.
    async fn find_wildcard(&self, provider: &str, team_id: Option<&str>) -> Result<Option<ChannelConfig>, sqlx::Error>;
}

/// PostgreSQL implementation of [`ChannelConfigRepository`].
pub struct PgChannelConfigRepository(pub DbPool);

#[async_trait]
impl ChannelConfigRepository for PgChannelConfigRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<ChannelConfig>, sqlx::Error> {
        let sql = format!("SELECT {CHANNEL_CONFIG_COLUMNS} FROM channel_configs WHERE id = $1");
        sqlx::query_as::<_, ChannelConfig>(&sql)
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    async fn find_by_id_scoped(&self, id: &str, ctx: &ScopeContext) -> Result<Option<ChannelConfig>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::ChannelConfig, "", 2);
        let sql = format!(
            "SELECT {CHANNEL_CONFIG_COLUMNS} FROM channel_configs \
             WHERE id = $1 \
             AND {}", access.sql()
        );
        sqlx::query_as::<_, ChannelConfig>(&sql)
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn find_by_provider_source(&self, provider: &str, source_id: &str) -> Result<Option<ChannelConfig>, sqlx::Error> {
        let sql = format!(
            "SELECT {CHANNEL_CONFIG_COLUMNS} FROM channel_configs WHERE provider = $1 AND source_id = $2"
        );
        sqlx::query_as::<_, ChannelConfig>(&sql)
            .bind(provider)
            .bind(source_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn list(&self, provider: Option<&str>, repository_id: Option<&str>, active_only: bool) -> Result<Vec<ChannelConfig>, sqlx::Error> {
        let mut sql = format!(
            "SELECT {CHANNEL_CONFIG_COLUMNS} FROM channel_configs WHERE 1=1"
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

    async fn list_scoped(&self, provider: Option<&str>, repository_id: Option<&str>, active_only: bool, ctx: &ScopeContext) -> Result<Vec<ChannelConfig>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::ChannelConfig, "", 1);
        let mut sql = format!(
            "SELECT {CHANNEL_CONFIG_COLUMNS} FROM channel_configs \
             WHERE {}", access.sql()
        );
        let mut param_idx = 3u32;
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

        let mut query = sqlx::query_as::<_, ChannelConfig>(&sql)
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id);
        for b in &binds {
            query = query.bind(b);
        }
        query.fetch_all(&self.0).await
    }

    async fn insert(&self, row: &ChannelConfig) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO channel_configs
               (id, provider, source_id, source_name, repository_id, routing_mode,
                prompt_template, active, allow_direct_send, provider_config, ongoing_task_id,
                response_config_id, scope, team_id, task_name, tag_ids,
                run_as_user_id, created_by)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)"#,
        )
        .bind(&row.id)
        .bind(&row.provider)
        .bind(&row.source_id)
        .bind(&row.source_name)
        .bind(&row.repository_id)
        .bind(row.routing_mode)
        .bind(&row.prompt_template)
        .bind(row.active)
        .bind(row.allow_direct_send)
        .bind(&row.provider_config)
        .bind(&row.ongoing_task_id)
        .bind(&row.response_config_id)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(&row.task_name)
        .bind(&row.tag_ids)
        .bind(&row.run_as_user_id)
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
        if patch.repository_id.is_some() {
            param_idx += 1;
            sets.push(format!("repository_id = ${param_idx}"));
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
        if patch.response_config_id.is_some() {
            param_idx += 1;
            sets.push(format!("response_config_id = ${param_idx}"));
        }
        if patch.task_name.is_some() {
            param_idx += 1;
            sets.push(format!("task_name = ${param_idx}"));
        }
        if patch.tag_ids.is_some() {
            param_idx += 1;
            sets.push(format!("tag_ids = ${param_idx}"));
        }
        if patch.run_as_user_id.is_some() {
            param_idx += 1;
            sets.push(format!("run_as_user_id = ${param_idx}"));
        }
        if patch.allow_direct_send.is_some() {
            param_idx += 1;
            sets.push(format!("allow_direct_send = ${param_idx}"));
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
        if let Some(ref v) = patch.repository_id {
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
        if let Some(ref v) = patch.response_config_id {
            query = query.bind(v.as_deref());
        }
        if let Some(ref v) = patch.task_name {
            query = query.bind(v.as_deref());
        }
        if let Some(ref v) = patch.tag_ids {
            query = query.bind(v);
        }
        if let Some(ref v) = patch.run_as_user_id {
            query = query.bind(v.as_deref());
        }
        if let Some(v) = patch.allow_direct_send {
            query = query.bind(v);
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

    async fn find_wildcard(&self, provider: &str, team_id: Option<&str>) -> Result<Option<ChannelConfig>, sqlx::Error> {
        // Try team-scoped wildcard first
        if let Some(tid) = team_id {
            let sql = format!(
                "SELECT {CHANNEL_CONFIG_COLUMNS} FROM channel_configs \
                 WHERE provider = $1 AND source_id = '*' AND team_id = $2 AND active = true \
                 LIMIT 1"
            );
            let result = sqlx::query_as::<_, ChannelConfig>(&sql)
                .bind(provider)
                .bind(tid)
                .fetch_optional(&self.0)
                .await?;
            if result.is_some() {
                return Ok(result);
            }
        }

        // Fall back to global wildcard (no team_id constraint)
        let sql = format!(
            "SELECT {CHANNEL_CONFIG_COLUMNS} FROM channel_configs \
             WHERE provider = $1 AND source_id = '*' AND active = true \
             ORDER BY team_id NULLS LAST \
             LIMIT 1"
        );
        sqlx::query_as::<_, ChannelConfig>(&sql)
            .bind(provider)
            .fetch_optional(&self.0)
            .await
    }
}
