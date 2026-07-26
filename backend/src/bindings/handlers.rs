use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::bindings::{
    CreateTaskBindingRequest, SendTaskReplyRequest, SendTaskReplyResponse, TaskBindingResponse,
};
use moatless_api_schemas::message::{
    AddMessageReactionRequest, MessageReactionResponse, ReplyToMessageRequest,
    ReplyToMessageResponse,
};

use crate::adapters::common::{
    ensure_task_access, load_adapter_connection_for_auth, load_loop_for_auth, task_binding_response,
};
use crate::adapters::error::AdapterError;
use crate::adapters::models::{OutboundReply, TaskBinding, ADAPTER_EVENT_BINDING_ORIGIN};
use crate::bindings::outbound::resolve_files;
use crate::bindings::reply::ReplyService;
use crate::extractors::AuthUser;
use crate::message::models::{
    ContentBlockType, MessageRole, MessageType, TaskMessage, TaskMessageContentBlock,
};
use crate::state::AppState;
use moatless_backend_core::scope::ScopeContext;

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/bindings",
    tag = "task-bindings",
    responses((status = 200, description = "Task bindings", body = Vec<TaskBindingResponse>))
)]
pub async fn list_task_bindings(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<TaskBindingResponse>>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access(&state, &ctx, &task_id).await?;
    let rows = state.task_bindings.find_by_task_id(&task_id).await?;
    Ok(Json(rows.iter().map(task_binding_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/bindings",
    tag = "task-bindings",
    request_body = CreateTaskBindingRequest,
    responses((status = 200, description = "Task binding created", body = TaskBindingResponse))
)]
pub async fn create_task_binding(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<CreateTaskBindingRequest>,
) -> Result<Json<TaskBindingResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let task = ensure_task_access(&state, &ctx, &task_id).await?;
    let provider = req.adapter_kind.as_str().to_string();
    let connection = load_adapter_connection_for_auth(&state, &ctx, &req.connection_id).await?;
    if connection.provider != provider {
        return Err(AdapterError::BadRequest(format!(
            "Connection {} is for provider {}, not {}",
            connection.id, connection.provider, provider
        )));
    }
    if let Some(trigger_id) = req.trigger_id.as_deref() {
        let trigger = load_loop_for_auth(&state, &ctx, trigger_id).await?;
        let crate::loops::store::LoopSourceRow::AdapterEvent(source) = trigger.source else {
            return Err(AdapterError::BadRequest(format!(
                "Trigger {trigger_id} is not an adapter-event trigger"
            )));
        };
        if source.adapter_kind != provider {
            return Err(AdapterError::BadRequest(format!(
                "Trigger {} is for adapter {}, not {}",
                trigger.trigger.id, source.adapter_kind, provider
            )));
        }
        if source.connection_id != req.connection_id {
            return Err(AdapterError::BadRequest(format!(
                "Trigger {} belongs to connection {}, not {}",
                trigger.trigger.id, source.connection_id, req.connection_id
            )));
        }
    }
    let now = Utc::now();
    let row = TaskBinding {
        id: format!("tb_{}", uuid::Uuid::new_v4()),
        task_id: task.task_id,
        provider,
        subject_kind: req.subject_kind,
        subject_key: req.subject_key,
        connection_id: req.connection_id,
        trigger_id: req.trigger_id,
        destination_config: req
            .destination_config
            .unwrap_or_else(|| serde_json::json!({})),
        metadata: req.metadata.unwrap_or_else(|| serde_json::json!({})),
        last_external_event_id: None,
        origin: req.origin.unwrap_or_else(|| "api".to_string()),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    };
    let saved = state.task_bindings.upsert(&row).await?;
    Ok(Json(task_binding_response(&saved)))
}

#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/reply",
    tag = "task-bindings",
    request_body = SendTaskReplyRequest,
    responses((status = 200, description = "Reply sent", body = SendTaskReplyResponse))
)]
pub async fn send_task_reply(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<SendTaskReplyRequest>,
) -> Result<Json<SendTaskReplyResponse>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access(&state, &ctx, &task_id).await?;
    let response = ReplyService::new(&state)
        .send_reply(
            &ctx,
            &task_id,
            req.binding_id.as_deref(),
            &req.text,
            &req.file_ids,
        )
        .await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/messages/{message_id}/reactions",
    tag = "task-bindings",
    request_body = AddMessageReactionRequest,
    responses(
        (status = 200, description = "Reaction added", body = MessageReactionResponse),
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn add_message_reaction(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(message_id): Path<String>,
    Json(req): Json<AddMessageReactionRequest>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (binding, ext_id, provider_key) = resolve_message_target(&state, &ctx, &message_id).await?;
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("No adapters configured".into()))?;
    let adapter = providers
        .get_by_name(&provider_key)
        .ok_or_else(|| AdapterError::BadRequest(format!("Unknown adapter `{provider_key}`")))?;
    let binding_ctx = crate::bindings::runtime::binding_context(&binding);
    let external_id = adapter
        .add_reaction(&binding_ctx, &ext_id, &req.emoji)
        .await?;
    Ok(Json(MessageReactionResponse {
        ok: true,
        external_id,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/messages/{message_id}/reactions/{emoji}",
    tag = "task-bindings",
    responses(
        (status = 200, description = "Reaction removed", body = MessageReactionResponse),
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn remove_message_reaction(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((message_id, emoji)): Path<(String, String)>,
) -> Result<Json<MessageReactionResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (binding, ext_id, provider_key) = resolve_message_target(&state, &ctx, &message_id).await?;
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("No adapters configured".into()))?;
    let adapter = providers
        .get_by_name(&provider_key)
        .ok_or_else(|| AdapterError::BadRequest(format!("Unknown adapter `{provider_key}`")))?;
    let binding_ctx = crate::bindings::runtime::binding_context(&binding);
    adapter
        .remove_reaction(&binding_ctx, &ext_id, &emoji)
        .await?;
    Ok(Json(MessageReactionResponse {
        ok: true,
        external_id: None,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/messages/{message_id}/reply",
    tag = "task-bindings",
    request_body = ReplyToMessageRequest,
    responses(
        (status = 200, description = "Reply sent", body = ReplyToMessageResponse),
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn reply_to_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(message_id): Path<String>,
    Json(req): Json<ReplyToMessageRequest>,
) -> Result<Json<ReplyToMessageResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let msg = state
        .messages
        .find_by_uuid(&message_id)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("message {message_id}")))?;
    let task = ensure_task_access(&state, &ctx, &msg.task_id).await?;

    // Internal-only branch: the message wasn't sourced from an external
    // adapter, so there's no provider thread to reply to. Persist the reply
    // as a normal assistant turn on the same task and call it a day.
    let Some(ext_id) = msg.external_event_id.clone() else {
        append_internal_assistant_reply(&state, &msg.task_id, &req.text).await?;
        return Ok(Json(ReplyToMessageResponse {
            ok: true,
            external_id: None,
        }));
    };
    let provider_key = msg
        .external_adapter
        .clone()
        .ok_or_else(|| AdapterError::BadRequest("Message has no external_adapter".into()))?;
    let binding = load_binding_for_message_and_adapter(
        &state,
        &msg.task_id,
        &provider_key,
        msg.external_subject_key.as_deref(),
    )
    .await?;

    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("No adapters configured".into()))?;
    let adapter = providers
        .get_by_name(&provider_key)
        .ok_or_else(|| AdapterError::BadRequest(format!("Unknown adapter `{provider_key}`")))?;

    // reason: delivery routing for an already read-authorized reply; public
    // tasks accept replies from non-owners, so the owner's connection row is
    // resolved with system context.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let connection = state
        .adapter_connections
        .find_by_id(&binding.connection_id, &sys_ctx)
        .await?
        .ok_or_else(|| {
            AdapterError::ConfigNotFound(format!(
                "Provider connection {} not found",
                binding.connection_id
            ))
        })?;
    let rule = if let Some(ref rule_id) = binding.trigger_id {
        state.loops.find_subscription_by_id(rule_id).await?
    } else {
        None
    };

    let files = resolve_files(&state, &req.file_ids).await;
    if !files.is_empty() && !adapter.supports_files() {
        return Err(AdapterError::BadRequest(format!(
            "Provider {:?} does not support file attachments",
            adapter.adapter_kind()
        )));
    }

    let footer = crate::bindings::outbound::maybe_task_comment_footer(
        &state,
        adapter.adapter_kind(),
        &binding.task_id,
    )
    .await;

    let outbound = OutboundReply {
        text: req.text,
        task_url: None,
        footer,
        author_user_id: task.owner_user_id,
        is_error: false,
        files,
    };
    let external_id = adapter
        .reply_to_message(&binding, &connection, rule.as_ref(), &ext_id, &outbound)
        .await?;

    Ok(Json(ReplyToMessageResponse {
        ok: true,
        external_id,
    }))
}

/// Persist a synthetic assistant reply into the task timeline when a message
/// has no `external_event_id`. The reply shows up like any other agent turn —
/// no adapter call, no event publishing beyond what `messages.append` already
/// emits.
async fn append_internal_assistant_reply(
    state: &AppState,
    task_id: &str,
    text: &str,
) -> Result<(), AdapterError> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let uuid = uuid::Uuid::new_v4().to_string();
    let turn_number = state.task_turns.latest_turn_number(task_id).await?;
    let content = serde_json::json!([{ "type": "text", "text": text }]);
    let mut msg = TaskMessage::new_user_message(task_id.to_string(), uuid, content, now);
    msg.message_type = MessageType::Assistant;
    msg.message_role = MessageRole::Assistant;
    msg.user_type = "assistant".to_string();
    msg.turn_number = turn_number;

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

    state.messages.append(&msg, &[], &[block]).await?;
    Ok(())
}

/// Resolve a message id into the adapter context needed for binding-scoped
/// calls (reactions, replies): the task binding, the message's external id,
/// and the provider key.
async fn resolve_message_target(
    state: &AppState,
    ctx: &ScopeContext,
    message_id: &str,
) -> Result<(TaskBinding, String, String), AdapterError> {
    let msg = state
        .messages
        .find_by_uuid(message_id)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("message {message_id}")))?;
    ensure_task_access(state, ctx, &msg.task_id).await?;
    let ext_id = msg
        .external_event_id
        .clone()
        .ok_or_else(|| AdapterError::BadRequest("Message has no external_event_id".into()))?;
    let provider_key = msg
        .external_adapter
        .clone()
        .ok_or_else(|| AdapterError::BadRequest("Message has no external_adapter".into()))?;
    let binding = load_binding_for_message_and_adapter(
        state,
        &msg.task_id,
        &provider_key,
        msg.external_subject_key.as_deref(),
    )
    .await?;
    Ok((binding, ext_id, provider_key))
}

/// Look up a `TaskBinding` matching `(task_id, adapter_key)`. The adapter key
/// arrives via `task_messages.external_adapter`, which carries the adapter's
/// `provider_key` (e.g. `"github_pr"`). Bindings are written with whichever
/// of `provider_key` / `connection_provider_key` the inbound runtime chose
/// (GitHub PR uses `"github"` on bindings; all others use a single shared
/// name). Resolve through the adapter registry and match against either
/// key so callers can pass the message column verbatim.
async fn load_binding_for_message_and_adapter(
    state: &AppState,
    task_id: &str,
    adapter_key: &str,
    subject_key: Option<&str>,
) -> Result<TaskBinding, AdapterError> {
    let mut candidates: Vec<String> = vec![adapter_key.to_string()];
    if let Some(providers) = state.adapters.as_ref() {
        if let Some(provider) = providers.get_by_name(adapter_key) {
            let provider_key = provider.provider_key().to_string();
            let connection_key = provider.connection_provider_key().to_string();
            if !candidates.contains(&provider_key) {
                candidates.push(provider_key);
            }
            if !candidates.contains(&connection_key) {
                candidates.push(connection_key);
            }
        }
    }

    let rows = state.task_bindings.find_by_task_id(task_id).await?;

    select_binding_for_reply(&rows, &candidates, subject_key)
        .cloned()
        .ok_or_else(|| {
            AdapterError::NotFound(format!("No {adapter_key} binding for task {task_id}"))
        })
}

/// Pick which of a task's bindings a reply should target.
///
/// A single ongoing task accumulates many bindings (one per thread). When the
/// message carries its thread's `subject_key`, select that exact binding so the
/// reply threads correctly. When the subject_key is absent (legacy rows with a
/// NULL `external_subject_key`) or matches no binding, prefer the unique
/// `ingress` binding — the thread that spawned the task — over an arbitrary
/// first-match across the task's agent-owned bindings. Only then degrade to the
/// first provider match.
fn select_binding_for_reply<'a>(
    rows: &'a [TaskBinding],
    candidates: &[String],
    subject_key: Option<&str>,
) -> Option<&'a TaskBinding> {
    if let Some(sk) = subject_key {
        if let Some(binding) = rows
            .iter()
            .find(|b| candidates.contains(&b.provider) && b.subject_key == sk)
        {
            return Some(binding);
        }
    }

    let provider_rows: Vec<&TaskBinding> = rows
        .iter()
        .filter(|b| candidates.contains(&b.provider))
        .collect();

    // A single spawning (ingress) thread is the safe default when we can't
    // resolve the exact thread; ambiguity across multiple ingress bindings
    // falls through to first-match rather than guessing.
    let ingress: Vec<&TaskBinding> = provider_rows
        .iter()
        .copied()
        .filter(|b| b.origin == ADAPTER_EVENT_BINDING_ORIGIN)
        .collect();
    if ingress.len() == 1 {
        return Some(ingress[0]);
    }

    provider_rows.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(id: &str, provider: &str, subject_key: &str) -> TaskBinding {
        binding_origin(id, provider, subject_key, "test")
    }

    fn binding_origin(id: &str, provider: &str, subject_key: &str, origin: &str) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: id.to_string(),
            task_id: "task_1".to_string(),
            provider: provider.to_string(),
            subject_kind: "slack_thread".to_string(),
            subject_key: subject_key.to_string(),
            connection_id: "conn_1".to_string(),
            trigger_id: None,
            destination_config: serde_json::json!({}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: origin.to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        }
    }

    #[test]
    fn subject_key_selects_the_exact_thread_binding() {
        // Two slack_thread bindings on one ongoing task. A reply carrying
        // thread B's subject_key must select binding B, not the first row.
        let rows = vec![
            binding("bind_a", "slack", "T06:C0A:thread_a"),
            binding("bind_b", "slack", "T06:C0A:thread_b"),
        ];
        let candidates = vec!["slack".to_string()];

        let selected = select_binding_for_reply(&rows, &candidates, Some("T06:C0A:thread_b"))
            .expect("binding B should be selected");
        assert_eq!(selected.id, "bind_b");
    }

    #[test]
    fn missing_subject_key_falls_back_to_first_provider_match() {
        // Legacy rows (NULL external_subject_key) carry no subject_key — the
        // selection degenerates to the previous first-match behaviour.
        let rows = vec![
            binding("bind_a", "slack", "T06:C0A:thread_a"),
            binding("bind_b", "slack", "T06:C0A:thread_b"),
        ];
        let candidates = vec!["slack".to_string()];

        let selected = select_binding_for_reply(&rows, &candidates, None)
            .expect("first slack binding should be selected");
        assert_eq!(selected.id, "bind_a");
    }

    #[test]
    fn unknown_subject_key_falls_back_to_first_provider_match() {
        // A subject_key that matches no binding (stale/foreign) falls back to
        // the first provider match rather than failing the reply.
        let rows = vec![
            binding("bind_a", "slack", "T06:C0A:thread_a"),
            binding("bind_b", "slack", "T06:C0A:thread_b"),
        ];
        let candidates = vec!["slack".to_string()];

        let selected = select_binding_for_reply(&rows, &candidates, Some("T06:C0A:thread_unknown"))
            .expect("fallback to first slack binding");
        assert_eq!(selected.id, "bind_a");
    }

    #[test]
    fn no_subject_key_prefers_unique_ingress_binding() {
        // The task spawned from thread_a (ingress) and later opened thread_b
        // itself. A reply with no resolvable subject_key must thread back to
        // the spawning ingress binding, not the first agent-owned row.
        let rows = vec![
            binding_origin("bind_owned", "slack", "T06:C0A:thread_b", "task_owned"),
            binding_origin(
                "bind_ingress",
                "slack",
                "T06:C0A:thread_a",
                ADAPTER_EVENT_BINDING_ORIGIN,
            ),
        ];
        let candidates = vec!["slack".to_string()];

        let selected = select_binding_for_reply(&rows, &candidates, None)
            .expect("ingress binding should be selected");
        assert_eq!(selected.id, "bind_ingress");
    }

    #[test]
    fn multiple_ingress_bindings_fall_back_to_first_match() {
        // Ambiguous: two ingress bindings and no subject_key — don't guess
        // between them, degrade to first provider match.
        let rows = vec![
            binding_origin(
                "bind_a",
                "slack",
                "T06:C0A:thread_a",
                ADAPTER_EVENT_BINDING_ORIGIN,
            ),
            binding_origin(
                "bind_b",
                "slack",
                "T06:C0A:thread_b",
                ADAPTER_EVENT_BINDING_ORIGIN,
            ),
        ];
        let candidates = vec!["slack".to_string()];

        let selected = select_binding_for_reply(&rows, &candidates, None)
            .expect("first slack binding should be selected");
        assert_eq!(selected.id, "bind_a");
    }
}
