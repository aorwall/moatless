//! Connect tasks to external threads and fan-out inbound messages.
//!
//! `connect` links a task to an existing external thread (e.g. a GitHub PR)
//! so that webhooks are delivered and the agent can reply.
//!
//! `deliver_to_connected_threads` fans out an inbound message to all tasks
//! connected to the same external thread beyond the primary routing result.

use tracing::{info, warn};

use super::error::ChannelError;
use super::models::{ChannelConfig, ChannelThread, InboundMessage, RoutingMode};
use super::provider::ChannelType;
use crate::state::AppState;

/// Connect a task to an existing external thread.
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
pub async fn connect(
    state: &AppState,
    provider: ChannelType,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    mut thread_properties: serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(String, ChannelThread), ChannelError> {
    // 1. Find config
    let config = state
        .channel_configs
        .find_by_provider_source(provider.as_str(), source_id)
        .await?;

    let config = if let Some(c) = config {
        c
    } else {
        // Try wildcard auto-create
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
                // Auto-create a minimal config
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

    // 2. Merge filter_user_id into properties.
    //
    // When the caller does not specify one, default to the channel's
    // effective user (run_as_user_id falls back to created_by). That user is
    // who the agent posts as, so any inbound webhook whose user_id matches
    // is by definition an echo of something this thread's task posted.
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

    // 3. Insert thread — on unique constraint violation, fetch existing
    let thread = match state
        .channel_threads
        .insert(&config.id, task_id, external_thread_id, &thread_properties)
        .await
    {
        Ok(t) => t,
        Err(sqlx::Error::Database(ref db_err)) if db_err.is_unique_violation() => {
            // Idempotent: thread already exists for this (config, thread, task)
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

    // Look up the config once so we can default filter_user_id to its
    // effective user when the thread doesn't carry an explicit one.
    let config = match state.channel_configs.find_by_id(config_id).await {
        Ok(Some(c)) => Some(c),
        Ok(None) => None,
        Err(e) => {
            warn!(error = %e, config_id, "Failed to load channel config for fan-out");
            None
        }
    };

    for thread in threads {
        // Skip the thread already handled by primary routing
        if thread.task_id == primary_task_id {
            continue;
        }

        // Check per-thread filter_user_id (with config fallback)
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

        // Deliver message to the task
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

        // Update thread last_message_at
        if let Err(e) = state.channel_threads.touch(thread.id, None).await {
            warn!(task_id, error = %e, "Failed to touch connected thread");
        }

        // Wake sandbox
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
///
/// Resolution: the thread's stored `filter_user_id`, or — when absent — the
/// channel config's `effective_user_id()` (run_as_user_id ?? created_by).
/// That user is who the agent posts as, so any inbound whose `msg.user_id`
/// matches is by definition our own echo.
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

/// Create a minimal channel config for auto-connect.
///
/// Auto-connect is explicit opt-in (the user ran `channel connect`), so we
/// default to listening for every supported event the provider emits. For
/// GitHub PR that means also seeding `notify_on_success: true` so successful
/// `check_suite` events are forwarded — the failure path is already on by
/// default via `check_run`.
async fn create_minimal_config(
    state: &AppState,
    provider: ChannelType,
    source_id: &str,
    repository_id: &str,
    created_by: &str,
    team_id: Option<&str>,
) -> Result<ChannelConfig, ChannelError> {
    let now = chrono::Utc::now();
    let provider_config = match provider {
        ChannelType::GithubPr => Some(serde_json::json!({ "notify_on_success": true })),
        _ => None,
    };
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
        provider_config,
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
            provider: "github_pr".into(),
            source_id: "Frank-Penny/soap-config".into(),
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
        // Explicit empty string in thread_properties — falls back to default
        // since helper requires non-empty filter id.
        let cfg = cfg_with(Some("bot-user"), "alice");
        let props = serde_json::json!({"filter_user_id": ""});
        assert!(!should_filter_self_echo(&props, "bot-user", &cfg));

        // Explicit different filter overrides config default
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
