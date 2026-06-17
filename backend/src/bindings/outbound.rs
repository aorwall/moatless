//! Outbound adapter notifications — event-driven replies and explicit sends.

use tracing::warn;

use crate::message::models::{
    ContentBlockType, MessageRole, MessageType, TaskMessage, TaskMessageContentBlock,
};
use crate::state::AppState;

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{OutboundFile, OutboundReply};
use crate::adapters::Adapters;
use crate::bindings::dao::PgTaskBindingRepository;
use crate::bindings::metrics::record_outbound;
use crate::triggers::rule_dao::PgTaskTriggerRouteRepository;

/// Send a message directly through a task trigger.
///
/// This is the rules-based replacement for direct sends to Slack/Telegram
/// trigger configs. When `task_id` is provided, a binding is created so
/// future replies can target the external thread via `moat reply`.
pub async fn direct_send_to_rule(
    state: &AppState,
    providers: &Adapters,
    rule_id: &str,
    task_id: Option<&str>,
    text: &str,
    file_ids: &[String],
) -> Result<Option<String>, AdapterError> {
    let rule = PgTaskTriggerRouteRepository(state.db.clone())
        .find_by_id(rule_id)
        .await?
        .ok_or_else(|| AdapterError::ConfigNotFound(rule_id.to_string()))?;

    if !rule.allow_direct_send {
        return Err(AdapterError::BadRequest(format!(
            "Task trigger {rule_id} does not allow direct send"
        )));
    }
    if !rule.active {
        return Err(AdapterError::BadRequest(format!(
            "Task trigger {rule_id} is not active"
        )));
    }

    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_id(&rule.connection_id)
        .await?
        .ok_or_else(|| AdapterError::ConfigNotFound(rule.connection_id.clone()))?;

    let provider = providers
        .get_by_name(&rule.provider)
        .ok_or_else(|| AdapterError::AdapterNotConfigured(rule.provider.clone()))?;
    let provider_label = provider.provider_key();

    let files = resolve_files(state, file_ids).await;
    if !files.is_empty() && !provider.supports_files() {
        record_outbound(
            provider_label,
            "error",
            "unsupported_feature",
            &rule.id,
            &connection.id,
        );
        return Err(AdapterError::BadRequest(format!(
            "Provider {:?} does not support file attachments",
            provider.adapter_kind()
        )));
    }

    let reply = OutboundReply {
        text: truncate_text(text, provider.max_message_length()),
        task_url: None,
        footer: None,
        is_error: false,
        files,
    };
    let config = crate::bindings::runtime::trigger_context(&rule, provider_label);
    let thread_info = match provider.send_message(&config, &reply).await {
        Ok(info) => info,
        Err(e) => {
            record_outbound(
                provider_label,
                "error",
                "send_failed",
                &rule.id,
                &connection.id,
            );
            return Err(AdapterError::AdapterApi(format!(
                "Failed to send message via {}: {e}",
                rule.provider
            )));
        }
    };

    if let Some(tid) = task_id {
        let destination = provider.binding_destination_from_new_thread(&rule, &thread_info)?;

        crate::bindings::runtime::upsert_binding(
            state,
            &connection,
            Some(&rule),
            tid,
            provider.connection_provider_key(),
            destination.subject_kind,
            &destination.subject_key,
            destination.destination_config,
            crate::bindings::runtime::ORIGIN_CONNECTED_THREAD,
            destination.last_external_event_id,
        )
        .await?;
    }

    record_outbound(
        provider_label,
        "success",
        "direct_send",
        &rule.id,
        &connection.id,
    );
    Ok(Some(thread_info.external_thread_id))
}

/// Truncate text to a max length, appending "..." if truncated.
pub(crate) fn truncate_text(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}...", &text[..max.saturating_sub(3)])
    }
}

// ---------------------------------------------------------------------------
// File attachment helpers
// ---------------------------------------------------------------------------

/// Resolve file IDs to [`OutboundFile`] structs by loading metadata + data from S3.
///
/// Failures for individual files are logged as warnings and skipped.
pub(crate) async fn resolve_files(state: &AppState, file_ids: &[String]) -> Vec<OutboundFile> {
    if file_ids.is_empty() {
        return vec![];
    }

    let Some(s3_client) = state.s3.as_ref() else {
        warn!("S3 not configured, cannot resolve file attachments");
        return vec![];
    };
    let Some(bucket) = state.config.s3.bucket_name.as_ref() else {
        warn!("S3 bucket not configured, cannot resolve file attachments");
        return vec![];
    };

    let mut files = Vec::with_capacity(file_ids.len());
    for file_id in file_ids {
        let meta = match state.files.find_by_id(file_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                warn!(file_id = %file_id, "File metadata not found for outbound attachment");
                continue;
            }
            Err(e) => {
                warn!(file_id = %file_id, error = %e, "Failed to load file metadata");
                continue;
            }
        };

        let s3_key = format!("{}/{}", state.config.s3.key_prefix, file_id);
        match crate::file_storage::s3::get_object(s3_client, bucket, &s3_key).await {
            Ok((data, _content_type)) => {
                files.push(OutboundFile {
                    filename: meta.filename,
                    mime_type: meta.mime_type,
                    data,
                });
            }
            Err(e) => {
                warn!(file_id = %file_id, error = %e, "Failed to download file from S3");
            }
        }
    }

    files
}

// ---------------------------------------------------------------------------
// Binding-based outbound
// ---------------------------------------------------------------------------

/// Send a reply to the external target referenced by a task binding.
///
/// Loads the binding, its adapter connection, and its optional
/// trigger rule, then calls `send_reply_for_binding` on the matching
/// adapter. Persists a visible assistant stub up front, tags it with the
/// returned external id, populates the recently_sent race-window cache, and
/// advances the binding cursor.
pub async fn send_to_binding(
    state: &AppState,
    providers: &Adapters,
    task_id: &str,
    binding_id: &str,
    text: &str,
    file_ids: &[String],
) -> Result<(u32, Vec<String>), AdapterError> {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
    let binding = binding_repo
        .find_by_id(binding_id)
        .await?
        .ok_or_else(|| AdapterError::ConfigNotFound(format!("No task binding {binding_id}")))?;

    if binding.task_id != task_id {
        return Err(AdapterError::BadRequest(format!(
            "Binding {binding_id} does not belong to task {task_id}"
        )));
    }

    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_id(&binding.connection_id)
        .await?
        .ok_or_else(|| {
            AdapterError::ConfigNotFound(format!(
                "Provider connection {} not found",
                binding.connection_id
            ))
        })?;

    let rule = if let Some(ref rule_id) = binding.trigger_id {
        PgTaskTriggerRouteRepository(state.db.clone())
            .find_by_id(rule_id)
            .await?
    } else {
        None
    };

    let provider = providers
        .get_by_name(&binding.provider)
        .ok_or_else(|| AdapterError::AdapterNotConfigured(binding.provider.clone()))?;
    let provider_label = provider.provider_key();

    let binding_trigger_id = binding.trigger_id.as_deref().unwrap_or("");
    let files = resolve_files(state, file_ids).await;
    if !files.is_empty() && !provider.supports_files() {
        record_outbound(
            provider_label,
            "error",
            "unsupported_feature",
            binding_trigger_id,
            &binding.connection_id,
        );
        return Ok((
            0,
            vec![format!(
                "Provider {:?} does not support file attachments",
                provider.adapter_kind()
            )],
        ));
    }

    // GitHub/Linear comments end with a link back to the moatless task. Other
    // providers don't carry the footer.
    let footer = match provider.adapter_kind() {
        crate::adapters::AdapterKind::GithubPr | crate::adapters::AdapterKind::Linear => {
            crate::adapters::footer::task_comment_footer(state, task_id).await
        }
        _ => None,
    };

    let reply = OutboundReply {
        text: truncate_text(text, provider.max_message_length()),
        task_url: None,
        footer,
        is_error: false,
        files,
    };

    // Persist a visible stub up front so the row shows in the task timeline
    // and we have a uuid to tag with the outbound `external_event_id`.
    let stub_uuid =
        persist_outbound_stub_for_binding(state, task_id, &reply.text, &binding, provider_label)
            .await;

    let external_id = match provider
        .send_reply_for_binding(&binding, &connection, rule.as_ref(), &reply)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            record_outbound(
                provider_label,
                "error",
                "send_failed",
                binding_trigger_id,
                &binding.connection_id,
            );
            return Ok((
                0,
                vec![format!(
                    "Failed to send reply via {}: {e}",
                    binding.provider
                )],
            ));
        }
    };

    if let Some(ref ext_id) = external_id {
        state.recently_sent.note(task_id, ext_id);
        if let Some(ref msg_id) = stub_uuid {
            if let Err(e) = state
                .messages
                .set_external_source_metadata(
                    msg_id,
                    ext_id,
                    provider_label,
                    Some(binding.subject_key.as_str()),
                )
                .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to tag outbound stub with external_event_id");
            }
        }
        if let Err(e) = binding_repo.touch_cursor(&binding.id, ext_id).await {
            warn!(
                binding_id = %binding.id,
                error = %e,
                "Failed to advance binding cursor"
            );
        }
    }

    record_outbound(
        provider_label,
        "success",
        "",
        binding_trigger_id,
        &binding.connection_id,
    );
    Ok((1, vec![]))
}

/// Persist a visible assistant row for a binding-based outbound reply.
async fn persist_outbound_stub_for_binding(
    state: &AppState,
    task_id: &str,
    text: &str,
    binding: &crate::adapters::models::TaskBinding,
    provider_label: &str,
) -> Option<String> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let uuid = uuid::Uuid::new_v4().to_string();

    let turn_number = match state.messages.current_turn_number(task_id).await {
        Ok(t) => Some(t),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to read turn number for outbound stub");
            None
        }
    };

    let content = serde_json::json!([{ "type": "text", "text": text }]);

    let mut msg =
        TaskMessage::new_user_message(task_id.to_string(), uuid.clone(), content, now.clone());
    msg.message_type = MessageType::System;
    msg.message_role = MessageRole::Assistant;
    msg.user_type = "assistant".to_string();
    msg.turn_number = turn_number;
    msg.external_adapter = Some(provider_label.to_string());
    msg.external_source_name = Some(binding.subject_key.clone());

    let block = TaskMessageContentBlock {
        id: 0,
        task_message_id: 0,
        sort_order: 0,
        block_type: ContentBlockType::Text,
        show_in_ui: true,
        text_content: Some(text.to_string()),
        tool_use_id: None,
        tool_result_content: None,
        tool_result_is_error: None,
        image_media_type: None,
        image_data: None,
        created_at: None,
    };

    match state.messages.append(&msg, &[], &[block]).await {
        Ok(_) => Some(uuid),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to persist outbound stub message");
            None
        }
    }
}
