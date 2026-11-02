//! Durable fork context, consumed with the parent's next inbox insert.

use crate::inbox::InboxContext;
use moatless_core::db::DbPool;
use sqlx::{Postgres, Transaction};

pub(crate) struct MergeBackDao(pub(crate) DbPool);

impl MergeBackDao {
    pub(crate) async fn queue(
        &self,
        source: &str,
        target: &str,
        through: i32,
        summary: &str,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM tasks WHERE task_id = $1 FOR UPDATE")
                .bind(target)
                .fetch_optional(&mut *tx)
                .await?;
        if status.as_deref() != Some("open") {
            return Ok(false);
        }
        let previous: Option<i32> = sqlx::query_scalar("SELECT through_turn FROM task_merge_backs WHERE target_task_id = $1 AND source_task_id = $2")
            .bind(target).bind(source).fetch_optional(&mut *tx).await?;
        if previous.is_some_and(|previous| previous >= through) {
            return Ok(true);
        }
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_turns WHERE task_id = $1 AND status IN ('active', 'interrupted')) OR EXISTS(SELECT 1 FROM task_inbox WHERE task_id = $1 AND status = 'pending')")
            .bind(target).fetch_one(&mut *tx).await?;
        if busy {
            return Ok(false);
        }
        sqlx::query("INSERT INTO task_merge_backs (source_task_id, target_task_id, through_turn, summary) VALUES ($1, $2, $3, $4) ON CONFLICT (target_task_id, source_task_id) DO UPDATE SET through_turn = EXCLUDED.through_turn, summary = EXCLUDED.summary, pending = true WHERE task_merge_backs.through_turn < EXCLUDED.through_turn")
            .bind(source).bind(target).bind(through).bind(summary).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
}

/// The caller holds the target Task row lock until its inbox insert commits.
pub(crate) async fn take(
    tx: &mut Transaction<'_, Postgres>,
    target: &str,
) -> Result<Vec<InboxContext>, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as("UPDATE task_merge_backs SET pending = false WHERE target_task_id = $1 AND pending RETURNING source_task_id, summary")
        .bind(target).fetch_all(&mut **tx).await?;
    Ok(rows
        .into_iter()
        .map(|(source, summary)| InboxContext {
            context_type: "mergeBack".into(),
            reference_id: source,
            data: serde_json::json!({"text": summary}),
        })
        .collect())
}
