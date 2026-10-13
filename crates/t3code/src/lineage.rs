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
