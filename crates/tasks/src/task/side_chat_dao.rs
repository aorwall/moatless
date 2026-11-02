//! Side-chat context frozen with its first inbound inbox item.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::agent::side_chat;
use crate::inbox::InboxContext;

/// The caller holds the Task row lock until the inbox insert commits.
pub(crate) async fn first_message_context(
    tx: &mut Transaction<'_, Postgres>,
    task_id: &str,
    inbox_id: Uuid,
) -> Result<Option<InboxContext>, sqlx::Error> {
    // Inherited inbox rows belong to the parent; only this Task's inputs count.
    let parent: Option<(String, Option<i32>)> = sqlx::query_as(
        "SELECT r.to_task_id, (r.context ->> 'at_turn')::int FROM task_relations r \
         JOIN task_inbox current ON current.id = $2 AND current.task_id = r.from_task_id \
         WHERE r.from_task_id = $1 AND r.relation_type = 'forked_from' \
           AND r.context -> 'side_chat' = 'true'::jsonb AND current.text !~ $3 \
           AND NOT EXISTS (SELECT 1 FROM task_inbox WHERE task_id = $1 AND id <> $2 \
                AND origin <> 'sandbox' AND answers_tool_use_id IS NULL AND text !~ $3)",
    )
    .bind(task_id)
    .bind(inbox_id)
    .bind("^[[:space:]]*/(compact|goal)([[:space:]]|$)")
    .fetch_optional(&mut **tx)
    .await?;
    let Some((parent_id, at_turn)) = parent else {
        return Ok(None);
    };
    let items = sqlx::query_as(
        "SELECT m.author, COALESCE(i.text, m.text) FROM conversation_messages m \
         LEFT JOIN task_inbox i ON i.id = m.inbox_id \
         WHERE m.task_id = $1 AND m.parent_tool_use_id IS NULL \
           AND COALESCE(i.text, m.text) IS NOT NULL \
           AND (COALESCE(m.turn_number, i.turn_number) > COALESCE($2, 0) \
                OR (i.status = 'pending' AND i.task_id = $1)) \
         ORDER BY m.position DESC LIMIT 2001",
    )
    .bind(&parent_id)
    .bind(at_turn)
    .fetch_all(&mut **tx)
    .await?;
    let text = side_chat::section(&parent_id, &side_chat::digest(&parent_id, &items));
    Ok(Some(InboxContext {
        context_type: "sideChat".into(),
        reference_id: parent_id,
        data: serde_json::json!({"text": text}),
    }))
}
