//! Task relation persistence and parent lookup SQL.

use super::relations::{TaskRelation, TaskRelationType};
use async_trait::async_trait;
use moatless_core::db::DbPool;
use sqlx::AssertSqlSafe;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRelationDao: Send + Sync {
    /// Insert a relation. `ON CONFLICT (from, to, relation_type) DO NOTHING`.
    async fn insert(&self, row: &TaskRelation) -> Result<(), sqlx::Error>;

    /// All relations originating from `from_task_id`.
    async fn list_outgoing(&self, from_task_id: &str) -> Result<Vec<TaskRelation>, sqlx::Error>;

    /// All relations pointing at `to_task_id`, optionally filtered by type.
    async fn list_incoming(
        &self,
        to_task_id: &str,
        kind: Option<TaskRelationType>,
    ) -> Result<Vec<TaskRelation>, sqlx::Error>;

    /// The single parent of `from_task_id`, if any — the `created_by` task
    /// that spawned it or the `forked_from` task it was branched off. The
    /// returned row's `relation_type` says which, because the two read the same
    /// in a listing and differently in a prompt.
    async fn find_parent(&self, from_task_id: &str) -> Result<Option<TaskRelation>, sqlx::Error>;

    /// Batch lookup of parents for many child tasks, over the same two types as
    /// [`TaskRelationDao::find_parent`]. Returned map keys are `from_task_id`;
    /// missing keys mean no parent.
    async fn get_parent_map(
        &self,
        from_task_ids: &[String],
    ) -> Result<std::collections::HashMap<String, String>, sqlx::Error>;
}

/// The two relation types that make `from_task_id` a child of `to_task_id`.
///
/// Bound as a `text[]` rather than interpolated: these are data values, and the
/// SQL that reads them must stay parameterised.
pub const PARENT_RELATION_TYPES: [&str; 2] = ["created_by", "forked_from"];

pub struct PgTaskRelationDao(pub DbPool);

const TASK_RELATION_SELECT: &str = r#"
    SELECT id, from_task_id, to_task_id, relation_type, context, created_at
    FROM task_relations
"#;

#[async_trait]
impl TaskRelationDao for PgTaskRelationDao {
    async fn insert(&self, row: &TaskRelation) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO task_relations
                   (id, from_task_id, to_task_id, relation_type, context, created_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (from_task_id, to_task_id, relation_type) DO NOTHING"#,
        )
        .bind(&row.id)
        .bind(&row.from_task_id)
        .bind(&row.to_task_id)
        .bind(row.relation_type.as_str())
        .bind(&row.context)
        .bind(row.created_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list_outgoing(&self, from_task_id: &str) -> Result<Vec<TaskRelation>, sqlx::Error> {
        sqlx::query_as::<_, TaskRelation>(AssertSqlSafe(format!(
            "{TASK_RELATION_SELECT} WHERE from_task_id = $1 ORDER BY created_at ASC"
        )))
        .bind(from_task_id)
        .fetch_all(&self.0)
        .await
    }

    async fn list_incoming(
        &self,
        to_task_id: &str,
        kind: Option<TaskRelationType>,
    ) -> Result<Vec<TaskRelation>, sqlx::Error> {
        let sql = format!(
            "{TASK_RELATION_SELECT} WHERE to_task_id = $1 \
              AND ($2::text IS NULL OR relation_type = $2) \
            ORDER BY created_at ASC"
        );
        sqlx::query_as::<_, TaskRelation>(AssertSqlSafe(sql))
            .bind(to_task_id)
            .bind(kind.map(TaskRelationType::as_str))
            .fetch_all(&self.0)
            .await
    }

    async fn find_parent(&self, from_task_id: &str) -> Result<Option<TaskRelation>, sqlx::Error> {
        sqlx::query_as::<_, TaskRelation>(AssertSqlSafe(format!(
            "{TASK_RELATION_SELECT} WHERE from_task_id = $1 AND relation_type = ANY($2) \
            ORDER BY created_at ASC LIMIT 1"
        )))
        .bind(from_task_id)
        .bind(&PARENT_RELATION_TYPES[..])
        .fetch_optional(&self.0)
        .await
    }

    async fn get_parent_map(
        &self,
        from_task_ids: &[String],
    ) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
        if from_task_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let rows: Vec<(String, String)> = sqlx::query_as(
            r#"SELECT DISTINCT ON (from_task_id) from_task_id, to_task_id
               FROM task_relations
               WHERE from_task_id = ANY($1) AND relation_type = ANY($2)
               ORDER BY from_task_id, created_at ASC"#,
        )
        .bind(from_task_ids)
        .bind(&PARENT_RELATION_TYPES[..])
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().collect())
    }
}
