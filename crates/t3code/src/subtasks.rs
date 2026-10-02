//! The Tasks a Task spawned, as the agents panel's second fleet.
//!
//! T3's agents surface folds out of a thread's activities: a subagent is
//! something that happened *inside* one conversation. A Task spawned by another
//! Task is not a subagent — it has its own sandbox, transcript, and status — so
//! it is listed by its own method and the panel renders the two side by side.
//!
//! Everything here is a read.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::dao::ConversationDao;
use crate::ports::TaskRelationSource;
use crate::projection;
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use moatless_types::task::TaskRelationType;

/// Everything the `subtasks.*` handlers read through.
#[derive(Clone)]
pub struct SubtasksState {
    /// The child edges. Access is not decided here — see [`list`].
    pub relations: Arc<dyn TaskRelationSource>,
    /// The rows behind those edges, and the read rule that decides which of
    /// them this viewer is shown.
    pub conversations: Arc<dyn ConversationDao>,
}

/// Why a `subtasks.*` read could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum SubtasksError {
    /// The viewer cannot read the parent Task, or it does not exist. One answer
    /// for both: a scoped read that finds nothing must not let a caller tell
    /// another tenant's Task from a missing one.
    #[error("conversation not found")]
    NotFound,
    #[error("the task's relations could not be read")]
    Relations(#[source] StoreError),
    #[error("the child tasks could not be read")]
    Database(#[source] sqlx::Error),
}

/// The two relation types that make the `from` Task a child of the `to` Task.
///
/// `related_to` is deliberately excluded: a cross-referenced Task is not work
/// this Task spawned.
fn is_parent_edge(relation: TaskRelationType) -> bool {
    matches!(
        relation,
        TaskRelationType::CreatedBy | TaskRelationType::ForkedFrom
    )
}

/// Every child Task of `task_id` this viewer may read, in spawn order (creation
/// time ascending, stable — rows update in place and never re-sort).
///
/// Two reads, not a join: the relation read answers every edge, the
/// conversation read applies the same access rule a link resolves through, so
/// an unreadable child is absent, not blanked. An unreadable parent is
/// [`SubtasksError::NotFound`] before either read runs.
pub async fn list(
    state: &SubtasksState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SubtasksError> {
    if !state
        .conversations
        .can_read_conversation(viewer, task_id)
        .await
        .map_err(SubtasksError::Database)?
    {
        return Err(SubtasksError::NotFound);
    }

    let children = state
        .relations
        .children(task_id)
        .await
        .map_err(SubtasksError::Relations)?
        .into_iter()
        .filter(|child| is_parent_edge(child.relation))
        .collect::<Vec<_>>();
    let child_ids = children
        .iter()
        .map(|child| child.task_id.clone())
        .collect::<Vec<_>>();

    let mut rows = state
        .conversations
        .readable_by_ids(viewer, &child_ids)
        .await
        .map_err(SubtasksError::Database)?;
    rows.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.task_id.cmp(&right.task_id))
    });

    let subtasks = rows
        .iter()
        .filter_map(|row| {
            let relation = children
                .iter()
                .find(|child| child.task_id == row.task_id)?
                .relation;
            Some(projection::subtask(row, relation))
        })
        .collect::<Vec<_>>();

    Ok(json!({ "subtasks": subtasks }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dao::{Conversation, MockConversationDao};
    use async_trait::async_trait;
    use moatless_types::agent::AgentProfile;

    struct StubRelations(Vec<crate::ports::ChildTask>);

    #[async_trait]
    impl TaskRelationSource for StubRelations {
        async fn children(
            &self,
            _parent_task_id: &str,
        ) -> Result<Vec<crate::ports::ChildTask>, StoreError> {
            Ok(self
                .0
                .iter()
                .map(|child| crate::ports::ChildTask {
                    task_id: child.task_id.clone(),
                    relation: child.relation,
                })
                .collect())
        }
    }

    fn conversation(task_id: &str, created_at: &str, name: &str) -> Conversation {
        Conversation {
            task_id: task_id.to_string(),
            workspace_id: None,
            name: Some(name.to_string()),
            branch: None,
            agent_profile: AgentProfile::from(AgentProfile::CLAUDE_CODE),
            created_at: created_at.to_string(),
            created_at_db: None,
            updated_at: None,
            closed_at: None,
            visibility: moatless_types::task::Visibility::Private,
            owner_name: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state_raw: None,
            sandbox_stop_reason_raw: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            running_commands: 0,
            latest_user_message_at: None,
            latest_queued_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
            pull_requests: None,
        }
    }

    fn state(
        relations: Vec<crate::ports::ChildTask>,
        conversations: MockConversationDao,
    ) -> SubtasksState {
        SubtasksState {
            relations: Arc::new(StubRelations(relations)),
            conversations: Arc::new(conversations),
        }
    }

    fn child(task_id: &str, relation: TaskRelationType) -> crate::ports::ChildTask {
        crate::ports::ChildTask {
            task_id: task_id.to_string(),
            relation,
        }
    }

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "someone".to_string(),
            is_admin: false,
        }
    }

    #[tokio::test]
    async fn unreadable_parent_is_not_found_and_reads_no_relations() {
        let mut conversations = MockConversationDao::new();
        conversations
            .expect_can_read_conversation()
            .returning(|_, _| Ok(false));
        conversations.expect_readable_by_ids().never();

        let error = list(
            &state(
                vec![child("child", TaskRelationType::CreatedBy)],
                conversations,
            ),
            &viewer(),
            "parent",
        )
        .await
        .expect_err("an unreadable parent has no subtask listing");

        assert!(matches!(error, SubtasksError::NotFound));
    }

    #[tokio::test]
    async fn lists_both_parent_edges_in_spawn_order_and_drops_peers() {
        let mut conversations = MockConversationDao::new();
        conversations
            .expect_can_read_conversation()
            .returning(|_, _| Ok(true));
        conversations.expect_readable_by_ids().returning(|_, ids| {
            // `related_to` must never reach the conversation read.
            assert_eq!(ids.len(), 2);
            assert!(!ids.iter().any(|id| id == "peer"));
            Ok(vec![
                conversation("fork", "2024-01-03T00:00:00Z", "Forked work"),
                conversation("spawned", "2024-01-02T00:00:00Z", "Spawned work"),
            ])
        });

        let listing = list(
            &state(
                vec![
                    child("spawned", TaskRelationType::CreatedBy),
                    child("peer", TaskRelationType::RelatedTo),
                    child("fork", TaskRelationType::ForkedFrom),
                ],
                conversations,
            ),
            &viewer(),
            "parent",
        )
        .await
        .expect("a readable parent lists its children");

        let subtasks = listing["subtasks"].as_array().expect("an array");
        assert_eq!(subtasks.len(), 2);
        assert_eq!(subtasks[0]["threadId"], "spawned");
        assert_eq!(subtasks[0]["relation"], "createdBy");
        assert_eq!(subtasks[1]["threadId"], "fork");
        assert_eq!(subtasks[1]["relation"], "forkedFrom");
    }

    #[tokio::test]
    async fn a_child_the_viewer_cannot_read_is_absent() {
        let mut conversations = MockConversationDao::new();
        conversations
            .expect_can_read_conversation()
            .returning(|_, _| Ok(true));
        conversations
            .expect_readable_by_ids()
            .returning(|_, _| Ok(vec![]));

        let listing = list(
            &state(
                vec![child("hidden", TaskRelationType::CreatedBy)],
                conversations,
            ),
            &viewer(),
            "parent",
        )
        .await
        .expect("a readable parent lists its children");

        assert_eq!(listing["subtasks"].as_array().expect("an array").len(), 0);
    }
}
