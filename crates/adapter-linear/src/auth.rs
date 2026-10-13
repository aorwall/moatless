//! Who a Linear call is made as, and which workspace it reaches.
//!
//! Linear's tokens are per *workspace*, not per user: one OAuth install grants
//! one organization, and every call the Adapter makes on its behalf uses it.
//! Credential selection here is really workspace selection, arriving in a
//! different shape at each call site — hence one module.
//!
//! The [`AppCredentials`] a client is built with matters: [`LinearAgentApi`]
//! refreshes the workspace token through it on every call.

use moatless_adapter_model::apps::{AdapterAppReader, AdapterAppSecrets};
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::connections::resolve_connection;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::LinearConfig;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{AdapterConnection, AdapterKind};
use moatless_types::http::HttpTimeouts;
use std::sync::Arc;

use moatless_ports::identity::AppCredentials;

use super::adapter::LinearAdapter;
use super::agent_api::LinearAgentApi;

/// What building a Linear client needs, beyond the organization and the pool.
///
/// A copy of the three `LinearConfig` fields deciding how the client
/// authenticates and where it points; the Adapter and `AppState` each carry
/// one, so both sides are provably the same credentials.
#[derive(Clone, Debug)]
pub(super) struct LinearCredentials {
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    /// `None` means Linear itself. A test points this at a loopback stub.
    api_base_url: Option<String>,
}

impl From<&LinearConfig> for LinearCredentials {
    fn from(config: &LinearConfig) -> Self {
        Self {
            oauth_client_id: config.oauth_client_id.clone(),
            oauth_client_secret: config.oauth_client_secret.clone(),
            api_base_url: config.api_base_url.clone(),
        }
    }
}

/// A stored Linear app, in the shape this Adapter authenticates with. Every
/// field is optional: an install with no OAuth pair still reaches Linear with
/// the workspace token [`AppCredentials`] holds.
impl From<AdapterAppSecrets> for LinearCredentials {
    fn from(app: AdapterAppSecrets) -> Self {
        Self {
            oauth_client_id: app.secret("oauth_client_id"),
            oauth_client_secret: app.secret("oauth_client_secret"),
            api_base_url: app.setting("api_base_url"),
        }
    }
}

impl LinearCredentials {
    /// A client for `org_id`. The one place any of these fields is read.
    pub(super) fn api_for_org(
        &self,
        timeouts: HttpTimeouts,
        tokens: Arc<dyn AppCredentials>,
        org_id: &str,
    ) -> LinearAgentApi {
        LinearAgentApi::for_org(
            timeouts,
            tokens,
            org_id,
            self.oauth_client_id.as_deref(),
            self.oauth_client_secret.as_deref(),
            self.api_base_url.as_deref(),
        )
    }
}

pub(super) async fn api_for_connection(
    timeouts: HttpTimeouts,
    tokens: Arc<dyn AppCredentials>,
    apps: &dyn AdapterAppReader,
    connection: &AdapterConnection,
) -> Result<LinearAgentApi, AdapterError> {
    let app_key =
        moatless_adapter_model::apps::app_key_from_transport_config(&connection.transport_config);
    let Some(app) = apps.app("linear", app_key).await? else {
        return Err(
            SharedAdapterError::AdapterNotConfigured(format!("linear app '{app_key}'")).into(),
        );
    };
    let credentials = LinearCredentials::from(app);
    Ok(credentials.api_for_org(timeouts, tokens, &connection.scope_value))
}

/// A client for the workspace a Binding belongs to.
///
/// Returns `None` rather than an error when the Binding's connection has been
/// deleted or cannot be read — every caller is a best-effort emitter on the
/// event-consumer hot path, where the alternative is bubbling a database error
/// into event handling.
pub(super) async fn api_for_binding(
    timeouts: HttpTimeouts,
    adapter_connections: &dyn ConnectionLookup,
    tokens: Arc<dyn AppCredentials>,
    apps: &dyn AdapterAppReader,
    binding: &TaskBinding,
) -> Option<LinearAgentApi> {
    let connection = adapter_connections
        .by_id_trusted(&binding.connection_id)
        .await;
    match connection {
        Ok(Some(connection)) => match api_for_connection(timeouts, tokens, apps, &connection).await
        {
            Ok(api) => Some(api),
            Err(e) => {
                tracing::warn!(
                    task_id = %binding.task_id,
                    connection_id = %binding.connection_id,
                    error = %e,
                    "linear auth: app config lookup failed"
                );
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(
                task_id = %binding.task_id,
                connection_id = %binding.connection_id,
                error = %e,
                "linear auth: connection lookup failed"
            );
            None
        }
    }
}

/// A client for the single installed Linear workspace.
///
/// Takes the Adapter rather than an `AppState` since its one caller has neither
/// an `AppState` nor a connection parameter; refusing when more than one
/// workspace is installed is the point — guessing would post to the wrong
/// company's Linear.
pub(super) async fn api_for_single_workspace(
    adapter: &LinearAdapter,
) -> Result<LinearAgentApi, AdapterError> {
    let rows = adapter
        .require_adapter_connections()?
        .by_provider(super::subject::ADAPTER)
        .await
        .map_err(SharedAdapterError::Store)?;
    match rows.len() {
        0 => Err(SharedAdapterError::AdapterNotConfigured(
            "No Linear workspace installed — run the Linear OAuth install first".into(),
        )
        .into()),
        1 => {
            adapter
                .agent_api_for_transport(&rows[0].scope_value, &rows[0].transport_config)
                .await
        }
        _ => Err(SharedAdapterError::BadRequest(
            "Multiple Linear workspaces installed — use the /api/v1/linear/* \
             endpoints with an explicit connectionId"
                .into(),
        )
        .into()),
    }
}

/// Unwrap the composition-root-injected Linear adapter, or fail the request
/// with the not-configured error.
pub(super) fn require_linear(
    linear: Option<Arc<LinearAdapter>>,
) -> Result<Arc<LinearAdapter>, AdapterError> {
    linear.ok_or_else(|| SharedAdapterError::AdapterNotConfigured("linear".into()).into())
}

/// Resolve the caller's Linear connection (default: the one accessible
/// connection) and build a [`LinearAgentApi`] for it.
///
/// Takes the adapter directly rather than via `state.provider_handles.linear`:
/// self-resolution through the registry would dissolve once the route already
/// extracted `State<Option<Arc<LinearAdapter>>>`.
pub(super) async fn resolve_linear_api(
    adapter: &LinearAdapter,
    ctx: &ScopeContext,
    connection_id: Option<&str>,
) -> Result<(AdapterConnection, LinearAgentApi), AdapterError> {
    let connection = resolve_connection(
        adapter.require_connection_reader()?,
        ctx,
        AdapterKind::Linear,
        connection_id,
    )
    .await?;
    let api = adapter.agent_api_for_connection(&connection).await?;
    Ok((connection, api))
}
