//! Connect tasks to external threads and fan-out inbound messages.
//!
//! `connect` links a task to an existing external thread (e.g. a Slack thread)
//! so that webhooks are delivered and the agent can reply. It is used only by
//! non-GitHub providers — GitHub uses subject-keyed task subscriptions instead
//! and goes through [`connect_github_subscription`].
//!
//! `deliver_to_connected_threads` fans out an inbound message to all tasks
//! connected to the same external thread beyond the primary routing result.

use tracing::{info, warn};

use super::error::ChannelError;
use super::ingress_rule_dao::PgIngressRuleRepository;
use super::models::{
    ChannelConfig, ChannelThread, InboundMessage, IngressRule, RoutingMode, TaskSubscription,
};
use super::provider::ChannelType;
use super::provider_connection_dao::PgProviderConnectionRepository;
use crate::state::AppState;

/// Connect a task to an existing external thread (non-GitHub providers only).
///
/// Finds (or auto-creates) a channel config for the provider+source, then
/// inserts a thread record linking the task to the external thread.
///
/// If `filter_user_id` is provided it is stored in `thread_properties` so
/// that `deliver_to_connected_threads` can skip messages from that user.
///
/// Idempotent: connecting twice to the same thread from the same task returns
/// the existing thread (enforced by the unique constraint on
/// `(config_id, external_thread_id, task_id)`).
///
/// For GitHub PR connections, callers must use [`connect_github_subscription`]
/// instead — GitHub no longer uses `channel_configs` / `channel_threads`.
pub async fn connect(
    state: &AppState,
    provider: ChannelType,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    mut thread_properties: serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(String, ChannelThread), ChannelError> {
    if provider == ChannelType::GithubPr {
        return Err(ChannelError::BadRequest(
            "Use connect_github_subscription for GitHub channels — \
             channel_configs / channel_threads no longer back GitHub PRs"
                .into(),
        ));
    }

    if matches!(provider, ChannelType::Slack | ChannelType::Telegram) {
        let (rule, subscription) = connect_chat_subscription(
            state,
            provider,
            source_id,
            task_id,
            external_thread_id,
            thread_properties,
            filter_user_id,
        )
        .await?;
        let thread = super::subscription::synthetic_thread(&subscription);
        return Ok((rule.id, thread));
    }

    let config = state
        .channel_configs
        .find_by_provider_source(provider.as_str(), source_id)
        .await?;

    let config = if let Some(c) = config {
        c
    } else {
        let task = state
            .tasks
            .find_by_id_internal(task_id)
            .await
            .map_err(|e| ChannelError::Internal(format!("Failed to look up task: {e}")))?
            .ok_or_else(|| ChannelError::BadRequest(format!("Task not found: {task_id}")))?;

        let created_by = task.created_by.as_deref().unwrap_or("system");

        match super::defaults::create_config_from_wildcard(
            state.channel_configs.as_ref(),
            provider.as_str(),
            source_id,
            Some(source_id.to_string()),
            task.team_id.as_deref(),
            created_by,
        )
        .await?
        {
            Some(c) => c,
            None => {
                create_minimal_config(
                    state,
                    provider,
                    source_id,
                    &task.repository_id,
                    created_by,
                    task.team_id.as_deref(),
                )
                .await?
            }
        }
    };

    // Merge filter_user_id into properties (defaulting to config.effective_user_id()).
    let resolved_filter_uid =
        filter_user_id.map_or_else(|| config.effective_user_id().to_string(), str::to_string);
    if !resolved_filter_uid.is_empty() {
        if let Some(obj) = thread_properties.as_object_mut() {
            obj.insert(
                "filter_user_id".into(),
                serde_json::json!(resolved_filter_uid),
            );
        }
    }

    let thread = match state
        .channel_threads
        .insert(&config.id, task_id, external_thread_id, &thread_properties)
        .await
    {
        Ok(t) => t,
        Err(sqlx::Error::Database(ref db_err)) if db_err.is_unique_violation() => {
            let existing = state
                .channel_threads
                .find_by_task_id_and_config(task_id, &config.id)
                .await?
                .ok_or_else(|| {
                    ChannelError::Internal(
                        "Unique violation but thread not found on re-fetch".into(),
                    )
                })?;
            info!(
                config_id = %config.id,
                task_id,
                external_thread_id,
                "Channel thread already exists (idempotent connect)"
            );
            existing
        }
        Err(e) => return Err(e.into()),
    };

    info!(
        config_id = %config.id,
        task_id,
        external_thread_id,
        thread_id = thread.id,
        "Task connected to channel thread"
    );

    Ok((config.id.clone(), thread))
}

async fn connect_chat_subscription(
    state: &AppState,
    provider: ChannelType,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    mut thread_properties: serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(IngressRule, TaskSubscription), ChannelError> {
    let provider_name = provider.as_str();
    let (connection, rule) =
        super::subscription::resolve_chat_ingress(state, provider_name, "default", source_id)
            .await?
            .ok_or_else(|| {
                ChannelError::BadRequest(format!(
                    "No ingress rule for {provider_name} source '{source_id}'"
                ))
            })?;

    let resolved_filter_uid =
        filter_user_id.map_or_else(|| rule.effective_user_id().to_string(), str::to_string);
    if !resolved_filter_uid.is_empty() {
        if let Some(obj) = thread_properties.as_object_mut() {
            obj.insert(
                "filter_user_id".into(),
                serde_json::json!(resolved_filter_uid),
            );
        }
    }

    let (subject_kind, subject_key, reply_target) = match provider {
        ChannelType::Slack => {
            let channel = thread_properties
                .get("channel")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(source_id);
            let thread_ts = thread_properties
                .get("thread_ts")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(external_thread_id);
            let team = thread_properties
                .get("team_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("default");
            (
                "slack_thread",
                format!("{team}:{channel}:{thread_ts}"),
                serde_json::json!({
                    "channel": channel,
                    "thread_ts": thread_ts,
                    "ts": thread_properties
                        .get("ts")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(thread_ts),
                    "filter_user_id": thread_properties.get("filter_user_id").cloned(),
                }),
            )
        }
        ChannelType::Telegram => {
            let chat_id = thread_properties
                .get("chat_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(source_id);
            let forum_topic_id = thread_properties
                .get("forum_topic_id")
                .and_then(serde_json::Value::as_i64);
            let subject_key = match forum_topic_id {
                Some(topic_id) => format!("{chat_id}:topic:{topic_id}"),
                None => format!("{chat_id}:thread:{external_thread_id}"),
            };
            (
                "telegram_thread",
                subject_key,
                serde_json::json!({
                    "chat_id": chat_id,
                    "message_id": thread_properties
                        .get("message_id")
                        .and_then(serde_json::Value::as_i64),
                    "forum_topic_id": forum_topic_id,
                    "filter_user_id": thread_properties.get("filter_user_id").cloned(),
                }),
            )
        }
        _ => unreachable!("only chat subscription providers call this helper"),
    };

    let subscription = super::subscription::upsert_subscription(
        state,
        &connection,
        Some(&rule),
        task_id,
        provider_name,
        subject_kind,
        &subject_key,
        reply_target,
        super::subscription::CREATED_REASON_CONNECTED_THREAD,
        None,
    )
    .await?;

    info!(
        rule_id = %rule.id,
        task_id,
        subscription_id = %subscription.id,
        subject_key = %subscription.subject_key,
        "Task connected to channel subscription"
    );

    Ok((rule, subscription))
}

/// Connect a task to a GitHub PR via the connection/rule/subscription model.
///
/// Returns the `IngressRule` plus the upserted (or existing) `TaskSubscription`.
///
/// Idempotent on `(task_id, provider, subject_kind, subject_key)` — calling
/// twice for the same task and PR returns the same row.
///
/// `thread_properties` must contain `owner`, `repo`, and `pr_number`. When
/// `filter_user_id` is `None`, the rule's effective user (`run_as_user_id` ??
/// `created_by`) is stored as `metadata.filter_user_id` so future events from
/// that user can be filtered as self-echoes.
pub async fn connect_github_subscription(
    state: &AppState,
    source_id: &str,
    task_id: &str,
    mut thread_properties: serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(IngressRule, TaskSubscription), ChannelError> {
    let owner = source_id.split('/').next().ok_or_else(|| {
        ChannelError::BadRequest(format!(
            "Invalid GitHub source_id (expected owner/repo): {source_id}"
        ))
    })?;

    let connection = PgProviderConnectionRepository(state.db.clone())
        .find_by_scope_value("github", owner)
        .await?
        .ok_or_else(|| {
            ChannelError::BadRequest(format!(
                "No GitHub provider_connection for org '{owner}' — \
                 register the org webhook and run the 0034 migration first"
            ))
        })?;

    let rule = PgIngressRuleRepository(state.db.clone())
        .find_by_scope_matcher("github", source_id)
        .await?
        .ok_or_else(|| {
            ChannelError::BadRequest(format!(
                "No ingress rule for GitHub repo '{source_id}' — \
                 create one with `moat channels rules create --repo {source_id}`"
            ))
        })?;

    let pr_number = thread_properties
        .get("pr_number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            ChannelError::BadRequest("thread_properties missing required pr_number".into())
        })?;
    let repo = thread_properties
        .get("repo")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ChannelError::BadRequest("thread_properties missing required repo".into()))?
        .to_string();
    let owner_field = thread_properties
        .get("owner")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ChannelError::BadRequest("thread_properties missing required owner".into()))?
        .to_string();
    let subject_key = format!("{owner_field}/{repo}#{pr_number}");

    let resolved_filter_uid =
        filter_user_id.map_or_else(|| rule.effective_user_id().to_string(), str::to_string);
    if !resolved_filter_uid.is_empty() {
        if let Some(obj) = thread_properties.as_object_mut() {
            obj.insert(
                "filter_user_id".into(),
                serde_json::json!(resolved_filter_uid),
            );
        }
    }

    let subscription = super::github_pr::runtime::upsert_subscription(
        state,
        &connection,
        Some(&rule),
        task_id,
        &subject_key,
        &thread_properties,
        super::github_pr::runtime::CREATED_REASON_CONNECTED_THREAD,
    )
    .await?;

    info!(
        rule_id = %rule.id,
        task_id,
        subject_key = %subject_key,
        subscription_id = %subscription.id,
        "Task connected to GitHub PR subscription"
    );

    Ok((rule, subscription))
}

/// Deliver an inbound message to all tasks connected to the same external thread,
/// excluding the task that was already handled by the primary `process_inbound`.
///
/// Returns the list of task IDs that received the message.
pub async fn deliver_to_connected_threads(
    state: &AppState,
    config_id: &str,
    external_thread_id: &str,
    primary_task_id: &str,
    msg: &InboundMessage,
) -> Vec<String> {
    let threads = match state
        .channel_threads
        .find_all_by_config_and_thread(config_id, external_thread_id)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, config_id, external_thread_id, "Failed to fetch connected threads");
            return vec![];
        }
    };

    let mut delivered_to = Vec::new();

    let config = match state.channel_configs.find_by_id(config_id).await {
        Ok(Some(c)) => Some(c),
        Ok(None) => None,
        Err(e) => {
            warn!(error = %e, config_id, "Failed to load channel config for fan-out");
            None
        }
    };

    for thread in threads {
        if thread.task_id == primary_task_id {
            continue;
        }

        if let Some(ref cfg) = config {
            if should_filter_self_echo(&thread.properties, &msg.user_id, cfg) {
                continue;
            }
        } else if let Some(filter_uid) = thread
            .properties
            .get("filter_user_id")
            .and_then(|v| v.as_str())
        {
            if msg.user_id == filter_uid {
                continue;
            }
        }

        let task_id = &thread.task_id;

        if let Err(e) = crate::message::send::send_message_flow(
            state,
            task_id,
            &msg.text,
            None,
            None,
            &msg.file_ids,
            &[],
            true,
            msg.channel_external_id.as_deref(),
            Some(msg.provider.as_str()),
            None,
            None,
        )
        .await
        {
            warn!(task_id, error = %e, "Failed to deliver message to connected thread");
            continue;
        }

        if let Err(e) = state.channel_threads.touch(thread.id, None).await {
            warn!(task_id, error = %e, "Failed to touch connected thread");
        }

        if let Err(e) = crate::sandbox::notify::ensure_sandbox_and_notify(state, task_id).await {
            warn!(task_id, error = %e, "Failed to ensure sandbox for connected thread");
        }

        delivered_to.push(task_id.clone());
    }

    if !delivered_to.is_empty() {
        info!(
            config_id,
            external_thread_id,
            count = delivered_to.len(),
            "Delivered message to connected threads"
        );
    }

    delivered_to
}

/// Returns true when an inbound message from `msg_user_id` should be skipped
/// because it is the originating side of a self-echo.
pub fn should_filter_self_echo(
    thread_properties: &serde_json::Value,
    msg_user_id: &str,
    config: &ChannelConfig,
) -> bool {
    let stored = thread_properties
        .get("filter_user_id")
        .and_then(|v| v.as_str());
    let filter_uid = stored.unwrap_or_else(|| config.effective_user_id());
    !filter_uid.is_empty() && filter_uid == msg_user_id
}

/// Create a minimal channel config for non-GitHub auto-connect.
///
/// Auto-connect is explicit opt-in (the user ran `channel connect`), so we
/// default to listening for every supported event the provider emits.
async fn create_minimal_config(
    state: &AppState,
    provider: ChannelType,
    source_id: &str,
    repository_id: &str,
    created_by: &str,
    team_id: Option<&str>,
) -> Result<ChannelConfig, ChannelError> {
    let now = chrono::Utc::now();
    let config = ChannelConfig {
        id: nanoid::nanoid!(21),
        provider: provider.as_str().to_string(),
        source_id: source_id.to_string(),
        source_name: Some(source_id.to_string()),
        repository_id: repository_id.to_string(),
        routing_mode: RoutingMode::ThreadBased,
        prompt_template: None,
        active: true,
        allow_direct_send: true,
        provider_config: None,
        ongoing_task_id: None,
        response_config_id: None,
        scope: crate::scope::Scope::Team,
        team_id: team_id.map(String::from),
        task_name: None,
        tag_ids: vec![],
        run_as_user_id: None,
        agent_type: "claude-code".to_string(),
        created_by: created_by.to_string(),
        created_at: now,
        updated_at: now,
    };

    state.channel_configs.insert(&config).await?;

    info!(
        config_id = %config.id,
        provider = provider.as_str(),
        source_id,
        "Auto-created minimal channel config for connect"
    );

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;

    fn cfg_with(run_as: Option<&str>, created_by: &str) -> ChannelConfig {
        ChannelConfig {
            id: "cfg-1".into(),
            provider: "slack".into(),
            source_id: "C123".into(),
            source_name: None,
            repository_id: "repo-1".into(),
            routing_mode: RoutingMode::ThreadBased,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            provider_config: None,
            ongoing_task_id: None,
            response_config_id: None,
            scope: Scope::Team,
            team_id: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: run_as.map(String::from),
            agent_type: "claude-code".into(),
            created_by: created_by.into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn should_filter_self_echo_explicit_filter() {
        let cfg = cfg_with(None, "alice");
        let props = serde_json::json!({"filter_user_id": "bot-user"});
        assert!(should_filter_self_echo(&props, "bot-user", &cfg));
        assert!(!should_filter_self_echo(&props, "other", &cfg));
    }

    #[test]
    fn should_filter_self_echo_defaults_to_run_as_user() {
        let cfg = cfg_with(Some("bot-user"), "alice");
        let props = serde_json::json!({});
        assert!(should_filter_self_echo(&props, "bot-user", &cfg));
        assert!(!should_filter_self_echo(&props, "alice", &cfg));
    }

    #[test]
    fn should_filter_self_echo_defaults_to_created_by_when_no_run_as() {
        let cfg = cfg_with(None, "alice");
        let props = serde_json::json!({});
        assert!(should_filter_self_echo(&props, "alice", &cfg));
        assert!(!should_filter_self_echo(&props, "bob", &cfg));
    }

    #[test]
    fn should_filter_self_echo_explicit_overrides_default() {
        let cfg = cfg_with(Some("bot-user"), "alice");
        let props = serde_json::json!({"filter_user_id": ""});
        assert!(!should_filter_self_echo(&props, "bot-user", &cfg));

        let props = serde_json::json!({"filter_user_id": "explicit-bot"});
        assert!(should_filter_self_echo(&props, "explicit-bot", &cfg));
        assert!(!should_filter_self_echo(&props, "bot-user", &cfg));
    }

    #[test]
    fn should_filter_self_echo_empty_user_id_is_safe() {
        let cfg = cfg_with(Some(""), "");
        let props = serde_json::json!({});
        assert!(!should_filter_self_echo(&props, "anyone", &cfg));
        assert!(!should_filter_self_echo(&props, "", &cfg));
    }
}
