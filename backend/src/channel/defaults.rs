//! Shared wildcard config cloning — creates a channel config from a `source_id="*"` template.

use tracing::info;

use super::config_dao::ChannelConfigRepository;
use super::error::ChannelError;
use super::models::ChannelConfig;

/// Look up a wildcard (`source_id="*"`) config for the given provider and clone
/// it into a new config bound to `source_id`.
///
/// Lookup order:
/// 1. Team-scoped wildcard (`provider, *, team_id`) — if `team_id` is provided.
/// 2. Global wildcard (`provider, *`).
/// 3. Returns `Ok(None)` if no wildcard exists.
pub async fn create_config_from_wildcard(
    channel_configs: &dyn ChannelConfigRepository,
    provider: &str,
    source_id: &str,
    source_name: Option<String>,
    team_id: Option<&str>,
    created_by: &str,
) -> Result<Option<ChannelConfig>, ChannelError> {
    let wildcard = channel_configs
        .find_wildcard(provider, team_id)
        .await?;

    let Some(template) = wildcard else {
        return Ok(None);
    };

    let now = chrono::Utc::now();

    let new_config = ChannelConfig {
        id: nanoid::nanoid!(21),
        provider: provider.to_string(),
        source_id: source_id.to_string(),
        source_name,
        repository_id: template.repository_id.clone(),
        routing_mode: template.routing_mode,
        prompt_template: template.prompt_template.clone(),
        active: true,
        provider_config: template.provider_config.clone(),
        ongoing_task_id: None,
        response_config_id: template.response_config_id.clone(),
        scope: template.scope,
        team_id: template.team_id.clone(),
        created_by: created_by.to_string(),
        created_at: now,
        updated_at: now,
    };

    channel_configs.insert(&new_config).await?;

    info!(
        config_id = %new_config.id,
        provider,
        source_id,
        template_id = %template.id,
        "Created channel config from wildcard template"
    );

    Ok(Some(new_config))
}
