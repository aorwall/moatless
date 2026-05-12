//! Shared binding runtime backed by adapter connections, triggers, rules,
//! and task bindings.

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, InboundMessage, RoutingMode, TaskBinding, TaskTriggerRoute,
    TriggerContext,
};
use crate::bindings::dao::PgTaskBindingRepository;
use crate::bindings::metrics::record_inbound;
use crate::state::AppState;
use crate::task::models::TaskStatus;
use crate::triggers::inbound::{self, CommandInfo};
use crate::triggers::rule_dao::PgTaskTriggerRouteRepository;

pub use crate::adapters::models::{
    ADAPTER_EVENT_BINDING_ORIGIN as ORIGIN_ADAPTER_EVENT,
    CONNECTED_THREAD_BINDING_ORIGIN as ORIGIN_CONNECTED_THREAD,
};

/// Build a synthetic `TriggerContext` from a trigger+route view for provider
/// trait methods that still accept a config-shaped value.
pub fn trigger_context(rule: &TaskTriggerRoute, provider_label: &str) -> TriggerContext {
    TriggerContext {
        id: rule.id.clone(),
        provider: provider_label.to_string(),
        source_id: rule.source_matcher.clone(),
        source_name: rule.source_name.clone(),
        repository_id: rule.repository_id.clone(),
        routing_mode: rule.routing_mode,
        prompt_template: rule.prompt_template.clone(),
        active: rule.active,
        allow_direct_send: rule.allow_direct_send,
        provider_config: rule.provider_config.clone(),
        ongoing_task_id: rule.ongoing_task_id.clone(),
        scope: rule.scope,
        team_id: rule.team_id.clone(),
        task_name: rule.task_name.clone(),
        tag_ids: rule.tag_ids.clone(),
        run_as_user_id: rule.run_as_user_id.clone(),
        agent_type: rule.agent_type.clone(),
        created_by: rule.created_by.clone(),
        created_at: rule.created_at,
        updated_at: rule.updated_at,
    }
}

/// Build a provider binding context from a task binding. Never persist this row.
pub fn binding_context(binding: &TaskBinding) -> BindingContext {
    let mut destination_config = binding.destination_config.clone();
    if let Some(obj) = destination_config.as_object_mut() {
        if let Some(ext_id) = binding.last_external_event_id.as_ref() {
            obj.insert("last_external_event_id".into(), serde_json::json!(ext_id));
        }
        if let Some(filter_uid) = binding.metadata.get("filter_user_id").cloned() {
            obj.insert("filter_user_id".into(), filter_uid);
        }
    }

    BindingContext {
        binding_id: binding.id.clone(),
        trigger_id: binding.trigger_id.clone(),
        task_id: binding.task_id.clone(),
        subject_kind: binding.subject_kind.clone(),
        subject_key: binding.subject_key.clone(),
        destination_config,
        metadata: binding.metadata.clone(),
        last_external_event_id: binding.last_external_event_id.clone(),
        last_message_at: binding.last_event_at,
        created_at: binding.created_at,
    }
}

pub async fn resolve_chat_provider_event(
    state: &AppState,
    provider: &str,
    scope_value: &str,
    source_id: &str,
) -> Result<Option<(AdapterConnection, TaskTriggerRoute)>, AdapterError> {
    let connection_repo = PgAdapterConnectionRepository(state.db.clone());
    // Look up exactly the requested scope_value. We deliberately do not fall
    // back to the "default" connection when scope_value is unknown — that would
    // silently route a tenant A event to tenant B's connection. Callers that
    // want the default behavior pass "default" explicitly.
    let Some(connection) = connection_repo
        .find_by_scope_value(provider, scope_value)
        .await?
    else {
        warn!(
            provider,
            scope_value, "No provider connection matched incoming event"
        );
        record_inbound(provider, "ignored", "no_connection", "", "");
        return Ok(None);
    };

    let rule_repo = PgTaskTriggerRouteRepository(state.db.clone());
    let rule = rule_repo
        .find_by_source_matcher(provider, source_id)
        .await?
        .or(rule_repo.find_wildcard(provider).await?);
    let Some(rule) = rule else {
        record_inbound(provider, "ignored", "no_rule_match", "", &connection.id);
        return Ok(None);
    };

    if rule.connection_id != connection.id {
        debug!(
            provider,
            source_id,
            rule_id = %rule.id,
            connection_id = %connection.id,
            "Task trigger belongs to a different connection"
        );
        record_inbound(
            provider,
            "ignored",
            "wrong_connection",
            &rule.id,
            &connection.id,
        );
        return Ok(None);
    }

    Ok(Some((connection, rule)))
}

#[allow(clippy::too_many_arguments)]
pub async fn route_inbound_message(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &TaskTriggerRoute,
    msg: InboundMessage,
    command: Option<CommandInfo>,
    provider_label: &str,
    subject_kind: &str,
    subject_key: &str,
    destination_config: serde_json::Value,
) -> Result<String, AdapterError> {
    if !rule.active {
        debug!(rule_id = %rule.id, "Ignoring external message: task trigger inactive");
        record_inbound(
            provider_label,
            "ignored",
            "inactive_rule",
            &rule.id,
            &connection.id,
        );
        return Err(AdapterError::ConfigNotFound(rule.id.clone()));
    }

    let binding_repo = PgTaskBindingRepository(state.db.clone());
    let origin_task_id = match msg.external_event_id.as_deref() {
        Some(ext_id) => {
            let candidates = external_id_aliases(state, provider_label, ext_id);
            state
                .messages
                .find_task_id_by_any_external_event_id(&candidates)
                .await?
        }
        None => None,
    };

    let mut bindings = binding_repo
        .find_by_subject(provider_label, subject_kind, subject_key)
        .await?;

    if let Some(origin_task_id) = origin_task_id.as_deref() {
        if !bindings.iter().any(|s| s.task_id == origin_task_id) {
            let sub = upsert_binding(
                state,
                connection,
                Some(rule),
                origin_task_id,
                provider_label,
                subject_kind,
                subject_key,
                destination_config.clone(),
                ORIGIN_CONNECTED_THREAD,
                msg.external_event_id.clone(),
            )
            .await?;
            bindings.push(sub);
        }
    }

    if rule.routing_mode == RoutingMode::Ongoing {
        if let Some(task_id) = open_ongoing_task_id(state, rule).await {
            if !bindings.iter().any(|s| s.task_id == task_id) {
                // Pass None so a newly attached binding starts with no cursor —
                // `deliver_to_binding` then fetches the full prior thread as
                // context, and `touch_cursor` advances it after delivery. For an
                // existing binding the COALESCE in the upsert preserves the
                // current cursor.
                let sub = upsert_binding(
                    state,
                    connection,
                    Some(rule),
                    &task_id,
                    provider_label,
                    subject_kind,
                    subject_key,
                    destination_config.clone(),
                    ORIGIN_ADAPTER_EVENT,
                    None,
                )
                .await?;
                bindings.push(sub);
            }
        }
    }

    let mut is_new_task = false;
    if should_create_binding_task(state, rule, &bindings).await? {
        let config = trigger_context(rule, provider_label);
        let task_id =
            inbound::create_task_for_external_message(state, &config, &msg, command.as_ref())
                .await?;
        provision_task(state, &task_id).await;
        if rule.routing_mode == RoutingMode::Ongoing {
            update_rule_ongoing_task(state, &rule.id, &task_id).await?;
        }
        // Seed the cursor as None so `fetch_context` retrieves the full prior
        // thread on the first delivery; `touch_cursor` advances it afterward.
        let sub = upsert_binding(
            state,
            connection,
            Some(rule),
            &task_id,
            provider_label,
            subject_kind,
            subject_key,
            destination_config.clone(),
            ORIGIN_ADAPTER_EVENT,
            None,
        )
        .await?;
        bindings.push(sub);
        is_new_task = true;
        info!(task_id = %task_id, rule_id = %rule.id, provider = provider_label, "Created task from provider event");
    }

    let mut delivered_task_id = None;
    let receipts = crate::triggers::receipts::PgEventReceiptRepository(state.db.clone());
    for binding in bindings {
        if should_filter_self_echo(&binding, &msg.user_id)
            || origin_task_id.as_deref() == Some(binding.task_id.as_str())
        {
            debug!(
                task_id = %binding.task_id,
                subject_key,
                provider = provider_label,
                "Skipping self-echo/originating binding"
            );
            record_inbound(
                provider_label,
                "ignored",
                "self_echo",
                &rule.id,
                &connection.id,
            );
            delivered_task_id = Some(binding.task_id.clone());
            continue;
        }

        if let Some(external_event_id) = msg.external_event_id.as_deref() {
            let is_first_delivery = receipts
                .record_once(
                    provider_label,
                    external_event_id,
                    subject_kind,
                    subject_key,
                    &binding.task_id,
                )
                .await?;
            if !is_first_delivery {
                debug!(
                    task_id = %binding.task_id,
                    external_event_id,
                    provider = provider_label,
                    "Skipping duplicate external event receipt"
                );
                record_inbound(
                    provider_label,
                    "ignored",
                    "duplicate_receipt",
                    &rule.id,
                    &connection.id,
                );
                delivered_task_id = Some(binding.task_id.clone());
                continue;
            }
        }

        let this_is_new = is_new_task && binding.origin == ORIGIN_ADAPTER_EVENT;
        match deliver_to_binding(
            state,
            rule,
            &binding,
            &msg,
            command.as_ref(),
            provider_label,
            this_is_new,
        )
        .await
        {
            Ok(()) => {
                let reason = if this_is_new { "new_task" } else { "existing_task" };
                record_inbound(provider_label, "success", reason, &rule.id, &connection.id);
            }
            Err(e) => {
                record_inbound(
                    provider_label,
                    "error",
                    "deliver_failed",
                    &rule.id,
                    &connection.id,
                );
                return Err(e);
            }
        }
        delivered_task_id = Some(binding.task_id.clone());
    }

    delivered_task_id.ok_or_else(|| AdapterError::ConfigNotFound(rule.id.clone()))
}

pub fn external_id_aliases(
    state: &AppState,
    provider_label: &str,
    external_id: &str,
) -> Vec<String> {
    state
        .adapters
        .as_ref()
        .and_then(|providers| providers.get_by_name(provider_label))
        .map_or_else(
            || vec![external_id.to_string()],
            |provider| provider.external_id_aliases(external_id),
        )
}

async fn should_create_binding_task(
    state: &AppState,
    rule: &TaskTriggerRoute,
    bindings: &[TaskBinding],
) -> Result<bool, AdapterError> {
    if rule.routing_mode != RoutingMode::Ongoing {
        return Ok(bindings.is_empty());
    }

    if !bindings.is_empty() {
        return Ok(false);
    }
    Ok(open_ongoing_task_id(state, rule).await.is_none())
}

async fn open_ongoing_task_id(state: &AppState, rule: &TaskTriggerRoute) -> Option<String> {
    let task_id = rule.ongoing_task_id.as_deref()?;
    match state.tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) if task.status == TaskStatus::Open => Some(task.task_id),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_binding(
    state: &AppState,
    connection: &AdapterConnection,
    rule: Option<&TaskTriggerRoute>,
    task_id: &str,
    provider: &str,
    subject_kind: &str,
    subject_key: &str,
    destination_config: serde_json::Value,
    origin: &str,
    last_external_event_id: Option<String>,
) -> Result<TaskBinding, AdapterError> {
    let now = Utc::now();
    let filter_uid = destination_config
        .get("filter_user_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let binding = TaskBinding {
        id: format!("ts_{}", uuid::Uuid::new_v4()),
        task_id: task_id.to_string(),
        provider: provider.to_string(),
        subject_kind: subject_kind.to_string(),
        subject_key: subject_key.to_string(),
        connection_id: connection.id.clone(),
        trigger_id: rule.map(|r| r.id.clone()),
        destination_config,
        metadata: serde_json::json!({ "filter_user_id": filter_uid }),
        last_external_event_id,
        origin: origin.to_string(),
        created_at: now,
        last_event_at: now,
    };
    Ok(PgTaskBindingRepository(state.db.clone())
        .upsert(&binding)
        .await?)
}

async fn deliver_to_binding(
    state: &AppState,
    rule: &TaskTriggerRoute,
    binding: &TaskBinding,
    msg: &InboundMessage,
    command: Option<&CommandInfo>,
    provider_label: &str,
    is_new_task: bool,
) -> Result<(), AdapterError> {
    let config = trigger_context(rule, provider_label);
    let thread = binding_context(binding);

    let thread_context = if msg.thread_id.is_some() {
        if let Some(ref providers) = state.adapters {
            if let Some(provider) = providers.get_by_name(provider_label) {
                let mut exclude_ids = state
                    .messages
                    .get_external_event_ids(&binding.task_id)
                    .await
                    .unwrap_or_default();
                if let Some(ref ext_id) = msg.external_event_id {
                    exclude_ids.push(ext_id.clone());
                }
                provider
                    .fetch_context(&thread, &config, 20, &exclude_ids)
                    .await
                    .unwrap_or_else(|e| {
                        warn!(provider = provider_label, error = %e, "Failed to fetch thread context");
                        None
                    })
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let base_text = match command {
        Some(cmd) => {
            let prefix = format!("/{}", cmd.name);
            msg.text
                .trim()
                .strip_prefix(&prefix)
                .unwrap_or(&msg.text)
                .trim()
                .to_string()
        }
        None => msg.text.clone(),
    };

    let mut message_text = base_text;
    if let Ok(Some(user)) = state.users.get_user(&msg.user_id).await {
        let display = user.name.as_deref().unwrap_or(&user.login);
        let suffix = state
            .adapters
            .as_ref()
            .and_then(|providers| providers.get_by_name(provider_label))
            .and_then(|provider| provider.sender_label_suffix(msg))
            .unwrap_or_default();
        let label = format!("{display} ({login}{suffix})", login = user.login);
        message_text = format!("{label}: {message_text}");
    }

    if let Some(prefix) = state
        .adapters
        .as_ref()
        .and_then(|providers| providers.get_by_name(provider_label))
        .and_then(|provider| provider.inbound_message_prefix(msg))
    {
        message_text = format!("{prefix}\n\n{message_text}");
    }

    if msg.thread_id.is_some() {
        if let Some(providers) = state.adapters.as_ref() {
            if let Some(provider) = providers.get_by_name(provider_label) {
                if let Some(hint) = provider.thread_reply_hint(msg) {
                    message_text = format!("{hint}\n\n{message_text}");
                }
            }
        }
    }

    if let Some(ctx) = thread_context {
        if binding.last_external_event_id.is_some() {
            message_text = format!("{ctx}\n\n---\n\n{message_text}");
        } else {
            message_text = format!(
                "Thread context (previous messages):\n\n{ctx}\n\n---\n\nNew message:\n\n{message_text}"
            );
        }
    }

    let (cmd_resume, cmd_agent_mode, cmd_name) = match command {
        Some(cmd) => (cmd.resume, cmd.agent_mode.as_ref(), Some(cmd.name.as_str())),
        None => (true, None, None),
    };

    crate::message::send::send_message_flow(
        state,
        &binding.task_id,
        &message_text,
        None,
        cmd_agent_mode,
        &msg.file_ids,
        &[],
        cmd_resume,
        msg.external_event_id.as_deref(),
        Some(provider_label),
        config.source_name.as_deref(),
        cmd_name,
        None,
        false,
    )
    .await
    .map_err(|e| AdapterError::Internal(format!("Failed to send message: {e}")))?;

    let repo = PgTaskBindingRepository(state.db.clone());
    if let Some(ref ext_id) = msg.external_event_id {
        repo.touch_cursor(&binding.id, ext_id).await?;
    } else {
        repo.touch_last_event(&binding.id).await?;
    }

    if !is_new_task {
        if let Err(e) =
            crate::sandbox::notify::ensure_sandbox_and_notify(state, &binding.task_id).await
        {
            warn!(task_id = %binding.task_id, error = %e, "Failed to ensure sandbox for external message");
        }
    }

    Ok(())
}

async fn update_rule_ongoing_task(
    state: &AppState,
    rule_id: &str,
    task_id: &str,
) -> Result<(), AdapterError> {
    sqlx::query(
        r#"WITH updated_rule AS (
               UPDATE trigger_rules
               SET ongoing_task_id = $2
               WHERE trigger_id = $1
               RETURNING trigger_id
           )
           UPDATE triggers
           SET updated_at = now()
           WHERE id IN (SELECT trigger_id FROM updated_rule)"#,
    )
    .bind(rule_id)
    .bind(task_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

fn should_filter_self_echo(binding: &TaskBinding, user_id: &str) -> bool {
    binding
        .metadata
        .get("filter_user_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|filter| filter == user_id)
}

async fn provision_task(state: &AppState, task_id: &str) {
    match crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        task_id,
    )
    .await
    {
        Ok(request) => {
            if let Err(e) = state.sandbox.provision(task_id, &request).await {
                warn!(task_id, error = %e, "Failed to provision sandbox for external task");
            }
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to build provision request for external task");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_adapter_files_do_not_own_adapter_subject_or_external_id_builders() {
        let shared_files = [
            ("runtime.rs", include_str!("runtime.rs")),
            (
                "../triggers/listen.rs",
                include_str!("../triggers/listen.rs"),
            ),
            ("outbound.rs", include_str!("outbound.rs")),
            ("../adapters/mod.rs", include_str!("../adapters/mod.rs")),
            ("dao.rs", include_str!("dao.rs")),
        ];
        let forbidden = [
            concat!("slack_", "thread"),
            concat!("telegram_", "thread"),
            concat!("github", "_pr_event"),
            concat!("github", "_issue_comment"),
            concat!("github", "_review_comment"),
            concat!("telegram", "_msg"),
            concat!("slack", "_ts"),
        ];

        for (file, source) in shared_files {
            for pattern in forbidden {
                assert!(
                    !source.contains(pattern),
                    "{file} should not contain provider-specific builder/subject pattern {pattern}"
                );
            }
        }
    }
}
