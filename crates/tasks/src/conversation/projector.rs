//! The Projector: turns inbox items into the user Messages a conversation
//! shows, in the transaction that wrote the item.

use sqlx::{Postgres, Transaction};

use crate::inbox::{InboxItem, InboxOrigin, RunStamp};
use crate::task_activities::mapping::{
    ActivityStatus, ConversationWrite, ToolOutcome, user_input_resolved,
};

/// The only writer of a user `task_messages` row.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Projector;

impl Projector {
    /// Writes the item's Message, `uuid` = the item's id, positioned now. A
    /// `sandbox` item is nobody's message and projects nothing.
    pub(crate) async fn on_inbox(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: &InboxItem,
    ) -> Result<(), sqlx::Error> {
        if matches!(item.origin, InboxOrigin::Sandbox) {
            return Ok(());
        }
        let message = item.as_message();
        crate::message::dao::append_in(tx, &message, &item.context_rows(0), &answer_of(item))
            .await?;
        Ok(())
    }

    /// Stamps the delivered items' Messages with their run; their position does
    /// not move.
    pub(crate) async fn on_delivered(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_id: &str,
        ids: &[uuid::Uuid],
        run: &RunStamp,
    ) -> Result<(), sqlx::Error> {
        crate::message::dao::stamp_delivered_in(tx, task_id, ids, run).await?;
        Ok(())
    }
}

/// A tool response settles the call it answers: without the outcome the
/// question keeps rendering as unanswered, and the resolved row beside it is
/// what closes the panel.
fn answer_of(item: &InboxItem) -> ConversationWrite {
    let (Some(tool_use_id), Some(response)) = (
        item.answers_tool_use_id.as_deref(),
        item.options.tool_response.as_ref(),
    ) else {
        return ConversationWrite::default();
    };
    let answer: serde_json::Map<String, serde_json::Value> = response
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !matches!(key.as_str(), "toolUseId" | "toolName"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let answer = if answer.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(answer)
    };
    let uuid = item.id.to_string();
    ConversationWrite {
        outcomes: vec![ToolOutcome {
            task_id: item.task_id.clone(),
            tool_use_id: tool_use_id.to_string(),
            status: ActivityStatus::Completed,
            result: if answer.is_null() {
                serde_json::Value::String(item.text.clone())
            } else {
                answer.clone()
            },
            typed_result: None,
            is_error: false,
        }],
        activities: vec![user_input_resolved(
            &item.task_id,
            &uuid,
            tool_use_id,
            &answer,
        )],
        ..Default::default()
    }
}
