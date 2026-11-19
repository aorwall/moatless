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

/// Every child Task of `task_id` this viewer may read, in spawn order
/// (creation time ascending, stable).
///
/// Two reads, not a join: the relation read answers every edge; the
/// conversation read's access rule makes an unreadable child absent, not
/// blanked, and an unreadable parent [`SubtasksError::NotFound`] first.
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
        .filter(|child| child.relation.is_parent() && !child.side_chat)
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
    use moatless_types::task::TaskRelationType;

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
                    side_chat: child.side_chat,
                    relation: child.relation,
                })
                .collect())
        }

        async fn parent(
            &self,
            _child_task_id: &str,
        ) -> Result<Option<crate::ports::ParentTask>, StoreError> {
            Ok(None)
        }
    }

    fn conversation(task_id: &str, created_at: &str, name: &str) -> Conversation {
        Conversation {
            task_id: task_id.to_string(),
            workspace_id: None,
            name: Some(name.to_string()),
            branch: None,
            agent_profile: AgentProfile::ClaudeCode.into(),
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
            turn_stop_unresponsive_at: None,
            turn_error: None,
            turn_error_reason: None,
            turn_error_resets_at: None,
            child_count: 0,
            sandbox_desired_state_raw: None,
            sandbox_stop_reason_raw: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            running_commands: 0,
            running_command_labels: Vec::new(),
            latest_user_message_at: None,
            latest_queued_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            parent_task_id: None,
            parent_relation: None,
            fork_at_turn: None,
            side_chat: false,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
            last_read_at: None,
            pinned_at: None,
            pin_order_key: None,
            active_order_key: None,
            auto_settle_disabled_at: None,
            pull_requests: None,
            issues: None,
            tags: Vec::new(),
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
            side_chat: false,
            relation,
        }
    }

    fn viewer() -> ScopeContext {
        ScopeContext::user("someone")
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

    #[cfg(feature = "db-tests")]
    #[sqlx::test]
    async fn side_chats_are_absent_from_the_subtasks_listing(pool: sqlx::PgPool) {
        use moatless_tasks::task::relations::TaskRelation;
        use moatless_tasks::task::test_support::{make_task_row, seed_relation, seed_task};
        let owner = "11111111-1111-1111-1111-111111111111";
        seed_task(&pool, &make_task_row("parent", owner))
            .await
            .unwrap();
        for (id, relation_type, context) in [
            ("child", TaskRelationType::CreatedBy, json!({})),
            ("fork", TaskRelationType::ForkedFrom, json!({})),
            (
                "ordinary",
                TaskRelationType::ForkedFrom,
                json!({"side_chat": false}),
            ),
            (
                "side-chat",
                TaskRelationType::ForkedFrom,
                json!({"side_chat": true}),
            ),
        ] {
            seed_task(&pool, &make_task_row(id, owner)).await.unwrap();
            seed_relation(
                &pool,
                &TaskRelation {
                    id: uuid::Uuid::new_v4().to_string(),
                    from_task_id: id.into(),
                    to_task_id: "parent".into(),
                    relation_type,
                    context,
                    created_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        }
        let graph = moatless_tasks::compose::task_graph(pool.clone());
        let state = SubtasksState {
            relations: graph.relation_recorder(),
            conversations: Arc::new(crate::dao::PgConversationDao::new(
                pool,
                graph.activities(),
                graph.transcript_reads(),
                graph.task_reader(),
            )),
        };
        let listing = list(&state, &ScopeContext::user(owner), "parent")
            .await
            .unwrap();
        let mut ids: Vec<_> = listing["subtasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["threadId"].as_str().unwrap())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["child", "fork", "ordinary"]);
    }
}
