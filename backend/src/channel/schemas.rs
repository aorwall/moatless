//! Channel API request/response schemas.
//!
//! Re-exported from the `moatless-api-schemas` crate.
//! Conversion functions live here since we can't define inherent impls on foreign types.

pub use moatless_api_schemas::channel::*;

use super::provider::ChannelType;

pub fn channel_config_from_model(row: &super::models::ChannelConfig) -> ChannelConfigResponse {
    let provider = ChannelType::parse(&row.provider).unwrap_or(ChannelType::Slack);
    ChannelConfigResponse {
        id: row.id.clone(),
        provider,
        source_id: row.source_id.clone(),
        source_name: row.source_name.clone(),
        repository_id: row.repository_id.clone(),
        routing_mode: row.routing_mode,
        prompt_template: row.prompt_template.clone(),
        active: row.active,
        allow_direct_send: row.allow_direct_send,
        provider_config: row.provider_config.clone(),
        ongoing_task_id: row.ongoing_task_id.clone(),
        response_config_id: row.response_config_id.clone(),
        scope: row.scope,
        team_id: row.team_id.clone(),
        task_name: row.task_name.clone(),
        tag_ids: row.tag_ids.clone(),
        run_as_user_id: row.run_as_user_id.clone(),
        agent_type: row.agent_type.clone(),
        created_by: row.created_by.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub fn task_channel_from_model(config: &super::models::ChannelConfig) -> TaskChannelResponse {
    let provider = ChannelType::parse(&config.provider).unwrap_or(ChannelType::Slack);
    TaskChannelResponse {
        config_id: config.id.clone(),
        provider,
        source_id: config.source_id.clone(),
        source_name: config.source_name.clone(),
        thread_id: None,
        external_thread_id: None,
        subject_key: None,
        subscription_id: None,
        created_reason: None,
        last_event_at: None,
    }
}

pub fn task_channel_from_thread(
    config: &super::models::ChannelConfig,
    thread: &super::models::ChannelThread,
) -> TaskChannelResponse {
    let provider = ChannelType::parse(&config.provider).unwrap_or(ChannelType::Slack);
    TaskChannelResponse {
        config_id: config.id.clone(),
        provider,
        source_id: config.source_id.clone(),
        source_name: config.source_name.clone(),
        thread_id: Some(thread.id),
        external_thread_id: Some(thread.external_thread_id.clone()),
        subject_key: build_subject_key(config, thread),
        subscription_id: None,
        created_reason: None,
        last_event_at: None,
    }
}

pub fn task_channel_from_subscription(
    subscription: &super::models::TaskSubscription,
    rule: Option<&super::models::IngressRule>,
) -> TaskChannelResponse {
    let provider = ChannelType::GithubPr;
    let source_id = rule.map_or_else(
        || subscription.subject_key.clone(),
        |r| r.scope_matcher.clone(),
    );
    let pr_number = subscription
        .reply_target
        .get("pr_number")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n.to_string());

    TaskChannelResponse {
        config_id: rule.map(|r| r.id.clone()).unwrap_or_default(),
        provider,
        source_id: source_id.clone(),
        source_name: Some(source_id),
        thread_id: None,
        external_thread_id: pr_number,
        subject_key: Some(subscription.subject_key.clone()),
        subscription_id: Some(subscription.id.clone()),
        created_reason: Some(subscription.created_reason.clone()),
        last_event_at: Some(subscription.last_event_at.to_rfc3339()),
    }
}

fn build_subject_key(
    config: &super::models::ChannelConfig,
    thread: &super::models::ChannelThread,
) -> Option<String> {
    if config.provider != "github_pr" {
        return None;
    }

    let owner = thread.properties.get("owner").and_then(|v| v.as_str())?;
    let repo = thread.properties.get("repo").and_then(|v| v.as_str())?;
    let pr_number = thread
        .properties
        .get("pr_number")
        .and_then(serde_json::Value::as_u64)?;
    Some(format!("{owner}/{repo}#{pr_number}"))
}

pub fn pairing_request_from_model(
    r: &super::telegram::pairing::PairingRequest,
) -> PairingRequestResponse {
    PairingRequestResponse {
        id: r.id,
        config_id: r.config_id.clone(),
        provider_user_id: r.provider_user_id.clone(),
        code: r.code.clone(),
        status: r.status.clone(),
        meta: r.meta.clone(),
        created_at: r.created_at.to_rfc3339(),
        updated_at: r.updated_at.to_rfc3339(),
    }
}
