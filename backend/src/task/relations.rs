//! Typed relations between tasks (e.g. `created_by`).
//!
//! Each row links one task (`from_task_id`) to another (`to_task_id`) with a
//! typed `relation_type` and an optional JSON `context` describing why/how
//! the link was created.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use tracing::warn;

use crate::db::DbPool;

pub use moatless_api_schemas::enums::TaskRelationType;

/// Directive carried from a task-creation site to record a `created_by`
/// relation immediately after the child task is inserted.
#[derive(Debug, Clone)]
pub struct CreatedByParent {
    pub parent_task_id: String,
    /// Free-form metadata describing why/how the relation was created
    /// (e.g. `{"via":"github_pr_webhook","subject_kind":"github_pr","subject_key":"o/r#42"}`).
    pub context: serde_json::Value,
}

/// Directive carried from a task-creation site to record a `related_to`
/// relation immediately after the new task is inserted. Use this when a
/// webhook spawned the new task and we discovered a peer task that shares the
/// same external artifact (binding subject_key) — the peer is not a parent.
#[derive(Debug, Clone)]
pub struct RelatedTask {
    pub related_task_id: String,
    pub context: serde_json::Value,
}

/// A row from the `task_relations` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskRelation {
    pub id: String,
    pub from_task_id: String,
    pub to_task_id: String,
    pub relation_type: TaskRelationType,
    pub context: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl TaskRelation {
    /// Build a new row with a freshly-generated id and `now()` timestamp.
    pub fn new(
        from_task_id: impl Into<String>,
        to_task_id: impl Into<String>,
        relation_type: TaskRelationType,
        context: serde_json::Value,
    ) -> Self {
        Self {
            id: format!("tr_{}", uuid::Uuid::new_v4()),
            from_task_id: from_task_id.into(),
            to_task_id: to_task_id.into(),
            relation_type,
            context,
            created_at: Utc::now(),
        }
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRelationRepository: Send + Sync {
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

    /// The single `created_by` parent of `from_task_id`, if any.
    async fn find_created_by(
        &self,
        from_task_id: &str,
    ) -> Result<Option<TaskRelation>, sqlx::Error>;

    /// Batch lookup of `created_by` parents for many child tasks.
    /// Returned map keys are `from_task_id`; missing keys mean no parent.
    async fn get_created_by_map(
        &self,
        from_task_ids: &[String],
    ) -> Result<std::collections::HashMap<String, String>, sqlx::Error>;
}

pub struct PgTaskRelationRepository(pub DbPool);

const TASK_RELATION_SELECT: &str = r#"
    SELECT id, from_task_id, to_task_id, relation_type, context, created_at
    FROM task_relations
"#;

#[async_trait]
impl TaskRelationRepository for PgTaskRelationRepository {
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
        .bind(row.relation_type)
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
            .bind(kind.map(|k| k.as_str()))
            .fetch_all(&self.0)
            .await
    }

    async fn find_created_by(
        &self,
        from_task_id: &str,
    ) -> Result<Option<TaskRelation>, sqlx::Error> {
        sqlx::query_as::<_, TaskRelation>(AssertSqlSafe(format!(
            "{TASK_RELATION_SELECT} WHERE from_task_id = $1 AND relation_type = 'created_by' \
            ORDER BY created_at ASC LIMIT 1"
        )))
        .bind(from_task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn get_created_by_map(
        &self,
        from_task_ids: &[String],
    ) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
        if from_task_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let rows: Vec<(String, String)> = sqlx::query_as(
            r#"SELECT DISTINCT ON (from_task_id) from_task_id, to_task_id
               FROM task_relations
               WHERE from_task_id = ANY($1) AND relation_type = 'created_by'
               ORDER BY from_task_id, created_at ASC"#,
        )
        .bind(from_task_ids)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().collect())
    }
}

/// Fire-and-forget helper to record a `created_by` relation. Errors are
/// logged at WARN and swallowed — callers are typically in webhook paths
/// where the task itself was already created successfully and we don't want
/// to fail the whole request because the linkage couldn't be stored.
pub async fn record_created_by(
    repo: &dyn TaskRelationRepository,
    from_task_id: &str,
    parent: &CreatedByParent,
) {
    let row = TaskRelation::new(
        from_task_id,
        &parent.parent_task_id,
        TaskRelationType::CreatedBy,
        parent.context.clone(),
    );
    if let Err(e) = repo.insert(&row).await {
        warn!(
            from_task_id,
            parent_task_id = %parent.parent_task_id,
            error = %e,
            "Failed to record created_by relation"
        );
    }
}

/// Fire-and-forget helper to record a `related_to` peer relation. Same
/// swallow-and-log semantics as `record_created_by`. Used by webhook paths
/// that discover a peer task via shared bindings, where the peer is not a
/// parent.
pub async fn record_related_to(
    repo: &dyn TaskRelationRepository,
    from_task_id: &str,
    related: &RelatedTask,
) {
    let row = TaskRelation::new(
        from_task_id,
        &related.related_task_id,
        TaskRelationType::RelatedTo,
        related.context.clone(),
    );
    if let Err(e) = repo.insert(&row).await {
        warn!(
            from_task_id,
            related_task_id = %related.related_task_id,
            error = %e,
            "Failed to record related_to relation"
        );
    }
}
