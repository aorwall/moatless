//! The conversation, as a host and the message routes reach it.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use moatless_ports::task::EventSubject;

use super::dao::MessageDao;
use super::models::{InterruptedTurn, MessagePage, TaskMessage, ToolCallRow};
use crate::conversation::projector::Projector;
use crate::conversation::read_dao::ResponseUsage;
use crate::inbox::view::PendingView;
use crate::inbox::{InboxService, PENDING_LIMIT, RunStamp};

/// The conversation read model, as the backend and this crate's own handlers
/// reach it.
///
/// The published seams are narrower: [`crate::ports::Transcript`] appends,
/// [`crate::ports::PendingWork`] answers the wake predicate, and
/// [`crate::ports::Activities`] serves the read model.
pub struct Messages {
    store: Arc<dyn MessageDao>,
    inbox: InboxService,
}

impl Messages {
    pub(crate) fn new(store: Arc<dyn MessageDao>, inbox: InboxService) -> Self {
        Self { store, inbox }
    }

    /// The inbox pending work is read from.
    pub(crate) fn inbox(&self) -> &InboxService {
        &self.inbox
    }

    /// The store, for this crate's own services and mutations.
    pub(crate) fn store(&self) -> &dyn MessageDao {
        self.store.as_ref()
    }

    /// See [`MessageDao::get_all`].
    pub async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error> {
        self.store.get_all(task_id).await
    }

    /// See [`MessageDao::response_usage`].
    pub(crate) async fn response_usage(
        &self,
        task_id: &str,
    ) -> Result<Vec<ResponseUsage>, sqlx::Error> {
        self.store.response_usage(task_id).await
    }

    /// See [`MessageDao::find_by_uuid`].
    pub async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TaskMessage>, sqlx::Error> {
        self.store.find_by_uuid(uuid).await
    }

    /// The Task's pending inbox items, oldest first and at most
    /// [`PENDING_LIMIT`], as the Messages the enricher reads.
    pub async fn pending_user(&self, task_id: &str) -> Result<PendingView, sqlx::Error> {
        self.pending_deliverable(task_id, false).await
    }

    /// Pending items as Messages, less every `queue` item when `hold_queued`:
    /// those stay pending until a fetch made between turns takes them.
    pub async fn pending_deliverable(
        &self,
        task_id: &str,
        hold_queued: bool,
    ) -> Result<PendingView, sqlx::Error> {
        let items = self
            .inbox
            .pending(task_id, PENDING_LIMIT, hold_queued)
            .await?;
        Ok(PendingView::of(&items))
    }

    /// See [`MessageDao::latest_agent_session_id`].
    pub async fn latest_agent_session_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        self.store.latest_agent_session_id(task_id).await
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

    /// Marks the items `message_uuids` names delivered to `run`, and stamps
    /// their Messages, in one transaction. Ids that are not pending are
    /// skipped, so a repeated acknowledgement changes nothing.
    pub async fn acknowledge_messages(
        &self,
        task_id: &str,
        message_uuids: &[String],
        run: &RunStamp,
    ) -> Result<u64, sqlx::Error> {
        let ids: Vec<uuid::Uuid> = message_uuids
            .iter()
            .filter_map(|uuid| uuid.parse().ok())
            .collect();
        let mut tx = self.inbox.begin().await?;
        let marked = self
            .inbox
            .mark_delivered(&mut tx, task_id, &ids, run)
            .await?;
        Projector.on_delivered(&mut tx, task_id, &ids, run).await?;
        tx.commit().await?;
        Ok(marked)
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

    /// See [`MessageDao::file_on_any_message`].
    pub async fn file_on_any_message(&self, file_id: &str) -> Result<bool, sqlx::Error> {
        self.store.file_on_any_message(file_id).await
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

    /// See [`MessageDao::rebuild_conversation`].
    pub async fn rebuild_conversation(&self, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.rebuild_conversation(task_id).await
    }
}
