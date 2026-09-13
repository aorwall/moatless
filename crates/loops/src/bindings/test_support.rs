//! Binding row fixtures for a DB-backed test, so no test names the private
//! store.

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;
use moatless_types::adapter::TaskBinding;

use crate::bindings::operations::Bindings;

/// Write a Binding a host would have written itself.
///
/// [`Bindings::bind`] mints the id and timestamps; a test that later reads a
/// row back by id has to name both.
pub async fn upsert_binding(
    bindings: &Bindings,
    row: &TaskBinding,
) -> Result<TaskBinding, sqlx::Error> {
    bindings.upsert(row).await
}

/// The identity of a Binding row a fixture seeds.
pub struct BindingFixture<'a> {
    pub id: &'a str,
    pub task_id: &'a str,
    pub adapter_kind: &'a str,
    pub subject_kind: &'a str,
    pub subject_key: &'a str,
    pub connection_id: &'a str,
    pub origin: &'a str,
    pub external_state_kind: Option<&'a str>,
}

/// Seed a Binding in the shape the outbound path writes: it carries no status
/// blob until a refresh fetches one.
pub async fn insert_binding(db: &DbPool, row: &BindingFixture<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO task_bindings
               (id, task_id, adapter_kind, subject_kind, subject_key, connection_id,
                destination_config, origin, external_state_kind)
           VALUES ($1, $2, $3, $4, $5, $6, '{}'::jsonb, $7, $8)"#,
    )
    .bind(row.id)
    .bind(row.task_id)
    .bind(row.adapter_kind)
    .bind(row.subject_kind)
    .bind(row.subject_key)
    .bind(row.connection_id)
    .bind(row.origin)
    .bind(row.external_state_kind)
    .execute(db)
    .await?;
    Ok(())
}

/// Fill in the status blob a refresh writes, at the instant a test names.
pub async fn set_external_status_raw(
    db: &DbPool,
    binding_id: &str,
    raw: &serde_json::Value,
    updated_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE task_bindings
              SET external_status_raw = $2,
                  external_status_updated_at = $3
            WHERE id = $1"#,
    )
    .bind(binding_id)
    .bind(raw)
    .bind(updated_at)
    .execute(db)
    .await?;
    Ok(())
}
