//! Typed relations between tasks (e.g. `created_by`).
//!
//! Each row links one task (`from_task_id`) to another (`to_task_id`) with a
//! typed `relation_type` and an optional JSON `context` describing why/how
//! the link was created.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
pub use moatless_types::task::TaskRelationType;
use sqlx::AssertSqlSafe;
use tracing::warn;

use moatless_core::db::DbPool;
use moatless_types::task::RelatedTask;

/// Directive carried from a task-creation site to record a `created_by`
/// relation immediately after the child task is inserted.
#[derive(Debug, Clone)]
pub struct CreatedByParent {
    pub parent_task_id: String,
    /// Free-form metadata describing why/how the relation was created
    /// (e.g. `{"via":"github_pr_webhook","subject_kind":"github_pr","subject_key":"o/r#42"}`).
    pub context: serde_json::Value,
}

/// A row from the `task_relations` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskRelation {
    pub id: String,
    pub from_task_id: String,
    pub to_task_id: String,
    #[sqlx(try_from = "String")]
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

/// Fire-and-forget helper to record a `created_by` relation. Errors are
/// logged at WARN and swallowed — callers are typically in webhook paths
/// where the task itself was already created successfully and we don't want
/// to fail the whole request because the linkage couldn't be stored.
pub async fn record_created_by(
    repo: &dyn TaskRelationDao,
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

/// Record the `forked_from` edge from a fork to the Task it was branched off.
///
/// Not fire-and-forget, unlike its two neighbours: a fork whose edge is missing
/// shows up as an orphan with a conversation it cannot explain, and the fork
/// handler still has the transaction open to undo it. `at_turn` goes in the
/// context because that is what the presenter and the system prompt display;
/// the anchors the pod needs are typed columns on `task_forks`.
pub async fn record_forked_from(
    repo: &dyn TaskRelationDao,
    from_task_id: &str,
    source_task_id: &str,
    at_turn: Option<i32>,
) -> Result<(), sqlx::Error> {
    let row = TaskRelation::new(
        from_task_id,
        source_task_id,
        TaskRelationType::ForkedFrom,
        serde_json::json!({ "at_turn": at_turn }),
    );
    repo.insert(&row).await
}

/// Fire-and-forget helper to record a `related_to` peer relation. Same
/// swallow-and-log semantics as `record_created_by`. Used by webhook paths
/// that discover a peer task via shared bindings, where the peer is not a
/// parent.
pub async fn record_related_to(
    repo: &dyn TaskRelationDao,
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
