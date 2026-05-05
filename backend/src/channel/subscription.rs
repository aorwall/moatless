//! Shared subscription runtime for chat providers backed by
//! `provider_connections`, `ingress_rules`, and `task_subscriptions`.

use chrono::Utc;
use tracing::{debug, info, warn};

use super::error::ChannelError;
use super::inbound::{self, CommandInfo};
use super::ingress_rule_dao::PgIngressRuleRepository;
use super::models::{
    ChannelConfig, ChannelThread, InboundMessage, IngressRule, ProviderConnection, RoutingMode,
    TaskSubscription,
};
use super::provider_connection_dao::PgProviderConnectionRepository;
use super::task_subscription_dao::PgTaskSubscriptionRepository;
use crate::state::AppState;
use crate::task::models::TaskStatus;

pub const CREATED_REASON_INGRESS: &str = "ingress";
pub const CREATED_REASON_CONNECTED_THREAD: &str = "connected_thread";

/// Build a synthetic `ChannelConfig` from an ingress rule for provider trait
/// methods that still accept the legacy config shape.
pub fn synthetic_config(rule: &IngressRule, provider_label: &str) -> ChannelConfig {
    ChannelConfig {
        id: rule.id.clone(),
        provider: provider_label.to_string(),
        source_id: rule.scope_matcher.clone(),
        source_name: rule.source_name.clone(),
        repository_id: rule.repository_id.clone(),
        routing_mode: rule.routing_mode,
        prompt_template: rule.prompt_template.clone(),
        active: rule.active,
        allow_direct_send: rule.allow_direct_send,
        provider_config: rule.provider_config.clone(),
        ongoing_task_id: rule.ongoing_task_id.clone(),
        response_config_id: rule.response_rule_id.clone(),
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

/// Build a synthetic `ChannelThread` from a subscription for provider trait
/// methods that still accept the legacy thread shape. Never persist this row.
pub fn synthetic_thread(subscription: &TaskSubscription) -> ChannelThread {
    let mut props = subscription.reply_target.clone();
    if let Some(obj) = props.as_object_mut() {
        if let Some(ext_id) = subscription.last_channel_external_id.as_ref() {
            obj.insert("last_channel_external_id".into(), serde_json::json!(ext_id));
        }
        if let Some(filter_uid) = subscription.metadata.get("filter_user_id").cloned() {
            obj.insert("filter_user_id".into(), filter_uid);
        }
    }

    ChannelThread {
        id: 0,
        config_id: subscription.ingress_rule_id.clone().unwrap_or_default(),
        task_id: subscription.task_id.clone(),
        external_thread_id: subscription.subject_key.clone(),
        properties: props,
        last_message_at: subscription.last_event_at,
        created_at: subscription.created_at,
    }
}

pub fn slack_subject_key(msg: &InboundMessage) -> Result<String, ChannelError> {
    let channel = msg
        .thread_properties
        .get("channel")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&msg.source_id);
    let thread_ts = msg
        .thread_properties
        .get("thread_ts")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&msg.message_thread_id);
    let team = msg
        .provider_context
        .get("team_id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    Ok(format!("{team}:{channel}:{thread_ts}"))
}

pub fn telegram_subject_key(msg: &InboundMessage) -> Result<String, ChannelError> {
    let chat_id = msg
        .thread_properties
        .get("chat_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&msg.source_id);
    let forum_topic_id = msg
        .thread_properties
        .get("forum_topic_id")
        .and_then(serde_json::Value::as_i64);
    Ok(match forum_topic_id {
        Some(topic_id) => format!("{chat_id}:topic:{topic_id}"),
        None => format!("{chat_id}:thread:{}", msg.message_thread_id),
    })
}

pub async fn resolve_chat_ingress(
    state: &AppState,
    provider: &str,
    scope_value: &str,
    source_id: &str,
) -> Result<Option<(ProviderConnection, IngressRule)>, ChannelError> {
    let connection_repo = PgProviderConnectionRepository(state.db.clone());
    let Some(connection) = connection_repo
        .find_by_scope_value(provider, scope_value)
        .await?
        .or(connection_repo
            .find_by_scope_value(provider, "default")
            .await?)
    else {
        return Ok(None);
    };

    let rule_repo = PgIngressRuleRepository(state.db.clone());
    let rule = rule_repo
        .find_by_scope_matcher(provider, source_id)
        .await?
        .or(rule_repo.find_wildcard(provider).await?);
    let Some(rule) = rule else {
        return Ok(None);
    };

    if rule.connection_id != connection.id {
        debug!(
            provider,
            source_id,
            rule_id = %rule.id,
            connection_id = %connection.id,
            "Ingress rule belongs to a different connection"
        );
        return Ok(None);
    }

    Ok(Some((connection, rule)))
}

#[allow(clippy::too_many_arguments)]
pub async fn route_inbound_message(
    state: &AppState,
    connection: &ProviderConnection,
    rule: &IngressRule,
    msg: InboundMessage,
    command: Option<CommandInfo>,
    provider_label: &str,
    subject_kind: &str,
    subject_key: &str,
    reply_target: serde_json::Value,
) -> Result<String, ChannelError> {
    if !rule.active {
        debug!(rule_id = %rule.id, "Ignoring channel message: ingress rule inactive");
        return Err(ChannelError::ConfigNotFound(rule.id.clone()));
    }

    let subscription_repo = PgTaskSubscriptionRepository(state.db.clone());
    let origin_task_id = match msg.channel_external_id.as_deref() {
        Some(ext_id) => {
            let candidates = super::external_id::legacy_and_new(ext_id);
            state
                .messages
                .find_task_id_by_any_channel_external_id(&candidates)
                .await?
        }
        None => None,
    };

    let mut subscriptions = subscription_repo
        .find_by_subject(provider_label, subject_kind, subject_key)
        .await?;

    if let Some(origin_task_id) = origin_task_id.as_deref() {
        if !subscriptions.iter().any(|s| s.task_id == origin_task_id) {
            let sub = upsert_subscription(
                state,
                connection,
                Some(rule),
                origin_task_id,
                provider_label,
                subject_kind,
                subject_key,
                reply_target.clone(),
                CREATED_REASON_CONNECTED_THREAD,
                msg.channel_external_id.clone(),
            )
            .await?;
            subscriptions.push(sub);
        }
    }

    if rule.routing_mode == RoutingMode::Ongoing {
        if let Some(task_id) = open_ongoing_task_id(state, rule).await {
            if !subscriptions.iter().any(|s| s.task_id == task_id) {
                let sub = upsert_subscription(
                    state,
                    connection,
                    Some(rule),
                    &task_id,
                    provider_label,
                    subject_kind,
                    subject_key,
                    reply_target.clone(),
                    CREATED_REASON_INGRESS,
                    msg.channel_external_id.clone(),
                )
                .await?;
                subscriptions.push(sub);
            }
        }
    }

    let mut is_new_task = false;
    if should_create_subscription_task(state, rule, &subscriptions).await? {
        let config = synthetic_config(rule, provider_label);
        let task_id =
            inbound::create_task_for_channel(state, &config, &msg, command.as_ref()).await?;
        provision_task(state, &task_id).await;
        if rule.routing_mode == RoutingMode::Ongoing {
            update_rule_ongoing_task(state, &rule.id, &task_id).await?;
        }
        let sub = upsert_subscription(
            state,
            connection,
            Some(rule),
            &task_id,
            provider_label,
            subject_kind,
            subject_key,
            reply_target.clone(),
            CREATED_REASON_INGRESS,
            msg.channel_external_id.clone(),
        )
        .await?;
        subscriptions.push(sub);
        is_new_task = true;
        info!(task_id = %task_id, rule_id = %rule.id, provider = provider_label, "Created task from subscription ingress");
    }

    let mut delivered_task_id = None;
    for subscription in subscriptions {
        if should_filter_self_echo(&subscription, &msg.user_id)
            || origin_task_id.as_deref() == Some(subscription.task_id.as_str())
        {
            debug!(
                task_id = %subscription.task_id,
                subject_key,
                provider = provider_label,
                "Skipping self-echo/originating subscription"
            );
            delivered_task_id = Some(subscription.task_id.clone());
            continue;
        }

        let this_is_new = is_new_task && subscription.created_reason == CREATED_REASON_INGRESS;
        deliver_to_subscription(
            state,
            rule,
            &subscription,
            &msg,
            command.as_ref(),
            provider_label,
            this_is_new,
        )
        .await?;
        delivered_task_id = Some(subscription.task_id.clone());
    }

    delivered_task_id.ok_or_else(|| ChannelError::ConfigNotFound(rule.id.clone()))
}

async fn should_create_subscription_task(
    state: &AppState,
    rule: &IngressRule,
    subscriptions: &[TaskSubscription],
) -> Result<bool, ChannelError> {
    if rule.routing_mode != RoutingMode::Ongoing {
        return Ok(subscriptions.is_empty());
    }

    if !subscriptions.is_empty() {
        return Ok(false);
    }
    Ok(open_ongoing_task_id(state, rule).await.is_none())
}

async fn open_ongoing_task_id(state: &AppState, rule: &IngressRule) -> Option<String> {
    let task_id = rule.ongoing_task_id.as_deref()?;
    match state.tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) if task.status == TaskStatus::Open => Some(task.task_id),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_subscription(
    state: &AppState,
    connection: &ProviderConnection,
    rule: Option<&IngressRule>,
    task_id: &str,
    provider: &str,
    subject_kind: &str,
    subject_key: &str,
    reply_target: serde_json::Value,
    created_reason: &str,
    last_channel_external_id: Option<String>,
) -> Result<TaskSubscription, ChannelError> {
    let now = Utc::now();
    let filter_uid = reply_target
        .get("filter_user_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let subscription = TaskSubscription {
        id: format!("ts_{}", uuid::Uuid::new_v4()),
        task_id: task_id.to_string(),
        provider: provider.to_string(),
        subject_kind: subject_kind.to_string(),
        subject_key: subject_key.to_string(),
        connection_id: connection.id.clone(),
        ingress_rule_id: rule.map(|r| r.id.clone()),
        reply_target,
        metadata: serde_json::json!({ "filter_user_id": filter_uid }),
        last_channel_external_id,
        created_reason: created_reason.to_string(),
        created_at: now,
        last_event_at: now,
    };
    Ok(PgTaskSubscriptionRepository(state.db.clone())
        .upsert(&subscription)
        .await?)
}

async fn deliver_to_subscription(
    state: &AppState,
    rule: &IngressRule,
    subscription: &TaskSubscription,
    msg: &InboundMessage,
    command: Option<&CommandInfo>,
    provider_label: &str,
    is_new_task: bool,
) -> Result<(), ChannelError> {
    let config = synthetic_config(rule, provider_label);
    let thread = synthetic_thread(subscription);

    let thread_context = if msg.thread_id.is_some() {
        if let Some(ref providers) = state.channel_providers {
            if let Some(provider) = providers.get_by_name(provider_label) {
                let mut exclude_ids = state
                    .messages
                    .get_channel_external_ids(&subscription.task_id)
                    .await
                    .unwrap_or_default();
                if let Some(ref ext_id) = msg.channel_external_id {
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
        let provider_user_id = msg
            .provider_context
            .get("user_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let label = if provider_user_id.is_empty() {
            format!("{display} ({login})", login = user.login)
        } else {
            format!(
                "{display} ({login}, Slack: {provider_user_id})",
                login = user.login
            )
        };
        message_text = format!("{label}: {message_text}");
    }

    let is_bot_mentioned = msg
        .provider_context
        .get("is_bot_mentioned")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if is_bot_mentioned {
        message_text = format!(
            "[You were @mentioned — a reply via `moat reply` is required]\n\n{message_text}"
        );
    }

    if msg.thread_id.is_some() {
        if let Some(providers) = state.channel_providers.as_ref() {
            if let Some(provider) = providers.get_by_name(provider_label) {
                if let Some(hint) = provider.thread_reply_hint(msg) {
                    message_text = format!("{hint}\n\n{message_text}");
                }
            }
        }
    }

    if let Some(ctx) = thread_context {
        if subscription.last_channel_external_id.is_some() {
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
        &subscription.task_id,
        &message_text,
        None,
        cmd_agent_mode,
        &msg.file_ids,
        &[],
        cmd_resume,
        msg.channel_external_id.as_deref(),
        Some(provider_label),
        config.source_name.as_deref(),
        cmd_name,
        None,
        false,
    )
    .await
    .map_err(|e| ChannelError::Internal(format!("Failed to send message: {e}")))?;

    let repo = PgTaskSubscriptionRepository(state.db.clone());
    if let Some(ref ext_id) = msg.channel_external_id {
        repo.touch_cursor(&subscription.id, ext_id).await?;
    } else {
        repo.touch_last_event(&subscription.id).await?;
    }

    if !is_new_task {
        if let Err(e) =
            crate::sandbox::notify::ensure_sandbox_and_notify(state, &subscription.task_id).await
        {
            warn!(task_id = %subscription.task_id, error = %e, "Failed to ensure sandbox for channel message");
        }
    }

    Ok(())
}

async fn update_rule_ongoing_task(
    state: &AppState,
    rule_id: &str,
    task_id: &str,
) -> Result<(), ChannelError> {
    sqlx::query("UPDATE ingress_rules SET ongoing_task_id = $2, updated_at = now() WHERE id = $1")
        .bind(rule_id)
        .bind(task_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

fn should_filter_self_echo(subscription: &TaskSubscription, user_id: &str) -> bool {
    subscription
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
                warn!(task_id, error = %e, "Failed to provision sandbox for channel task");
            }
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to build provision request for channel task");
        }
    }
}
