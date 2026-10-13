//! Message DAO — queries for task message data access.
//!
//! `mockall::automock` mirrors every trait signature below into a mock whose
//! matcher parameters trip lints on argument shapes we chose deliberately. The
//! generated item lands in this module, so this is the only scope that can
//! carry the suppression.
#![cfg_attr(
    test,
    expect(
        clippy::ref_option_ref,
        reason = "mockall turns each `Option<&T>` argument into a `&Option<&T>` matcher"
    )
)]

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{InterruptedTurn, MessagePage, TaskMessage, ToolCallRow};
use crate::activities;
use crate::conversation::assemble;
use crate::conversation::read_dao;
use moatless_core::db::DbPool;
use moatless_ports::task::{EventSubject, TranscriptEntry, TranscriptEntryKind};

/// Per-`Agent`-call subagent aggregate: how many model responses reported
/// usage, and their summed tokens, keyed by the spawning `Agent` tool-use id.
/// Feeds the `subagentSummary` on the top-level read path and live emit.
#[derive(Debug, Clone)]
pub(crate) struct SubagentAggregate {
    /// The spawning `Agent` tool-use id.
    pub parent_uuid: String,
    pub message_count: i64,
    /// Input plus output tokens across the subagent's responses.
    pub tokens: i64,
}

/// DAO trait for reading the conversation (`conversation_*`) as Messages.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait MessageDao: Send + Sync {
    async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error>;

    /// Record a backend failure as a `task.error` event and project it as an
    /// Error Message, in one transaction.
    async fn append_error_result(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
        error_text: &str,
    ) -> Result<(), sqlx::Error>;

    /// Record a reply the backend sent on a Binding as a
    /// `task.binding_reply_sent` event under `entry.uuid`, with the external
    /// event it became, and project it, in one transaction.
    async fn append_binding_reply(&self, entry: &TranscriptEntry) -> Result<(), sqlx::Error>;

    /// Whether the Task's history froze at the migration to the conversation
    /// tables (D4a).
    async fn is_frozen(&self, task_id: &str) -> Result<bool, sqlx::Error>;

    /// Project a fork's conversation from its ancestors' sources up to `cut`,
    /// in one transaction; see [`Projector::inherit`](crate::conversation::projector::Projector).
    async fn inherit_conversation(
        &self,
        fork_task_id: &str,
        source_task_id: &str,
        cut: uuid::Uuid,
    ) -> Result<(), sqlx::Error>;

    /// Delete what [`Self::inherit_conversation`] wrote, for a fork whose
    /// create failed.
    async fn discard_conversation(&self, fork_task_id: &str) -> Result<(), sqlx::Error>;

    /// Replay the Task's conversation from its sources (S9); see
    /// [`Projector::rebuild`](crate::conversation::projector::Projector).
    async fn rebuild_conversation(&self, task_id: &str) -> Result<(), sqlx::Error>;

    /// Look up a single message by its public uuid. Returns `None` when no row
    /// matches.
    async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TaskMessage>, sqlx::Error>;

    /// The agent session the newest top-level Message that names one ran in:
    /// its line's, its inbox item's, or a frozen row's.
    async fn latest_agent_session_id(&self, task_id: &str) -> Result<Option<String>, sqlx::Error>;

    /// The newest turn result or error Message.
    async fn get_last_result(&self, task_id: &str) -> Result<Option<TaskMessage>, sqlx::Error>;
    /// Most recent top-level agent Message — the agent's final reply when
    /// called from a terminal hook.
    async fn get_last_assistant_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error>;

    /// Whether `turn_number` has a final assistant reply: its latest shown
    /// Trajectory line is assistant text with no tool call — or, for a batched
    /// turn, a later turn already got one, since coalesced runs attribute it
    /// there.
    async fn last_message_is_completed_reply(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
    ) -> Result<bool, sqlx::Error>;
    async fn get_latest_message_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;

    /// Record an external event this Task produced. See
    /// [`outbound_events`](crate::task::outbound_events_dao).
    async fn record_external_event<'a>(
        &self,
        task_id: &'a str,
        external_event_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<(), sqlx::Error>;

    /// Whether any of `external_ids` is recorded for `task_id`. Used
    /// by the dedup gate to recognize both the new `<provider>:<kind>:<id>`
    /// form and any legacy bare-int form in one round-trip.
    async fn has_any_external_event_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error>;

    async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error>;

    /// Find the owning task for any of the given external IDs, matching the
    /// id alone (suits ids a provider allocates once, e.g. a comment id).
    ///
    /// For ids a second subject can repeat, use
    /// [`find_task_id_by_any_external_event_id_on_subject`](Self::find_task_id_by_any_external_event_id_on_subject).
    async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error>;

    /// Find the owning task among events carried on `subject`: `task_inbox` is
    /// scoped by its subject key, `task_outbound_events` by its source name; a
    /// row carrying neither is not a match.
    ///
    /// Used by adapter delivery to attach a binding to the authoring task while
    /// sibling bindings on the subject still receive it.
    async fn find_task_id_by_any_external_event_id_on_subject<'a>(
        &self,
        external_ids: &[String],
        subject: EventSubject<'a>,
    ) -> Result<Option<String>, sqlx::Error>;

    /// Keyset-paginated page of top-level Messages strictly older than
    /// `before_id` (a Message `id`, which is its position), or the most recent
    /// `limit` when `None`. Returned oldest-first.
    async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<MessagePage, sqlx::Error>;

    /// Load exactly the rows to re-assemble the Message for `message_uuid`,
    /// plus the upsert's target `uuid`.
    ///
    /// Returns `None` for a subagent row (rides the progress path) or a uuid
    /// with neither text nor activity.
    async fn get_message_for_assembly(
        &self,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<Option<(MessagePage, String)>, sqlx::Error>;

    async fn get_subagent_aggregates(
        &self,
        task_id: &str,
        tool_use_ids: &[String],
    ) -> Result<Vec<SubagentAggregate>, sqlx::Error>;

    /// The usage of each response the Task's own agent made, oldest first.
    async fn response_usage(
        &self,
        task_id: &str,
    ) -> Result<Vec<read_dao::ResponseUsage>, sqlx::Error>;

    /// Load a single subagent's Messages and activities, for on-demand
    /// transcript assembly. Returns empty pages when the subagent has none.
    async fn get_subagent_messages(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<MessagePage, sqlx::Error>;

    /// Look up a single tool call by its `tool_use_id` within a task. Returns
    /// the assistant's `tool_use` input plus the matching `tool_result` content
    /// (if a result has been recorded yet).
    async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error>;

    /// Whether `file_id` is attached to a Message of `task_id`'s conversation.
    /// Used by the sandbox `DownloadUpload` RPC to defense-in-depth the
    /// cross-task check.
    async fn file_id_used_in_task(&self, task_id: &str, file_id: &str)
    -> Result<bool, sqlx::Error>;

    /// Locate the most recent user-turn that was acknowledged by the sandbox,
    /// has this Task's own `task_turns` row, and has no terminal event
    /// (`completed/error/stopped`) on it or any later turn. Returns `None` when
    /// no such turn exists.
    async fn find_interrupted_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, sqlx::Error>;

    /// Whether the turn produced agent output after its most recent
    /// `agent.recovery_started`: `true` marks a legitimate re-interruption,
    /// `false` a crash-loop that never resumed.
    ///
    /// Resets the attempt counter on `true`, so only a no-progress loop
    /// exhausts the cap.
    async fn turn_progressed_since_last_recovery(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`MessageDao`].
pub(crate) struct PgMessageDao(pub(crate) DbPool);

impl PgMessageDao {
    async fn one(
        &self,
        record: Option<read_dao::MessageRecord>,
    ) -> Result<Option<TaskMessage>, sqlx::Error> {
        match record {
            Some(record) => assemble::load_one(&self.0, record).await.map(Some),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl MessageDao for PgMessageDao {
    async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error> {
        let (records, activities) = tokio::try_join!(
            read_dao::all(&self.0, task_id),
            activities::dao::for_task(&self.0, task_id),
        )?;
        let (messages, contexts) = assemble::load(&self.0, &records, None).await?;
        Ok((messages, contexts, activities))
    }

    async fn append_error_result(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
        error_text: &str,
    ) -> Result<(), sqlx::Error> {
        let event = crate::task_event::models::InsertTaskEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            event_type: moatless_types::event::EventType::Task.as_str().to_string(),
            sub_type: moatless_types::task::TaskSubType::Error
                .as_str()
                .to_string(),
            is_error: true,
            agent_profile: None,
            turn_number,
            payload: serde_json::json!({ "text": error_text }),
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        let mut tx = self.0.begin().await?;
        crate::task_event::dao::insert_in(&mut tx, &event).await?;
        crate::conversation::projector::Projector
            .on_task_event(&mut tx, &event)
            .await?;
        tx.commit().await
    }

    async fn append_binding_reply(&self, entry: &TranscriptEntry) -> Result<(), sqlx::Error> {
        let provider = entry.external.as_ref().map(|e| e.provider.as_str());
        let event = crate::task_event::models::InsertTaskEvent {
            event_id: entry.uuid.clone(),
            task_id: entry.task_id.clone(),
            event_type: moatless_types::event::EventType::Task.as_str().to_string(),
            sub_type: moatless_types::task::TaskSubType::BindingReplySent
                .as_str()
                .to_string(),
            is_error: false,
            agent_profile: None,
            turn_number: entry.turn_number,
            payload: crate::conversation::payload::BindingReply::payload(
                &entry.text,
                entry.kind == TranscriptEntryKind::AssistantReply,
                provider,
            ),
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        let mut tx = self.0.begin().await?;
        crate::task_event::dao::insert_in(&mut tx, &event).await?;
        if let Some(external) = &entry.external {
            crate::task::outbound_events_dao::record(
                &mut *tx,
                &entry.task_id,
                &external.external_id,
                &external.provider,
                external.source_name.as_deref(),
            )
            .await?;
        }
        crate::conversation::projector::Projector
            .on_task_event(&mut tx, &event)
            .await?;
        tx.commit().await
    }

    async fn is_frozen(&self, task_id: &str) -> Result<bool, sqlx::Error> {
        let mut conn = self.0.acquire().await?;
        Ok(
            crate::conversation::replay_dao::frozen_at(&mut conn, task_id)
                .await?
                .is_some(),
        )
    }

    async fn inherit_conversation(
        &self,
        fork_task_id: &str,
        source_task_id: &str,
        cut: uuid::Uuid,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        crate::conversation::projector::Projector
            .inherit(&mut tx, fork_task_id, source_task_id, cut)
            .await?;
        tx.commit().await
    }

    async fn discard_conversation(&self, fork_task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        crate::conversation::replay_dao::discard(&mut tx, fork_task_id).await?;
        tx.commit().await
    }

    async fn rebuild_conversation(&self, task_id: &str) -> Result<(), sqlx::Error> {
        crate::conversation::projector::Projector
            .rebuild(&self.0, task_id)
            .await
    }

    async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TaskMessage>, sqlx::Error> {
        let Ok(uuid) = uuid.parse() else {
            return Ok(None);
        };
        let record = read_dao::by_uuid(&self.0, uuid).await?;
        self.one(record).await
    }

    async fn latest_agent_session_id(&self, task_id: &str) -> Result<Option<String>, sqlx::Error> {
        read_dao::latest_agent_session_id(&self.0, task_id).await
    }

    async fn get_last_result(&self, task_id: &str) -> Result<Option<TaskMessage>, sqlx::Error> {
        let record =
            read_dao::latest(&self.0, task_id, None, Some(&["turn_result", "error"])).await?;
        self.one(record).await
    }

    async fn get_last_assistant_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error> {
        let record = read_dao::latest(&self.0, task_id, Some("agent"), None).await?;
        self.one(record).await
    }

    async fn last_message_is_completed_reply(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
    ) -> Result<bool, sqlx::Error> {
        crate::message::recovery_dao::last_message_is_completed_reply(&self.0, task_id, turn_number)
            .await
    }

    async fn get_latest_message_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        read_dao::latest_times(&self.0, task_ids).await
    }

    async fn record_external_event<'a>(
        &self,
        task_id: &'a str,
        external_event_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<(), sqlx::Error> {
        crate::task::outbound_events_dao::record(
            &self.0,
            task_id,
            external_event_id,
            provider,
            source_name,
        )
        .await
    }

    async fn has_any_external_event_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error> {
        crate::message::external_events_dao::has_any(&self.0, task_id, external_ids).await
    }

    async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        crate::message::external_events_dao::ids_for_task(&self.0, task_id).await
    }

    async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error> {
        crate::message::external_events_dao::task_for_any(&self.0, external_ids).await
    }

    async fn find_task_id_by_any_external_event_id_on_subject<'a>(
        &self,
        external_ids: &[String],
        subject: EventSubject<'a>,
    ) -> Result<Option<String>, sqlx::Error> {
        crate::message::external_events_dao::task_for_any_on_subject(&self.0, external_ids, subject)
            .await
    }

    async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<MessagePage, sqlx::Error> {
        let mut records = read_dao::page(&self.0, task_id, before_id.map(i64::from), limit).await?;
        if records.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        // Oldest-first for the caller.
        records.reverse();
        let first = &records[0];
        let mode = read_dao::agent_mode_at(&self.0, task_id, first.position).await?;
        // Activities by when they happened, not by the Messages on this page: a
        // tool-only turn has no Message to be named by.
        let since = Some(first.created_at);
        let (assembled, activities) = tokio::try_join!(
            assemble::load(&self.0, &records, mode),
            activities::dao::for_task_since(&self.0, task_id, since),
        )?;
        Ok((assembled.0, assembled.1, activities))
    }

    async fn get_message_for_assembly(
        &self,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<Option<(MessagePage, String)>, sqlx::Error> {
        let record = match message_uuid.parse() {
            Ok(uuid) => read_dao::by_uuid(&self.0, uuid).await?,
            Err(_) => None,
        };
        // A subagent's rows never upsert a top-level Message: its progress rides
        // `subagentProgress` instead.
        if let Some(record) = &record {
            if record.task_id != task_id || record.parent_tool_use_id.is_some() {
                return Ok(None);
            }
        }
        let named = [message_uuid.to_string()];
        let activities = activities::dao::for_messages(&self.0, task_id, &named).await?;
        // Neither text nor activity under this uuid: a change naming a Message
        // this Task does not have.
        if record.is_none() && activities.is_empty() {
            return Ok(None);
        }
        let (messages, contexts) = match record {
            Some(record) => {
                let contexts = match record.inbox_id {
                    Some(inbox) => read_dao::inbox_items(&self.0, &[inbox])
                        .await?
                        .get(&inbox)
                        .map(|item| {
                            item.context_rows(i32::try_from(record.position).unwrap_or(i32::MAX))
                        })
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                (vec![assemble::load_one(&self.0, record).await?], contexts)
            }
            None => (Vec::new(), Vec::new()),
        };
        Ok(Some((
            (messages, contexts, activities),
            message_uuid.to_string(),
        )))
    }

    async fn response_usage(
        &self,
        task_id: &str,
    ) -> Result<Vec<read_dao::ResponseUsage>, sqlx::Error> {
        read_dao::response_usage(&self.0, task_id).await
    }

    async fn get_subagent_aggregates(
        &self,
        task_id: &str,
        tool_use_ids: &[String],
    ) -> Result<Vec<SubagentAggregate>, sqlx::Error> {
        Ok(read_dao::subagent_usage(&self.0, task_id, tool_use_ids)
            .await?
            .into_iter()
            .map(|(parent_uuid, message_count, tokens)| SubagentAggregate {
                parent_uuid,
                message_count,
                tokens,
            })
            .collect())
    }

    async fn get_subagent_messages(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<MessagePage, sqlx::Error> {
        let (records, activities) = tokio::try_join!(
            read_dao::subagent(&self.0, task_id, tool_use_id),
            activities::dao::for_subagent(&self.0, task_id, tool_use_id),
        )?;
        if records.is_empty() && activities.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        let (messages, contexts) = assemble::load(&self.0, &records, None).await?;
        Ok((messages, contexts, activities))
    }

    async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error> {
        // One row, found by an index: the call and its outcome are the same row.
        let Some(activity) = activities::dao::find_tool_call(&self.0, task_id, tool_use_id).await?
        else {
            return Ok(None);
        };

        Ok(Some(ToolCallRow {
            tool_use_id: tool_use_id.to_string(),
            tool_name: activity.summary.clone(),
            tool_input: activity.tool_input().clone(),
            result_content: activity
                .result()
                .and_then(crate::activities::mapping::result_text)
                .map(std::string::ToString::to_string),
            // `None` while the call is still out: the caller distinguishes "came
            // back clean" from "has not come back", and a bare `false` would
            // report the second as the first.
            is_error: activity.is_settled().then_some(activity.is_error),
            tool_use_result: activity.typed_result().cloned(),
            parent_message_uuid: activity.message_uuid.clone().unwrap_or_default(),
        }))
    }

    async fn file_id_used_in_task(
        &self,
        task_id: &str,
        file_id: &str,
    ) -> Result<bool, sqlx::Error> {
        read_dao::file_attached(&self.0, task_id, file_id).await
    }

    async fn find_interrupted_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, sqlx::Error> {
        crate::message::recovery_dao::find_interrupted_turn(&self.0, task_id).await
    }

    async fn turn_progressed_since_last_recovery(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<bool, sqlx::Error> {
        crate::message::recovery_dao::turn_progressed_since_last_recovery(
            &self.0,
            task_id,
            turn_number,
        )
        .await
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_pending_work_db_tests.rs"]
mod dao_pending_work_db_tests;

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
