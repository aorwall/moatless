//! CRUD operations for channel_configs table.

use super::models::ChannelConfig;
use crate::db::DbPool;

/// Find a channel config by its ID.
pub async fn find_by_id(
    pool: &DbPool,
    id: &str,
) -> Result<Option<ChannelConfig>, sqlx::Error> {
    sqlx::query_as::<_, ChannelConfig>(
        r#"SELECT id, provider, source_id, source_name, repository_id, routing_mode,
                  prompt_template, active, provider_config, ongoing_task_id,
                  created_by, created_at, updated_at
           FROM channel_configs WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Find a channel config by provider + source_id (unique index).
pub async fn find_by_provider_source(
    pool: &DbPool,
    provider: &str,
    source_id: &str,
) -> Result<Option<ChannelConfig>, sqlx::Error> {
    sqlx::query_as::<_, ChannelConfig>(
        r#"SELECT id, provider, source_id, source_name, repository_id, routing_mode,
                  prompt_template, active, provider_config, ongoing_task_id,
                  created_by, created_at, updated_at
           FROM channel_configs WHERE provider = $1 AND source_id = $2"#,
    )
    .bind(provider)
    .bind(source_id)
    .fetch_optional(pool)
    .await
}

/// List channel configs with optional filters.
pub async fn list(
    pool: &DbPool,
    provider: Option<&str>,
    repository_id: Option<&str>,
    active_only: bool,
) -> Result<Vec<ChannelConfig>, sqlx::Error> {
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
        sql.push_str(&format!(" AND provider = ${param_idx}"));
        binds.push(p.to_string());
    }
    if let Some(r) = repository_id {
        param_idx += 1;
        sql.push_str(&format!(" AND repository_id = ${param_idx}"));
        binds.push(r.to_string());
    }
    if active_only {
        sql.push_str(" AND active = true");
    }

    sql.push_str(" ORDER BY created_at DESC");

    // Build query dynamically
    let mut query = sqlx::query_as::<_, ChannelConfig>(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    query.fetch_all(pool).await
}

/// Insert a new channel config.
pub async fn insert(pool: &DbPool, row: &ChannelConfig) -> Result<(), sqlx::Error> {
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
    .bind(&row.routing_mode)
    .bind(&row.prompt_template)
    .bind(row.active)
    .bind(&row.provider_config)
    .bind(&row.ongoing_task_id)
    .bind(&row.created_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Updateable fields for a channel config.
#[derive(Default)]
pub struct ChannelConfigPatch {
    pub source_name: Option<String>,
    pub routing_mode: Option<String>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<Option<String>>,
}

/// Update a channel config by ID (only non-None fields are updated).
pub async fn update(
    pool: &DbPool,
    id: &str,
    patch: &ChannelConfigPatch,
) -> Result<(), sqlx::Error> {
    let mut sets = Vec::new();
    let mut param_idx = 1u32; // $1 = id

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
    if let Some(ref v) = patch.routing_mode {
        query = query.bind(v);
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

    query.execute(pool).await?;
    Ok(())
}

/// Delete a channel config by ID.
pub async fn delete(pool: &DbPool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM channel_configs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
