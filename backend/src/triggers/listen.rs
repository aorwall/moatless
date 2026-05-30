//! Connect tasks to external provider subjects.
//!
//! Connections are persisted as canonical task bindings. Shared code resolves
//! connections/triggers and delegates provider-specific binding destination
//! construction to the owning provider.

use tracing::info;

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, BindingContext, TaskBinding, TaskTriggerRoute};
use crate::adapters::{Adapter, AdapterKind};
use crate::state::AppState;
use crate::triggers::rule_dao::PgTaskTriggerRouteRepository;

/// Connect a task to an existing provider thread by creating a task binding.
pub async fn connect(
    state: &AppState,
    provider_type: AdapterKind,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(String, BindingContext), AdapterError> {
    let provider = state
        .adapters
        .as_ref()
        .and_then(|providers| providers.get(provider_type))
        .ok_or_else(|| AdapterError::AdapterNotConfigured(provider_type.as_str().to_string()))?;

    let (rule, binding) = connect_binding(
        state,
        provider.as_ref(),
        source_id,
        task_id,
        external_thread_id,
        &thread_properties,
        filter_user_id,
    )
    .await?;
    let thread = crate::bindings::runtime::binding_context(&binding);
    Ok((rule.id, thread))
}

#[allow(clippy::too_many_arguments)]
async fn connect_binding(
    state: &AppState,
    provider: &dyn Adapter,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(TaskTriggerRoute, TaskBinding), AdapterError> {
    let (connection, rule) =
        resolve_connection_and_rule(state, provider, source_id, thread_properties).await?;

    let resolved_filter_uid =
        filter_user_id.map_or_else(|| rule.effective_user_id().to_string(), str::to_string);
    let filter_user_id = if resolved_filter_uid.is_empty() {
        None
    } else {
        Some(resolved_filter_uid.as_str())
    };

    let destination = provider.binding_destination_from_existing_thread(
        source_id,
        external_thread_id,
        thread_properties,
        filter_user_id,
    )?;

    let binding = crate::bindings::runtime::upsert_binding(
        state,
        &connection,
        Some(&rule),
        task_id,
        provider.connection_provider_key(),
        destination.subject_kind,
        &destination.subject_key,
        destination.destination_config,
        crate::bindings::runtime::ORIGIN_CONNECTED_THREAD,
        destination.last_external_event_id,
    )
    .await?;

    info!(
        rule_id = %rule.id,
        task_id,
        binding_id = %binding.id,
        subject_key = %binding.subject_key,
        provider = provider.provider_key(),
        "Task connected to integration binding"
    );

    Ok((rule, binding))
}

async fn resolve_connection_and_rule(
    state: &AppState,
    provider: &dyn Adapter,
    source_id: &str,
    thread_properties: &serde_json::Value,
) -> Result<(AdapterConnection, TaskTriggerRoute), AdapterError> {
    let provider_key = provider.connection_provider_key();
    let scope_value =
        provider.connection_scope_value_from_existing_thread(source_id, thread_properties)?;

    let connection_repo = PgAdapterConnectionRepository(state.db.clone());
    let mut connection = connection_repo
        .find_by_scope_value(provider_key, &scope_value)
        .await?;
    if connection.is_none()
        && provider.allow_default_connection_fallback()
        && scope_value != "default"
    {
        connection = connection_repo
            .find_by_scope_value(provider_key, "default")
            .await?;
    }
    let connection = connection.ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "No {provider_key} provider_connection for scope '{scope_value}'"
        ))
    })?;

    let source_matcher =
        provider.trigger_source_matcher_from_existing_thread(source_id, thread_properties)?;
    let rule_repo = PgTaskTriggerRouteRepository(state.db.clone());
    let mut rule = rule_repo
        .find_by_source_matcher(provider_key, &source_matcher)
        .await?;
    if rule.is_none() && provider.allow_wildcard_trigger_fallback() {
        rule = rule_repo.find_wildcard(provider_key).await?;
    }
    let rule = rule.ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "No task trigger for {provider_key} source '{source_matcher}'"
        ))
    })?;

    if rule.connection_id != connection.id {
        return Err(AdapterError::BadRequest(format!(
            "Task trigger {} belongs to a different connection",
            rule.id
        )));
    }

    Ok((connection, rule))
}

/// Connect a task to a GitHub PR via the connection/trigger/binding model.
///
/// Compatibility wrapper for callers that already parse GitHub PR details.
pub async fn connect_github_binding(
    state: &AppState,
    source_id: &str,
    task_id: &str,
    thread_properties: serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(TaskTriggerRoute, TaskBinding), AdapterError> {
    if let Some(provider) = state
        .adapters
        .as_ref()
        .and_then(|providers| providers.get(AdapterKind::GithubPr))
    {
        return connect_binding(
            state,
            provider.as_ref(),
            source_id,
            task_id,
            "",
            &thread_properties,
            filter_user_id,
        )
        .await;
    }

    let provider = crate::adapters::github_pr::adapter::GithubPrAdapter::new(
        state.config.github_pr.webhook_secret.clone(),
        state.config.github_pr.bot_token.clone(),
        state.db.clone(),
        state.provider_tokens.clone(),
        Some(state.provider_token_auth.clone()),
        state.github_app_service.clone(),
        state.github_apps.clone(),
    );
    connect_binding(
        state,
        &provider,
        source_id,
        task_id,
        "",
        &thread_properties,
        filter_user_id,
    )
    .await
}
