//! Hooks invoked after agent messages are persisted.
//!
//! The derived, UI-only `message.upsert` / `subagentProgress` assembly
//! (server-owned assembly and skeleton-message model): re-assemble the Message
//! a persisted row affects and publish it for the live Chat.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_types::event::EventType;
use moatless_types::message::MessageSubType;
use serde_json::json;
use tracing::{debug, warn};

use super::schemas::{SubagentStatus, UiMessage};
use crate::message::operations::Messages;
use moatless_core::events::EventPublisher;
use moatless_core::file_storage::Files;

// =============================================================================
// message.upsert — derived, UI-only assembled Message events (server-owned assembly and skeleton-message model)
// =============================================================================

/// Inject `subagentSummary` onto every `Agent` tool-use block, from one batched
/// `GROUP BY parent_uuid` aggregate.
///
/// Shared by the read path and the live top-level emit for one source of truth;
/// fire-and-forget, so a failed aggregate just leaves summaries absent.
pub(crate) async fn hydrate_subagent_summaries_from_dao(
    message_dao: &Messages,
    task_id: &str,
    messages: &mut [UiMessage],
) {
    let ids = super::transform::agent_tool_use_ids(messages);
    if ids.is_empty() {
        return;
    }
    let aggregates = match message_dao
        .store()
        .get_subagent_aggregates(task_id, &ids)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to load subagent aggregates");
            return;
        }
    };
    let map: HashMap<String, (u32, u32)> = aggregates
        .into_iter()
        .map(|a| {
            (
                a.parent_uuid,
                (
                    u32::try_from(a.message_count).unwrap_or(0),
                    u32::try_from(a.tokens).unwrap_or(0),
                ),
            )
        })
        .collect();
    super::transform::inject_subagent_summaries(messages, &map);
}

/// Re-assemble the top-level Message affected by a change to `message_uuid`
/// and publish a derived `message.upsert` carrying it.
///
/// Resolves a folded `tool_result`/system row to its parent assistant
/// Message; UI-only, since `task_events` consumers gate `message.*` out.
/// Fire-and-forget — failures are logged; the read path hydrates on next open.
pub async fn emit_message_upsert_from_daos(
    events: Option<&Arc<dyn EventPublisher>>,
    message_dao: &Messages,
    files: Option<&Arc<Files>>,
    task_id: &str,
    message_uuid: &str,
) {
    let Some(events) = events else {
        return;
    };

    let loaded = match message_dao
        .get_message_for_assembly(task_id, message_uuid)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to load rows for message.upsert assembly");
            return;
        }
    };
    let Some(((messages, contexts, blocks), target_uuid)) = loaded else {
        // The changed row folds into a parent not yet resolvable, is a subagent
        // row (handled by `subagentProgress`), or is unknown. All are covered by
        // the read-path hydration.
        debug!(task_id = %task_id, uuid = %message_uuid, "no affected Message resolved; skipping upsert");
        return;
    };

    // Legacy file-context fallback, mirroring the read path.
    let legacy_file_ids: Vec<String> = messages
        .iter()
        .flat_map(super::models::TaskMessage::file_ids)
        .collect();
    let file_metadata = if legacy_file_ids.is_empty() {
        HashMap::new()
    } else if let Some(files) = files {
        files
            .metadata_for(&legacy_file_ids)
            .await
            .unwrap_or_else(|e| {
                warn!(task_id = %task_id, error = %e, "Failed to batch-load file metadata for message.upsert");
                HashMap::new()
            })
    } else {
        HashMap::new()
    };

    let mut assembled =
        super::transform::transform_to_ui_with_files(&messages, &contexts, &blocks, &file_metadata);
    hydrate_subagent_summaries_from_dao(message_dao, task_id, &mut assembled).await;

    let Some(message) = assembled.into_iter().find(|m| m.uuid == target_uuid) else {
        debug!(task_id = %task_id, uuid = %target_uuid, "affected Message not found in assembly; skipping upsert");
        return;
    };

    // Slim shape matches the frontend read path (`summary: true`).
    let slim = crate::message::summary::strip_heavy_fields(message);
    let payload = match serde_json::to_value(&slim) {
        Ok(v) => v,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to serialize UiMessage for message.upsert");
            return;
        }
    };

    events
        .publish_raw_event(
            task_id,
            EventType::Message.as_str(),
            MessageSubType::Upsert.as_str(),
            None,
            None,
            payload,
        )
        .await;
}

/// Publish a derived `message`/`subagentProgress` event for one subagent.
///
/// Carries the **absolute** summary (`{ toolUseId, messageCount, tokens, status
/// }`) instead of a parent `message.upsert`, so the collapsed `Agent` card
/// stays live without streaming subagent transcript content.
pub async fn emit_subagent_progress(
    events: Option<&Arc<dyn EventPublisher>>,
    messages: &Messages,
    task_id: &str,
    tool_use_id: &str,
) {
    let Some(events) = events else {
        return;
    };

    let aggregates = match messages
        .store()
        .get_subagent_aggregates(task_id, std::slice::from_ref(&tool_use_id.to_string()))
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id = %task_id, tool_use_id, error = %e, "Failed to load subagent aggregate for progress");
            return;
        }
    };
    let (message_count, tokens) = aggregates.first().map_or((0, 0), |a| {
        (
            u32::try_from(a.message_count).unwrap_or(0),
            u32::try_from(a.tokens).unwrap_or(0),
        )
    });

    // Status: the one shared rule keyed on the outer `Agent` tool_result, so the
    // live path can't drift from the read path's `inject_subagent_summaries`.
    let status = match messages.find_tool_call(task_id, tool_use_id).await {
        Ok(Some(call)) => {
            let present = call.result_content.is_some() || call.is_error.is_some();
            SubagentStatus::from_outer_result(present, call.is_error == Some(true))
        }
        _ => SubagentStatus::Running,
    };

    let payload = json!({
        "toolUseId": tool_use_id,
        "messageCount": message_count,
        "tokens": tokens,
        "status": status.as_str(),
    });

    events
        .publish_raw_event(
            task_id,
            EventType::Message.as_str(),
            MessageSubType::SubagentProgress.as_str(),
            None,
            None,
            payload,
        )
        .await;
}
