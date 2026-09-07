//! Task fork session anchors and their persistence.

use async_trait::async_trait;
use moatless_core::db::DbPool;

/// A `task_forks` row: the fork, the Task it branched from, and the three
/// anchors the delivery path hands the pod until the fork has a session of its
/// own.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskFork {
    pub task_id: String,
    pub source_task_id: String,
    pub at_turn: Option<i32>,
    pub source_session_id: String,
    pub source_message_uuid: Option<String>,
    pub source_agent_turn_id: Option<String>,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskForkDao: Send + Sync {
    async fn insert(&self, row: &TaskFork) -> Result<(), sqlx::Error>;

    /// The fork row for a Task, or `None` when the Task is not a fork — which
    /// is every Task but one in a hundred, so this is on the delivery path's
    /// hot side and reads by primary key.
    async fn find(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error>;
}

pub struct PgTaskForkDao(pub DbPool);

#[async_trait]
impl TaskForkDao for PgTaskForkDao {
    async fn insert(&self, row: &TaskFork) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO task_forks
                   (task_id, source_task_id, at_turn, source_session_id,
                    source_message_uuid, source_agent_turn_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (task_id) DO NOTHING"#,
        )
        .bind(&row.task_id)
        .bind(&row.source_task_id)
        .bind(row.at_turn)
        .bind(&row.source_session_id)
        .bind(&row.source_message_uuid)
        .bind(&row.source_agent_turn_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error> {
        sqlx::query_as::<_, TaskFork>(
            r#"SELECT task_id, source_task_id, at_turn, source_session_id,
                      source_message_uuid, source_agent_turn_id
               FROM task_forks WHERE task_id = $1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }
}
