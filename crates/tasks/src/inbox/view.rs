//! An item seen as the user Message the Projector writes for it, which is the
//! shape the enricher builds a delivery from.

use std::collections::HashMap;

use crate::message::models::{TOOL_RESPONSE_USER_TYPE, TaskMessage, TaskMessageContext};

use super::models::{InboxContext, InboxItem, InboxOrigin};

/// Pending items as Messages, oldest first, with each one's contexts keyed by
/// the Message `id` it was given here. The ids are positions, not rows.
#[derive(Debug, Default)]
pub struct PendingView {
    pub messages: Vec<TaskMessage>,
    pub contexts: HashMap<i32, Vec<TaskMessageContext>>,
}

impl PendingView {
    pub fn of(items: &[InboxItem]) -> Self {
        let mut view = Self::default();
        for (id, item) in (1..).zip(items) {
            view.contexts.insert(id, item.context_rows(id));
            view.messages.push(TaskMessage {
                id,
                ..item.as_message()
            });
        }
        view
    }
}

impl InboxItem {
    /// The user Message this item is shown as, `uuid` = the item's id.
    pub fn as_message(&self) -> TaskMessage {
        let timestamp = self
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let options = &self.options;
        let channel = self.origin.channel();
        let sender = self.origin.sender();
        let sandbox_wake = matches!(self.origin, InboxOrigin::Sandbox);
        let user_type = if sandbox_wake {
            "system"
        } else if options.tool_response.is_some() {
            // The answer is already on screen as the outcome of the call that
            // asked; rendering this carrier too would show it twice.
            TOOL_RESPONSE_USER_TYPE
        } else {
            "human"
        };
        TaskMessage {
            user_type: user_type.to_string(),
            sandbox_wake,
            attachments: (!self.file_ids.is_empty())
                .then(|| serde_json::json!({ "file_ids": &self.file_ids })),
            agent_mode_raw: options.agent_mode.clone(),
            model: options.model.clone(),
            effort: options.effort.clone(),
            skills: options.skills.clone(),
            resume: options.resume,
            start_fresh: options.start_fresh,
            tool_response: options.tool_response.clone(),
            external_event_id: channel.map(|c| c.external_message_id.clone()),
            external_adapter: channel.map(|c| c.adapter.clone()),
            external_subject_key: channel.and_then(|c| c.subject_key.clone()),
            external_source_name: channel.and_then(|c| c.source_name.clone()),
            external_author: sender.and_then(|s| s.author.clone()),
            external_sender_id: sender.and_then(|s| s.id.clone()),
            source_task_id: self.origin.source_task_id().map(str::to_string),
            sent_by: self.origin.sent_by().map(str::to_string),
            created_at: Some(self.created_at),
            ..TaskMessage::new_user_message(
                self.task_id.clone(),
                self.id.to_string(),
                self.text.clone(),
                timestamp,
            )
        }
    }

    /// The item's contexts as rows of the Message `message_id`.
    pub fn context_rows(&self, message_id: i32) -> Vec<TaskMessageContext> {
        self.options
            .contexts
            .iter()
            .map(|context| TaskMessageContext {
                id: 0,
                task_message_id: message_id,
                context_type: context.context_type.clone(),
                reference_id: context.reference_id.clone(),
                data: context.data.clone(),
                created_at: None,
            })
            .collect()
    }
}

impl From<&TaskMessageContext> for InboxContext {
    fn from(row: &TaskMessageContext) -> Self {
        Self {
            context_type: row.context_type.clone(),
            reference_id: row.reference_id.clone(),
            data: row.data.clone(),
        }
    }
}
