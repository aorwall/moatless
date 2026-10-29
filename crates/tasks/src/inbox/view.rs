//! An item seen as the user Message the Projector writes for it, which is the
//! shape the enricher builds a delivery from.

use std::collections::HashMap;

use crate::message::models::{TaskMessage, TaskMessageContext};
use moatless_types::agent::AgentMode;
use moatless_types::message::TOOL_RESPONSE_USER_TYPE;

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

/// Who a turn delivering `message_uuids` drives the Sandbox for: the last
/// person among them who wrote in Moatless itself, `None` for prompts nobody
/// wrote.
///
/// Adapter writers are skipped deliberately: their `sent_by` can be the
/// binding's subscriber, so driving for it would act for a bystander.
pub fn turn_driver<'a>(pending: &'a [TaskMessage], message_uuids: &[String]) -> Option<&'a str> {
    pending
        .iter()
        .filter(|message| message_uuids.contains(&message.uuid))
        .filter(|message| {
            message.source_task_id.is_none()
                && message.external_event_id.is_none()
                && !message.sandbox_wake
        })
        .filter_map(|message| message.sent_by.as_deref())
        .next_back()
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
            agent_mode: options.agent_mode.clone().map(AgentMode::from),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn message(uuid: &str, sent_by: Option<&str>) -> TaskMessage {
        TaskMessage {
            sent_by: sent_by.map(str::to_string),
            ..TaskMessage::new_user_message(
                "task".to_string(),
                uuid.to_string(),
                "hi".to_string(),
                String::new(),
            )
        }
    }

    #[test]
    fn the_last_person_in_the_turn_drives_it() {
        let pending = [
            message("1", Some("alice")),
            message("2", Some("bob")),
            message("3", Some("carol")),
        ];
        let turn = ["1".to_string(), "2".to_string()];

        assert_eq!(turn_driver(&pending, &turn), Some("bob"));
    }

    #[test]
    fn a_task_an_adapter_or_the_sandbox_drives_nothing() {
        let mut from_task = message("1", Some("alice"));
        from_task.source_task_id = Some("parent".to_string());
        let mut from_adapter = message("2", Some("subscriber"));
        from_adapter.external_event_id = Some("slack-1".to_string());
        let mut wake = message("3", None);
        wake.sandbox_wake = true;
        let system = message("4", None);
        let pending = [from_task, from_adapter, wake, system];
        let turn: Vec<String> = ["1", "2", "3", "4"].map(str::to_string).into();

        assert_eq!(turn_driver(&pending, &turn), None);
    }
}
