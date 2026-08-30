//! Who a Linear call is made as, and which workspace it reaches.
//!
//! Linear's tokens are per *workspace*, not per user: one OAuth install grants
//! one organization, and every call the Adapter makes on that organization's
//! behalf uses it. So credential selection here is really workspace selection —
//! "which org is this?" — and the answer arrives in a different shape at each
//! call site: an `AppState` in the webhook and worker paths, the Adapter type
//! in the capability impls, a Binding in the activity emitters, and nothing at
//! all in `post_to_destination`.
//!
//! Every one of those shapes resolves to the same three inputs, which is why
//! they belong in one module. They used to be assembled at three call sites
//! from two separate credential sources (defect D4), so "which token does this
//! call use?" was answered by whichever site you happened to be reading — and
//! the two sources were never checked against each other.
//!
//! The [`AppCredentials`] a client is built with is not incidental:
//! [`LinearAgentApi`] reads the workspace token through it on every call and
//! refreshes it in place when it is near expiry, so it is part of the
//! credential, not a convenience.

use moatless_adapter_model::apps::{AdapterAppReader, AdapterAppSecrets};
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::models::{AdapterConnection, TaskBinding};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::LinearConfig;
use std::sync::Arc;

use moatless_backend_core::identity::AppCredentials;

use super::adapter::LinearAdapter;
use super::agent_api::LinearAgentApi;

/// What building a Linear client needs, beyond the organization and the pool.
///
/// A copy of the three `LinearConfig` fields that decide *how* the client
/// authenticates and *where* it points. The Adapter carries one and `AppState`
/// carries the config it came from, so both sides are provably the same
/// credentials rather than two independently-assembled sets.
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
        tokens: Arc<dyn AppCredentials>,
        org_id: &str,
    ) -> LinearAgentApi {
        LinearAgentApi::for_org(
            tokens,
            org_id,
            self.oauth_client_id.as_deref(),
            self.oauth_client_secret.as_deref(),
            self.api_base_url.as_deref(),
        )
    }
}

pub(super) async fn api_for_connection(
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
    Ok(credentials.api_for_org(tokens, &connection.scope_value))
}

/// A client for the workspace a Binding belongs to.
///
/// Returns `None` — rather than an error — when the Binding's Adapter
/// connection has been deleted or cannot be read. Every caller is a
/// best-effort emitter on the event-consumer hot path, where the alternative to
/// skipping is bubbling a database error into event handling.
pub(super) async fn api_for_binding(
    adapter_connections: &dyn AdapterConnectionDao,
    tokens: Arc<dyn AppCredentials>,
    apps: &dyn AdapterAppReader,
    binding: &TaskBinding,
) -> Option<LinearAgentApi> {
    // Column is `external_account_id` on disk; the rest of the codebase reads
    // it via `external_account_id AS scope_value`. Use the DAO so we stay in
    // sync with future schema changes.
    // reason: session routing for an agent-side notification; no caller identity
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let connection = adapter_connections
        .find_by_id(&binding.connection_id, &sys)
        .await;
    match connection {
        Ok(Some(connection)) => match api_for_connection(tokens, apps, &connection).await {
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
/// Takes the Adapter rather than an `AppState` because its one caller —
/// [`AdapterDirectDestination::post_to_destination`] — has neither an
/// `AppState` nor a connection parameter; the destination string names an
/// issue and nothing else. Refusing when more than one workspace is installed
/// is the point: guessing would post a comment into the wrong company's Linear.
///
/// [`AdapterDirectDestination::post_to_destination`]: moatless_loops::adapters::AdapterDirectDestination::post_to_destination
pub(super) async fn api_for_single_workspace(
    adapter: &LinearAdapter,
) -> Result<LinearAgentApi, AdapterError> {
    let rows = adapter
        .require_adapter_connections()?
        .list_by_provider(super::subject::ADAPTER)
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
