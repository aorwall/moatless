use moatless_api_schemas::bindings::SendTaskReplyResponse;
use moatless_api_schemas::message::ReplyToMessageResponse;

use crate::adapters::common::{
    binding_descriptor, ensure_task_access, load_adapter_connection_for_auth,
};
use crate::adapters::error::AdapterError;
use crate::adapters::models::{OutboundReply, TaskBinding, ADAPTER_EVENT_BINDING_ORIGIN};
use crate::bindings::dao::PgTaskBindingDao;
use crate::bindings::outbound;
use crate::message::models::{MessageRole, MessageType, TaskMessage};
use crate::state::AppState;
use crate::task_activities::mapping::ConversationWrite;
use moatless_backend_core::scope::ScopeContext;

pub struct ExternalReplyService<'a> {
    state: &'a AppState,
}

impl<'a> ExternalReplyService<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    pub async fn send_task_reply(
        &self,
        ctx: &ScopeContext,
        task_id: &str,
        binding_id: Option<&str>,
        text: &str,
        file_ids: &[String],
    ) -> Result<SendTaskReplyResponse, AdapterError> {
        let providers =
            self.state.adapters.as_ref().ok_or_else(|| {
                AdapterError::AdapterNotConfigured("No adapters configured".into())
            })?;

        let repo = PgTaskBindingDao(self.state.db.clone());
        let bindings = if let Some(binding_id) = binding_id {
            let binding = repo
                .find_by_id(binding_id)
                .await?
                .ok_or_else(|| AdapterError::NotFound(format!("Binding {binding_id} not found")))?;
            if binding.task_id != task_id {
                return Err(AdapterError::BadRequest(format!(
                    "Binding {binding_id} does not belong to task {task_id}"
                )));
            }
            load_adapter_connection_for_auth(self.state, ctx, &binding.connection_id).await?;
            vec![binding]
        } else {
            repo.find_by_task_id(task_id).await?
        };

        match bindings.len() {
            0 => Err(AdapterError::ConfigNotFound(format!(
                "Task {task_id} has no bindings"
            ))),
            1 => {
                let binding = &bindings[0];
                let (sent_count, errors) = outbound::send_to_binding(
                    self.state,
                    providers,
                    task_id,
                    &binding.id,
                    text,
                    file_ids,
                )
                .await?;
                if sent_count == 0 && !errors.is_empty() {
                    return Err(AdapterError::AdapterApi(errors.join("; ")));
                }
                Ok(SendTaskReplyResponse {
                    sent_count,
                    errors,
                    targets: if sent_count > 0 {
                        vec![binding_descriptor(binding)]
                    } else {
                        vec![]
                    },
                })
            }
            _ => Err(AdapterError::MultipleTargets {
                targets: bindings.iter().map(binding_descriptor).collect(),
            }),
        }
    }

    pub async fn reply_to_message(
        &self,
        ctx: &ScopeContext,
        message_id: &str,
        text: &str,
        file_ids: &[String],
    ) -> Result<ReplyToMessageResponse, AdapterError> {
        let msg = self
            .state
            .messages
            .find_by_uuid(message_id)
            .await?
            .ok_or_else(|| AdapterError::NotFound(format!("message {message_id}")))?;
        let task = ensure_task_access(self.state, ctx, &msg.task_id).await?;

        let Some(ext_id) = msg.external_event_id.clone() else {
            append_internal_assistant_reply(self.state, &msg.task_id, text).await?;
            return Ok(ReplyToMessageResponse {
                ok: true,
                external_id: None,
            });
        };

        let provider_key = msg
            .external_adapter
            .clone()
            .ok_or_else(|| AdapterError::BadRequest("Message has no external_adapter".into()))?;
        let binding = load_binding_for_message_and_adapter(
            self.state,
            &msg.task_id,
            &provider_key,
            msg.external_subject_key.as_deref(),
        )
        .await?;

        let providers =
            self.state.adapters.as_ref().ok_or_else(|| {
                AdapterError::AdapterNotConfigured("No adapters configured".into())
            })?;
        let adapter = providers.outbound_for_message_reply_by_name(&provider_key)?;

        // reason: delivery routing for an already read-authorized reply; public
        // tasks accept replies from non-owners, so the owner's connection row is
        // resolved with system context.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let connection = self
            .state
            .adapter_connections
            .find_by_id(&binding.connection_id, &sys_ctx)
            .await?
            .ok_or_else(|| {
                AdapterError::ConfigNotFound(format!(
                    "Provider connection {} not found",
                    binding.connection_id
                ))
            })?;
        let rule = if let Some(ref subscription_id) = binding.loop_id {
            self.state
                .loops
                .find_subscription_by_id(subscription_id)
                .await?
        } else {
            None
        };

        let files = outbound::resolve_files(self.state, file_ids).await;
        if !files.is_empty() && !adapter.supports_files() {
            return Err(AdapterError::BadRequest(format!(
                "Provider {:?} does not support file attachments",
                adapter.adapter_kind()
            )));
        }

        let footer = outbound::maybe_task_comment_footer(
            self.state,
            adapter.adapter_kind(),
            &binding.task_id,
        )
        .await;
        let reply = OutboundReply {
            text: outbound::truncate_text(text, adapter.max_message_length()),
            task_url: None,
            footer,
            author_user_id: task.owner_user_id,
            is_error: false,
            files,
        };

        let external_id = adapter
            .reply_to_message(&binding, &connection, rule.as_ref(), &ext_id, &reply)
            .await?;

        let stub_uuid = outbound::persist_outbound_stub_for_binding(
            self.state,
            &msg.task_id,
            &reply.text,
            &binding,
            adapter.provider_key(),
        )
        .await;

        if let Some(ref external_id) = external_id {
            self.state.recently_sent.note(&msg.task_id, external_id);
            if let Some(ref stub_uuid) = stub_uuid {
                if let Err(e) = self
                    .state
                    .messages
                    .set_external_source_metadata(
                        stub_uuid,
                        external_id,
                        adapter.provider_key(),
                        Some(binding.subject_key.as_str()),
                    )
                    .await
                {
                    tracing::warn!(
                        task_id = %msg.task_id,
                        error = %e,
                        "Failed to tag outbound message reply stub with external_event_id"
                    );
                }
            }
            if let Err(e) = PgTaskBindingDao(self.state.db.clone())
                .touch_cursor(&binding.id, external_id)
                .await
            {
                tracing::warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to advance binding cursor for message reply"
                );
            }
        }

        Ok(ReplyToMessageResponse {
            ok: true,
            external_id,
        })
    }
}

pub(crate) async fn resolve_message_target(
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

async fn append_internal_assistant_reply(
    state: &AppState,
    task_id: &str,
    text: &str,
) -> Result<(), AdapterError> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let uuid = uuid::Uuid::new_v4().to_string();
    let turn_number = state.task_turns.latest_turn_number(task_id).await?;
    let mut msg = TaskMessage::new_user_message(task_id.to_string(), uuid, text.to_string(), now);
    msg.message_type = MessageType::Assistant;
    msg.message_role = MessageRole::Assistant;
    msg.user_type = "assistant".to_string();
    msg.turn_number = turn_number;

    state
        .messages
        .append(&msg, &[], &ConversationWrite::default())
        .await?;
    Ok(())
}

async fn load_binding_for_message_and_adapter(
    state: &AppState,
    task_id: &str,
    adapter_key: &str,
    subject_key: Option<&str>,
) -> Result<TaskBinding, AdapterError> {
    let mut candidates: Vec<String> = vec![adapter_key.to_string()];
    if let Some(providers) = state.adapters.as_ref() {
        if let Some(provider) = providers.identity_by_name(adapter_key) {
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

fn select_binding_for_reply<'a>(
    rows: &'a [TaskBinding],
    candidates: &[String],
    subject_key: Option<&str>,
) -> Option<&'a TaskBinding> {
    if let Some(sk) = subject_key {
        if let Some(binding) = rows
            .iter()
            .find(|b| candidates.contains(&b.adapter_kind) && b.subject_key == sk)
        {
            return Some(binding);
        }
    }

    let provider_rows: Vec<&TaskBinding> = rows
        .iter()
        .filter(|b| candidates.contains(&b.adapter_kind))
        .collect();

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
            adapter_kind: provider.to_string(),
            subject_kind: "slack_thread".to_string(),
            subject_key: subject_key.to_string(),
            connection_id: "conn_1".to_string(),
            loop_id: None,
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
