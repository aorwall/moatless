//! Where a thread sits among the Tasks that spawned or forked it, for the V2
//! `lineage` and `forkedFrom` fields.

use moatless_types::access_control::ScopeContext;
use tracing::warn;

use crate::ports::TaskRelationSource;
use crate::projection::Lineage;
use crate::reads::T3Reads;

/// How far up a chain of parents the root is looked for. A deeper chain
/// reports the furthest ancestor reached as its root.
const MAX_ANCESTRY: usize = 16;

/// `task_id`'s parent edge and root, as far as this viewer may read them.
///
/// An unreadable parent ends the walk where it stands, so the lineage never
/// names a Task the viewer cannot open. A failed read is logged and answers a
/// root: the thread still opens, without its relationship.
pub(crate) async fn resolve(
    reads: &T3Reads,
    relations: &dyn TaskRelationSource,
    viewer: &ScopeContext,
    task_id: &str,
) -> Lineage {
    let Some(parent) = readable_parent(reads, relations, viewer, task_id).await else {
        return Lineage::root(task_id);
    };
    let mut root = parent.task_id.clone();
    for _ in 0..MAX_ANCESTRY {
        match readable_parent(reads, relations, viewer, &root).await {
            Some(next) => root = next.task_id,
            None => break,
        }
    }
    Lineage::child(parent.task_id, parent.relation, root, parent.at_turn)
}

async fn readable_parent(
    reads: &T3Reads,
    relations: &dyn TaskRelationSource,
    viewer: &ScopeContext,
    task_id: &str,
) -> Option<crate::ports::ParentTask> {
    let parent = match relations.parent(task_id).await {
        Ok(parent) => parent?,
        Err(error) => {
            warn!(%error, task_id, "T3 UI could not read a thread's parent; showing it as a root");
            return None;
        }
    };
    match reads
        .conversations()
        .readable_by_id(viewer, &parent.task_id)
        .await
    {
        Ok(Some(_)) => Some(parent),
        Ok(None) => None,
        Err(error) => {
            warn!(%error, task_id, "T3 UI could not read a thread's parent; showing it as a root");
            None
        }
    }
}

/// The listing's `LEFT JOIN LATERAL … parent`: each row's first parent edge,
/// picked as `TaskRelationSource::parent` picks it, named only when the viewer
/// may read the parent (the listing's `$1`/`$2`) and follows it (`$follower`).
/// A client hides a subagent under its parent, so a parent missing from this
/// viewer's listing would hide the child too.
pub(crate) fn listed_parent_join(follower: usize) -> String {
    let access = moatless_core::db::access_control::access_filter(
        moatless_types::access_control::ScopedEntity::Task,
        "p.",
        1,
    );
    let access = access.sql();
    format!(
        "LEFT JOIN LATERAL (
            SELECT p.task_id, edge.relation_type,
                   CASE WHEN jsonb_typeof(edge.context -> 'at_turn') = 'number'
                         AND (edge.context ->> 'at_turn')::numeric BETWEEN 1 AND 2147483647
                        THEN (edge.context ->> 'at_turn')::numeric::int
                   END AS fork_at_turn
            FROM (
                SELECT r.to_task_id, r.relation_type, r.context
                FROM task_relations r
                WHERE r.from_task_id = t.task_id
                  AND r.relation_type IN ('created_by', 'forked_from')
                ORDER BY r.created_at ASC
                LIMIT 1
            ) edge
            JOIN tasks p ON p.task_id = edge.to_task_id
            JOIN task_follows pf ON pf.task_id = p.task_id AND pf.user_id = ${follower}
            WHERE {access}
        ) parent ON TRUE"
    )
}
