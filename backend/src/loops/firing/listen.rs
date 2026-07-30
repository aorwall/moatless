//! Connect tasks to external provider subjects.
//!
//! Connections are persisted as canonical task bindings. Shared code resolves
//! connections/triggers and delegates provider-specific binding destination
//! construction to the owning provider.

use tracing::{info, warn};

use crate::adapters::dao::PgAdapterConnectionDao;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, BindingContext, SubscriptionMatch, TaskBinding};
use crate::adapters::{AdapterDestinationBuilder, AdapterKind};
use crate::state::AppState;

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
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured(provider_type.as_str().to_string()))?;
    let provider = providers.destination_builder(provider_type)?;

    let (rule, binding) = connect_binding(
        state,
        provider,
        provider_type.as_str(),
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
    provider: &dyn AdapterDestinationBuilder,
    provider_label: &str,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(SubscriptionMatch, TaskBinding), AdapterError> {
    let (connection, rule) =
        resolve_connection_and_rule(state, provider, source_id, thread_properties).await?;

    let binding = write_binding(
        state,
        provider,
        &connection,
        Some(&rule),
        source_id,
        task_id,
        external_thread_id,
        thread_properties,
        filter_user_id,
    )
    .await?;

    info!(
        rule_id = %rule.id,
        task_id,
        binding_id = %binding.id,
        subject_key = %binding.subject_key,
        provider = provider_label,
        "Task connected to integration binding"
    );

    Ok((rule, binding))
}

/// Like [`connect_binding`], but does not require a matching trigger.
///
/// Used for outbound binds — e.g. a PR opened by the agent via
/// `moat gh pr create` — where the connection alone is enough to persist the
/// task↔thread binding so it appears under the task's Connections. Inbound
/// delivery still finds the binding by its subject key
/// (`route_inbound_message` looks up bindings via `find_by_subject`, not by
/// `trigger_id`). A trigger that resolves to a different connection is ignored
/// rather than treated as an error.
#[allow(clippy::too_many_arguments)]
async fn connect_binding_optional_rule(
    state: &AppState,
    provider: &dyn AdapterDestinationBuilder,
    provider_label: &str,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(Option<SubscriptionMatch>, TaskBinding), AdapterError> {
    let connection = resolve_connection(state, provider, source_id, thread_properties).await?;
    let rule =
        resolve_optional_rule(state, provider, source_id, thread_properties, &connection).await?;

    let binding = write_binding(
        state,
        provider,
        &connection,
        rule.as_ref(),
        source_id,
        task_id,
        external_thread_id,
        thread_properties,
        filter_user_id,
    )
    .await?;

    info!(
        rule_id = rule.as_ref().map_or("<none>", |r| r.id.as_str()),
        task_id,
        binding_id = %binding.id,
        subject_key = %binding.subject_key,
        provider = provider_label,
        "Task connected to integration binding (outbound, trigger optional)"
    );

    Ok((rule, binding))
}

/// Build the provider destination and upsert the canonical task binding.
///
/// `rule` is optional: outbound binds may have no matching trigger, in which
/// case `upsert_binding` writes a `NULL` `trigger_id`.
#[allow(clippy::too_many_arguments)]
async fn write_binding(
    state: &AppState,
    provider: &dyn AdapterDestinationBuilder,
    connection: &AdapterConnection,
    rule: Option<&SubscriptionMatch>,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<TaskBinding, AdapterError> {
    let resolved_filter_uid = filter_user_id
        .map(str::to_string)
        .or_else(|| rule.map(|r| r.effective_user_id().to_string()));
    let filter_user_id = resolved_filter_uid.as_deref().filter(|s| !s.is_empty());

    let destination = provider.binding_destination_from_existing_thread(
        source_id,
        external_thread_id,
        thread_properties,
        filter_user_id,
    )?;

    crate::bindings::runtime::upsert_binding(
        state,
        connection,
        rule,
        task_id,
        provider.connection_provider_key(),
        destination.subject_kind,
        &destination.subject_key,
        destination.destination_config,
        crate::bindings::runtime::ORIGIN_CONNECTED_THREAD,
        destination.last_external_event_id,
    )
    .await
}

/// Resolve the provider connection for a source. Errors when none exists.
async fn resolve_connection(
    state: &AppState,
    provider: &dyn AdapterDestinationBuilder,
    source_id: &str,
    thread_properties: &serde_json::Value,
) -> Result<AdapterConnection, AdapterError> {
    let provider_key = provider.connection_provider_key();
    let scope_value =
        provider.connection_scope_value_from_existing_thread(source_id, thread_properties)?;

    let connection_repo = PgAdapterConnectionDao(state.db.clone());
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
    connection.ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "No {provider_key} provider_connection for scope '{scope_value}'"
        ))
    })
}

/// Resolve the trigger for a source, returning `None` when no trigger matches.
///
/// A trigger belonging to a different connection is treated as absent (and
/// logged) so callers that tolerate a missing trigger still succeed.
async fn resolve_optional_rule(
    state: &AppState,
    provider: &dyn AdapterDestinationBuilder,
    source_id: &str,
    thread_properties: &serde_json::Value,
    connection: &AdapterConnection,
) -> Result<Option<SubscriptionMatch>, AdapterError> {
    let provider_key = provider.connection_provider_key();
    let source_matcher =
        provider.trigger_source_matcher_from_existing_thread(source_id, thread_properties)?;
    let rule_repo = state.loops.clone();
    let mut rule = rule_repo
        .find_subscription_by_source_matcher(provider_key, &source_matcher)
        .await?;
    if rule.is_none() && provider.allow_wildcard_trigger_fallback() {
        rule = rule_repo.find_subscription_wildcard(provider_key).await?;
    }

    if let Some(rule) = &rule {
        if rule.connection_id != connection.id {
            warn!(
                rule_id = %rule.id,
                connection_id = %connection.id,
                "Ignoring task trigger that belongs to a different connection"
            );
            return Ok(None);
        }
    }

    Ok(rule)
}

async fn resolve_connection_and_rule(
    state: &AppState,
    provider: &dyn AdapterDestinationBuilder,
    source_id: &str,
    thread_properties: &serde_json::Value,
) -> Result<(AdapterConnection, SubscriptionMatch), AdapterError> {
    let connection = resolve_connection(state, provider, source_id, thread_properties).await?;

    let provider_key = provider.connection_provider_key();
    let source_matcher =
        provider.trigger_source_matcher_from_existing_thread(source_id, thread_properties)?;
    let rule_repo = state.loops.clone();
    let mut rule = rule_repo
        .find_subscription_by_source_matcher(provider_key, &source_matcher)
        .await?;
    if rule.is_none() && provider.allow_wildcard_trigger_fallback() {
        rule = rule_repo.find_subscription_wildcard(provider_key).await?;
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
) -> Result<(Option<SubscriptionMatch>, TaskBinding), AdapterError> {
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("github_pr".into()))?;
    let provider = providers.destination_builder(AdapterKind::GithubPr)?;
    connect_binding_optional_rule(
        state,
        provider,
        AdapterKind::GithubPr.as_str(),
        source_id,
        task_id,
        "",
        &thread_properties,
        filter_user_id,
    )
    .await
}
