//! The `task_messages` table, as a host and the message routes reach it.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use moatless_ports::task::EventSubject;

use super::dao::MessageDao;
use super::models::{
    InterruptedTurn, MessagePage, MessageType, TaskMessage, TaskMessageContext, ToolCallRow,
};
use crate::task_activities::mapping::{ActivityRow, ConversationWrite};

/// The `task_messages` and `task_activities` tables, as the backend and this
/// crate's own handlers reach them.
///
/// The published seams are narrower: [`crate::ports::Transcript`] appends,
/// [`crate::ports::PendingWork`] answers the wake predicate, and
/// [`crate::ports::Activities`] serves the read model.
pub struct Messages {
    store: Arc<dyn MessageDao>,
}

impl Messages {
    pub(crate) fn new(store: Arc<dyn MessageDao>) -> Self {
        Self { store }
    }

    /// The store, for this crate's own services and mutations.
    pub(crate) fn store(&self) -> &dyn MessageDao {
        self.store.as_ref()
    }

    /// See [`MessageDao::get_all`].
    pub async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error> {
        self.store.get_all(task_id).await
    }

    /// See [`MessageDao::append`].
    pub async fn append(
        &self,
        msg: &TaskMessage,
        contexts: &[TaskMessageContext],
        conversation: &ConversationWrite,
    ) -> Result<i32, sqlx::Error> {
        self.store.append(msg, contexts, conversation).await
    }

    /// See [`MessageDao::append_activities`].
    pub async fn append_activities(
        &self,
        task_id: &str,
        message_uuid: &str,
        conversation: &ConversationWrite,
    ) -> Result<(), sqlx::Error> {
        self.store
            .append_activities(task_id, message_uuid, conversation)
            .await
    }

    /// See [`MessageDao::record_sandbox_activity`].
    pub async fn record_sandbox_activity(
        &self,
        activity: &ActivityRow,
    ) -> Result<bool, sqlx::Error> {
        self.store.record_sandbox_activity(activity).await
    }

    /// See [`MessageDao::record_standalone_activity`].
    pub async fn record_standalone_activity(
        &self,
        activity: &ActivityRow,
    ) -> Result<bool, sqlx::Error> {
        self.store.record_standalone_activity(activity).await
    }

    /// See [`MessageDao::get_contexts_for_messages`].
    pub async fn get_contexts_for_messages(
        &self,
        task_id: &str,
        message_ids: &[i32],
    ) -> Result<HashMap<i32, Vec<TaskMessageContext>>, sqlx::Error> {
        self.store
            .get_contexts_for_messages(task_id, message_ids)
            .await
    }

    /// See [`MessageDao::find_by_uuid`].
    pub async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TaskMessage>, sqlx::Error> {
        self.store.find_by_uuid(uuid).await
    }

    /// See [`MessageDao::get_pending_user`].
    pub async fn get_pending_user(&self, task_id: &str) -> Result<Vec<TaskMessage>, sqlx::Error> {
        self.store.get_pending_user(task_id).await
    }

    /// See [`MessageDao::get_last_result`].
    pub async fn get_last_result(&self, task_id: &str) -> Result<Option<TaskMessage>, sqlx::Error> {
        self.store.get_last_result(task_id).await
    }

    /// See [`MessageDao::get_last_assistant_message`].
    pub async fn get_last_assistant_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error> {
        self.store.get_last_assistant_message(task_id).await
    }

    /// See [`MessageDao::get_last_user_message`].
    pub async fn get_last_user_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error> {
        self.store.get_last_user_message(task_id).await
    }

    /// See [`MessageDao::backfill_last_assistant_usage`].
    #[allow(clippy::too_many_arguments)]
    pub async fn backfill_last_assistant_usage(
        &self,
        task_id: &str,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        cache_creation_input_tokens: Option<i32>,
        cache_read_input_tokens: Option<i32>,
        assistant_usage: Option<&serde_json::Value>,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .backfill_last_assistant_usage(
                task_id,
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                assistant_usage,
            )
            .await
    }

    /// See [`MessageDao::get_latest_session_id`].
    pub async fn get_latest_session_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        self.store.get_latest_session_id(task_id).await
    }

    /// See [`MessageDao::get_latest_agent_session_id`].
    pub async fn get_latest_agent_session_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        self.store.get_latest_agent_session_id(task_id).await
    }

    /// See [`MessageDao::set_session_id_for_task`].
    pub async fn set_session_id_for_task(
        &self,
        task_id: &str,
        session_id: &str,
    ) -> Result<u64, sqlx::Error> {
        self.store
            .set_session_id_for_task(task_id, session_id)
            .await
    }

    /// See [`MessageDao::acknowledge_messages`].
    pub async fn acknowledge_messages(
        &self,
        task_id: &str,
        message_uuids: &[String],
        session_id: &str,
        turn_number: i32,
    ) -> Result<u64, sqlx::Error> {
        self.store
            .acknowledge_messages(task_id, message_uuids, session_id, turn_number)
            .await
    }

    /// See [`MessageDao::get_last_message_type`].
    pub async fn get_last_message_type(
        &self,
        task_id: &str,
    ) -> Result<Option<MessageType>, sqlx::Error> {
        self.store.get_last_message_type(task_id).await
    }

    /// See [`MessageDao::last_message_is_completed_reply`].
    pub async fn last_message_is_completed_reply(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .last_message_is_completed_reply(task_id, turn_number)
            .await
    }

    /// See [`MessageDao::get_latest_message_times`].
    pub async fn get_latest_message_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        self.store.get_latest_message_times(task_ids).await
    }

    /// See [`MessageDao::set_external_source_metadata`].
    pub async fn set_external_source_metadata(
        &self,
        message_uuid: &str,
        external_id: &str,
        provider: &str,
        source_name: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .set_external_source_metadata(message_uuid, external_id, provider, source_name)
            .await
    }

    /// See [`MessageDao::record_external_event`].
    pub async fn record_external_event(
        &self,
        task_id: &str,
        external_event_id: &str,
        provider: &str,
        source_name: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        self.store
            .record_external_event(task_id, external_event_id, provider, source_name)
            .await
    }

    /// See [`MessageDao::has_external_event_id`].
    pub async fn has_external_event_id(
        &self,
        task_id: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.store.has_external_event_id(task_id, external_id).await
    }

    /// See [`MessageDao::has_any_external_event_id`].
    pub async fn has_any_external_event_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error> {
        self.store
            .has_any_external_event_id(task_id, external_ids)
            .await
    }

    /// See [`MessageDao::get_external_event_ids`].
    pub async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        self.store.get_external_event_ids(task_id).await
    }

    /// See [`MessageDao::find_task_id_by_any_external_event_id`].
    pub async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error> {
        self.store
            .find_task_id_by_any_external_event_id(external_ids)
            .await
    }

    /// See [`MessageDao::find_task_id_by_any_external_event_id_on_subject`].
    pub async fn find_task_id_by_any_external_event_id_on_subject(
        &self,
        external_ids: &[String],
        subject: EventSubject<'_>,
    ) -> Result<Option<String>, sqlx::Error> {
        self.store
            .find_task_id_by_any_external_event_id_on_subject(external_ids, subject)
            .await
    }

    /// See [`MessageDao::get_paginated`].
    pub async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<MessagePage, sqlx::Error> {
        self.store.get_paginated(task_id, before_id, limit).await
    }

    /// See [`MessageDao::get_message_for_assembly`].
    pub async fn get_message_for_assembly(
        &self,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<Option<(MessagePage, String)>, sqlx::Error> {
        self.store
            .get_message_for_assembly(task_id, message_uuid)
            .await
    }

    /// See [`MessageDao::get_subagent_messages`].
    pub async fn get_subagent_messages(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<MessagePage, sqlx::Error> {
        self.store.get_subagent_messages(task_id, tool_use_id).await
    }

    /// See [`MessageDao::find_tool_call`].
    pub async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error> {
        self.store.find_tool_call(task_id, tool_use_id).await
    }

    /// See [`MessageDao::file_id_used_in_task`].
    pub async fn file_id_used_in_task(
        &self,
        task_id: &str,
        file_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.store.file_id_used_in_task(task_id, file_id).await
    }

    /// See [`MessageDao::find_interrupted_turn`].
    pub async fn find_interrupted_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, sqlx::Error> {
        self.store.find_interrupted_turn(task_id).await
    }

    /// See [`MessageDao::turn_progressed_since_last_recovery`].
    pub async fn turn_progressed_since_last_recovery(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .turn_progressed_since_last_recovery(task_id, turn_number)
            .await
    }
}
