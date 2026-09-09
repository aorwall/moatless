//! Typed relations between tasks (e.g. `created_by`).
//!
//! Each row links one task (`from_task_id`) to another (`to_task_id`) with a
//! typed `relation_type` and an optional JSON `context` describing why/how
//! the link was created.

use std::sync::Arc;

use chrono::{DateTime, Utc};
pub use moatless_types::task::TaskRelationType;
use tracing::warn;

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

#[cfg(test)]
pub(crate) use super::relations_dao::MockTaskRelationDao;
pub(crate) use super::relations_dao::TaskRelationDao;

/// The `task_relations` table, as its peers read and write it.
pub struct TaskRelations {
    relations: Arc<dyn TaskRelationDao>,
}

impl TaskRelations {
    pub(crate) fn new(relations: Arc<dyn TaskRelationDao>) -> Self {
        Self { relations }
    }

    /// The single parent of `from_task_id`, if any. See
    /// [`TaskRelationDao::find_parent`].
    pub async fn find_parent(
        &self,
        from_task_id: &str,
    ) -> Result<Option<TaskRelation>, sqlx::Error> {
        self.relations.find_parent(from_task_id).await
    }

    /// Every relation pointing at `to_task_id`, optionally filtered by type.
    pub async fn list_incoming(
        &self,
        to_task_id: &str,
        kind: Option<TaskRelationType>,
    ) -> Result<Vec<TaskRelation>, sqlx::Error> {
        self.relations.list_incoming(to_task_id, kind).await
    }

    /// Record a `related_to` relation. See [`record_related_to`].
    pub(crate) async fn record_related_to(&self, from_task_id: &str, related: &RelatedTask) {
        record_related_to(self.relations.as_ref(), from_task_id, related).await;
    }
}

/// Fire-and-forget helper to record a `created_by` relation. Errors are
/// logged at WARN and swallowed — callers are typically in webhook paths
/// where the task itself was already created successfully and we don't want
/// to fail the whole request because the linkage couldn't be stored.
pub(crate) async fn record_created_by(
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
pub(crate) async fn record_forked_from(
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
pub(crate) async fn record_related_to(
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
