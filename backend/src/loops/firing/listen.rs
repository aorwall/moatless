//! Connect tasks to external Adapter subjects.
//!
//! Connections are persisted as canonical task bindings. Shared code resolves
//! connections/Loops and delegates Adapter-specific binding destination
//! construction to the owning Adapter.

use tracing::{info, warn};

use crate::adapters::dao::PgAdapterConnectionDao;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, BindingContext, SubscriptionMatch, TaskBinding};
use crate::adapters::{AdapterDestinationBuilder, AdapterKind};
use crate::state::AppState;

/// Connect a task to an existing Adapter thread by creating a task binding.
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
    let adapter_destination = providers.destination_builder(provider_type)?;

    let (subscription, binding) = connect_binding(
        state,
        adapter_destination,
        provider_type.as_str(),
        source_id,
        task_id,
        external_thread_id,
        &thread_properties,
        filter_user_id,
    )
    .await?;
    let thread = crate::bindings::runtime::binding_context(&binding);
    Ok((subscription.id, thread))
}

#[allow(clippy::too_many_arguments)]
async fn connect_binding(
    state: &AppState,
    adapter_destination: &dyn AdapterDestinationBuilder,
    adapter_label: &str,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(SubscriptionMatch, TaskBinding), AdapterError> {
    let (connection, subscription) = resolve_connection_and_subscription(
        state,
        adapter_destination,
        source_id,
        thread_properties,
    )
    .await?;

    let binding = write_binding(
        state,
        adapter_destination,
        &connection,
        Some(&subscription),
        source_id,
        task_id,
        external_thread_id,
        thread_properties,
        filter_user_id,
    )
    .await?;

    info!(
        subscription_id = %subscription.id,
        task_id,
        binding_id = %binding.id,
        subject_key = %binding.subject_key,
        adapter = adapter_label,
        "Task connected to Adapter binding"
    );

    Ok((subscription, binding))
}

/// Like [`connect_binding`], but does not require a matching Subscription.
///
/// Used for outbound binds — e.g. a PR opened by the agent via
/// `moat gh pr create` — where the connection alone is enough to persist the
/// task↔thread binding so it appears under the task's Connections. Inbound
/// delivery still finds the binding by its subject key
/// (`route_inbound_message` looks up bindings via `find_by_subject`, not by
/// `loop_id`). A Subscription that resolves to a different connection is ignored
/// rather than treated as an error.
#[allow(clippy::too_many_arguments)]
async fn connect_binding_optional_subscription(
    state: &AppState,
    adapter_destination: &dyn AdapterDestinationBuilder,
    adapter_label: &str,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<(Option<SubscriptionMatch>, TaskBinding), AdapterError> {
    let connection =
        resolve_connection(state, adapter_destination, source_id, thread_properties).await?;
    let subscription = resolve_optional_subscription(
        state,
        adapter_destination,
        source_id,
        thread_properties,
        &connection,
    )
    .await?;

    let binding = write_binding(
        state,
        adapter_destination,
        &connection,
        subscription.as_ref(),
        source_id,
        task_id,
        external_thread_id,
        thread_properties,
        filter_user_id,
    )
    .await?;

    info!(
        subscription_id = subscription.as_ref().map_or("<none>", |r| r.id.as_str()),
        task_id,
        binding_id = %binding.id,
        subject_key = %binding.subject_key,
        adapter = adapter_label,
        "Task connected to Adapter binding (outbound, Subscription optional)"
    );

    Ok((subscription, binding))
}

/// Build the Adapter destination and upsert the canonical task binding.
///
/// `subscription` is optional: outbound binds may have no matching Subscription,
/// in which case `upsert_binding` writes a `NULL` `loop_id`.
#[allow(clippy::too_many_arguments)]
async fn write_binding(
    state: &AppState,
    adapter_destination: &dyn AdapterDestinationBuilder,
    connection: &AdapterConnection,
    subscription: Option<&SubscriptionMatch>,
    source_id: &str,
    task_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> Result<TaskBinding, AdapterError> {
    let resolved_filter_uid = filter_user_id
        .map(str::to_string)
        .or_else(|| subscription.map(|s| s.effective_user_id().to_string()));
    let filter_user_id = resolved_filter_uid.as_deref().filter(|s| !s.is_empty());

    let destination = adapter_destination.binding_destination_from_existing_thread(
        source_id,
        external_thread_id,
        thread_properties,
        filter_user_id,
    )?;

    crate::bindings::runtime::upsert_binding(
        state,
        connection,
        subscription,
        task_id,
        adapter_destination.connection_provider_key(),
        destination.subject_kind,
        &destination.subject_key,
        destination.destination_config,
        crate::bindings::runtime::ORIGIN_CONNECTED_THREAD,
        destination.last_external_event_id,
    )
    .await
}

/// Resolve the Adapter connection for a source. Errors when none exists.
async fn resolve_connection(
    state: &AppState,
    adapter_destination: &dyn AdapterDestinationBuilder,
    source_id: &str,
    thread_properties: &serde_json::Value,
) -> Result<AdapterConnection, AdapterError> {
    let adapter_key = adapter_destination.connection_provider_key();
    let scope_value = adapter_destination
        .connection_scope_value_from_existing_thread(source_id, thread_properties)?;

    let connection_repo = PgAdapterConnectionDao(state.db.clone());
    let mut connection = connection_repo
        .find_by_scope_value(adapter_key, &scope_value)
        .await?;
    if connection.is_none()
        && adapter_destination.allow_default_connection_fallback()
        && scope_value != "default"
    {
        connection = connection_repo
            .find_by_scope_value(adapter_key, "default")
            .await?;
    }
    connection.ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "No {adapter_key} Adapter connection for scope '{scope_value}'"
        ))
    })
}

/// Resolve the Subscription for a source, returning `None` when no Subscription matches.
///
/// A Subscription belonging to a different connection is treated as absent (and
/// logged) so callers that tolerate a missing Subscription still succeed.
async fn resolve_optional_subscription(
    state: &AppState,
    adapter_destination: &dyn AdapterDestinationBuilder,
    source_id: &str,
    thread_properties: &serde_json::Value,
    connection: &AdapterConnection,
) -> Result<Option<SubscriptionMatch>, AdapterError> {
    let adapter_key = adapter_destination.connection_provider_key();
    let source_matcher = adapter_destination
        .trigger_source_matcher_from_existing_thread(source_id, thread_properties)?;
    let subscription_repo = state.loops.clone();
    let mut subscription = subscription_repo
        .find_subscription_by_source_matcher(adapter_key, &source_matcher)
        .await?;
    if subscription.is_none() && adapter_destination.allow_wildcard_trigger_fallback() {
        subscription = subscription_repo
            .find_subscription_wildcard(adapter_key)
            .await?;
    }

    if let Some(subscription) = &subscription {
        if subscription.connection_id != connection.id {
            warn!(
                subscription_id = %subscription.id,
                connection_id = %connection.id,
                "Ignoring Subscription that belongs to a different connection"
            );
            return Ok(None);
        }
    }

    Ok(subscription)
}

async fn resolve_connection_and_subscription(
    state: &AppState,
    adapter_destination: &dyn AdapterDestinationBuilder,
    source_id: &str,
    thread_properties: &serde_json::Value,
) -> Result<(AdapterConnection, SubscriptionMatch), AdapterError> {
    let connection =
        resolve_connection(state, adapter_destination, source_id, thread_properties).await?;

    let adapter_key = adapter_destination.connection_provider_key();
    let source_matcher = adapter_destination
        .trigger_source_matcher_from_existing_thread(source_id, thread_properties)?;
    let subscription_repo = state.loops.clone();
    let mut subscription = subscription_repo
        .find_subscription_by_source_matcher(adapter_key, &source_matcher)
        .await?;
    if subscription.is_none() && adapter_destination.allow_wildcard_trigger_fallback() {
        subscription = subscription_repo
            .find_subscription_wildcard(adapter_key)
            .await?;
    }
    let subscription = subscription.ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "No Subscription for {adapter_key} source '{source_matcher}'"
        ))
    })?;

    if subscription.connection_id != connection.id {
        return Err(AdapterError::BadRequest(format!(
            "Subscription {} belongs to a different connection",
            subscription.id
        )));
    }

    Ok((connection, subscription))
}

/// Connect a task to a GitHub PR via the connection/Subscription/binding model.
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
    let adapter_destination = providers.destination_builder(AdapterKind::GithubPr)?;
    connect_binding_optional_subscription(
        state,
        adapter_destination,
        AdapterKind::GithubPr.as_str(),
        source_id,
        task_id,
        "",
        &thread_properties,
        filter_user_id,
    )
    .await
}
