//! `task_forks`: the cut a fork or rewound Task continues from.

use async_trait::async_trait;
use moatless_core::db::DbPool;
use sqlx::PgConnection;

use super::models::TaskFork;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait TaskForkDao: Send + Sync {
    async fn insert(&self, row: &TaskFork) -> Result<(), sqlx::Error>;

    /// The fork row for a Task, or `None` when the Task is not a fork — which
    /// is every Task but one in a hundred, so this is on the delivery path's
    /// hot side and reads by primary key.
    async fn find(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error>;

    /// [`Self::find`], while the Task has recorded no top-level session event
    /// since the row was written: its next turn continues from the cut.
    async fn pending(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error>;
}

/// Write the cut a rewind leaves behind, replacing whatever was there. Not
/// a pool: the row must land in the same transaction as its deletes, or a Task
/// resumes its untruncated session past the cut.
pub(crate) async fn record_rewind_anchor(
    tx: &mut PgConnection,
    row: &TaskFork,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        // `created_at` moves with the cut: "has the Task recorded past its
        // cut" (`conversation_tip`, the fork-session check) compares with it.
        r#"INSERT INTO task_forks (task_id, source_task_id, at_turn, source_event_id)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (task_id) DO UPDATE
             SET source_task_id  = EXCLUDED.source_task_id,
                 at_turn         = EXCLUDED.at_turn,
                 source_event_id = EXCLUDED.source_event_id,
                 created_at      = now()"#,
    )
    .bind(&row.task_id)
    .bind(&row.source_task_id)
    .bind(row.at_turn)
    .bind(row.source_event_id)
    .execute(tx)
    .await?;
    Ok(())
}

/// Drop a Task's cut, for a rewind leaving no session to branch from. Same
/// transaction rule as [`record_rewind_anchor`]: a stale row would cut the next
/// turn into a transcript this Task lacks.
pub(crate) async fn clear_anchors(tx: &mut PgConnection, task_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM task_forks WHERE task_id = $1")
        .bind(task_id)
        .execute(tx)
        .await?;
    Ok(())
}

pub(crate) struct PgTaskForkDao(pub(crate) DbPool);

#[async_trait]
impl TaskForkDao for PgTaskForkDao {
    async fn insert(&self, row: &TaskFork) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO task_forks (task_id, source_task_id, at_turn, source_event_id)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (task_id) DO NOTHING"#,
        )
        .bind(&row.task_id)
        .bind(&row.source_task_id)
        .bind(row.at_turn)
        .bind(row.source_event_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error> {
        sqlx::query_as::<_, TaskFork>(
            r#"SELECT task_id, source_task_id, at_turn, source_event_id
               FROM task_forks WHERE task_id = $1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn pending(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error> {
        sqlx::query_as::<_, TaskFork>(
            r#"SELECT f.task_id, f.source_task_id, f.at_turn, f.source_event_id
               FROM task_forks f
              WHERE f.task_id = $1 AND NOT EXISTS (
                    SELECT 1 FROM trajectory_sessions s
                      JOIN trajectory_events e ON e.session_id = s.id
                     WHERE s.task_id = f.task_id AND s.parent_event_id IS NULL
                       AND e.created_at > f.created_at)"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }
}
