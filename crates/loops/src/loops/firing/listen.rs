//! Connect tasks to external Adapter subjects.
//!
//! Connections are persisted as canonical task bindings. Shared code resolves
//! connections/Loops and delegates Adapter-specific binding destination
//! construction to the owning Adapter.

use std::sync::Arc;

use tracing::{info, warn};

use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::AdapterConnection;
use moatless_types::loops::SubscriptionMatch;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::AdapterKind;
use moatless_adapter_model::capabilities::AdapterDestinationBuilder;
use moatless_adapter_model::loops::LoopReader;
use crate::bindings::dao::TaskBindingDao;

/// Wires a task to an external Adapter subject: resolves the connection and
/// (optionally) the Subscription, then upserts the canonical task binding.
///
/// Holds the four ports this module reads as typed fields — the GitHub PR
/// destination builder (`github_destination`), the adapter-connection DAO
/// (`adapter_connections`), the Subscription reads (`loops`), and the
/// task-binding DAO (`task_bindings`). Read ports through those fields.
///
/// `github_destination` is the *concrete* GitHub PR
/// [`AdapterDestinationBuilder`], not the adapter registry. A registry field
/// here would be an upward edge in disguise: this module only ever wants the
/// one builder named by `AdapterKind::GithubPr`, and reaching it through the
/// registry (`adapters.destination_builder(GithubPr)`) is a provider self-lookup
/// laundered through the band-8 aggregate — the caller supplies the concrete
/// builder instead, so nothing below the provider band names a specific kind to
/// fetch a provider.
///
/// [`Connector::write_binding`] upserts through the port-typed
/// [`crate::bindings::runtime::upsert_binding_into`], passing `task_bindings`
/// directly rather than the whole `AppState`.
pub struct Connector {
    github_destination: Option<Arc<dyn AdapterDestinationBuilder>>,
    adapter_connections: Arc<dyn AdapterConnectionDao>,
    loops: Arc<dyn LoopReader>,
    task_bindings: Arc<dyn TaskBindingDao>,
}

impl Connector {
    pub fn new(
        github_destination: Option<Arc<dyn AdapterDestinationBuilder>>,
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<dyn LoopReader>,
        task_bindings: Arc<dyn TaskBindingDao>,
    ) -> Self {
        Self {
            github_destination,
            adapter_connections,
            loops,
            task_bindings,
        }
    }

    /// Resolve the connection and its optional Subscription, then upsert the
    /// binding — a missing or mismatched Subscription is tolerated rather than
    /// treated as an error.
    ///
    /// Used for outbound binds — e.g. a PR opened by the agent via
    /// `moat gh pr create` — where the connection alone is enough to persist the
    /// task↔thread binding so it appears under the task's Connections. Inbound
    /// delivery still finds the binding by its subject key
    /// (`route_inbound_message` looks up bindings via `find_by_subject`, not by
    /// `loop_id`). A Subscription that resolves to a different connection is
    /// ignored rather than treated as an error.
    #[expect(
        clippy::too_many_arguments,
        reason = "eight against the threshold of seven: the &self receiver plus seven \
                  data parameters, no ports. adapter_label and task_id are read for \
                  the completion log; adapter_destination, source_id, \
                  external_thread_id, thread_properties, and filter_user_id are \
                  forwarded to resolve_connection, resolve_optional_subscription, and \
                  write_binding. The three ports live on Connector's fields, so this \
                  width is the binding's own identity threaded to its three helpers, \
                  with no frame above connect_github_binding to hoist a data argument \
                  to."
    )]
    async fn connect_binding_optional_subscription(
        &self,
        adapter_destination: &dyn AdapterDestinationBuilder,
        adapter_label: &str,
        source_id: &str,
        task_id: &str,
        external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<(Option<SubscriptionMatch>, TaskBinding), AdapterError> {
        let connection = self
            .resolve_connection(adapter_destination, source_id, thread_properties)
            .await?;
        let subscription = self
            .resolve_optional_subscription(
                adapter_destination,
                source_id,
                thread_properties,
                &connection,
            )
            .await?;

        let binding = self
            .write_binding(
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
    /// `subscription` is optional: outbound binds may have no matching
    /// Subscription, in which case `upsert_binding` writes a `NULL` `loop_id`.
    #[expect(
        clippy::too_many_arguments,
        reason = "nine against the threshold of seven: the &self receiver plus eight \
                  data parameters, no ports. This method builds one binding \
                  destination and upserts it. adapter_destination and filter_user_id \
                  are read (the destination builder; the effective-user resolution at \
                  the top); subscription is read for its effective user and then \
                  forwarded; connection, source_id, task_id, external_thread_id, and \
                  thread_properties are forwarded to the destination builder or to \
                  bindings::runtime::upsert_binding_into. The four ports this file \
                  reads now live on Connector's fields, so the width here is the \
                  binding's own identity, with no frame above to hoist a data argument \
                  to."
    )]
    async fn write_binding(
        &self,
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

        crate::bindings::runtime::upsert_binding_into(
            self.task_bindings.as_ref(),
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
        &self,
        adapter_destination: &dyn AdapterDestinationBuilder,
        source_id: &str,
        thread_properties: &serde_json::Value,
    ) -> Result<AdapterConnection, AdapterError> {
        let adapter_key = adapter_destination.connection_provider_key();
        let scope_value = adapter_destination
            .connection_scope_value_from_existing_thread(source_id, thread_properties)?;

        let connection_repo = self.adapter_connections.clone();
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
            SharedAdapterError::BadRequest(format!(
                "No {adapter_key} Adapter connection for scope '{scope_value}'"
            ))
            .into()
        })
    }

    /// Resolve the Subscription for a source, returning `None` when no
    /// Subscription matches.
    ///
    /// A Subscription belonging to a different connection is treated as absent
    /// (and logged) so callers that tolerate a missing Subscription still
    /// succeed.
    async fn resolve_optional_subscription(
        &self,
        adapter_destination: &dyn AdapterDestinationBuilder,
        source_id: &str,
        thread_properties: &serde_json::Value,
        connection: &AdapterConnection,
    ) -> Result<Option<SubscriptionMatch>, AdapterError> {
        let adapter_key = adapter_destination.connection_provider_key();
        let source_matcher = adapter_destination
            .trigger_source_matcher_from_existing_thread(source_id, thread_properties)?;
        let subscription_repo = self.loops.clone();
        let mut subscription = subscription_repo
            .subscription_for_source(adapter_key, &source_matcher)
            .await?;
        if subscription.is_none() && adapter_destination.allow_wildcard_trigger_fallback() {
            subscription = subscription_repo
                .wildcard_subscription(adapter_key)
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

    /// Connect a task to a GitHub PR via the connection/Subscription/binding
    /// model.
    ///
    /// Compatibility wrapper for callers that already parse GitHub PR details.
    pub async fn connect_github_binding(
        &self,
        source_id: &str,
        task_id: &str,
        thread_properties: serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<(Option<SubscriptionMatch>, TaskBinding), AdapterError> {
        let adapter_destination = self
            .github_destination
            .as_deref()
            .ok_or_else(|| SharedAdapterError::AdapterNotConfigured("github_pr".into()))?;
        self.connect_binding_optional_subscription(
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
}
