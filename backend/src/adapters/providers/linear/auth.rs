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
//! The `db` a client is built with is not incidental: [`LinearAgentApi`] reads
//! the workspace token out of `provider_tokens` on every call and refreshes it
//! in place when it is near expiry, so the pool is part of the credential, not
//! a convenience.

use crate::adapters::error::AdapterError;
use crate::adapters::models::TaskBinding;
use crate::config::LinearConfig;
use crate::db::DbPool;
use crate::state::AppState;
use moatless_backend_core::scope::ScopeContext;

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

impl LinearCredentials {
    /// A client for `org_id`. The one place any of these fields is read.
    pub(super) fn api_for_org(&self, db: DbPool, org_id: &str) -> LinearAgentApi {
        LinearAgentApi::for_org(
            db,
            org_id,
            self.oauth_client_id.as_deref(),
            self.oauth_client_secret.as_deref(),
            self.api_base_url.as_deref(),
        )
    }
}

/// A client for `org_id`, from application state.
///
/// Used by the webhook dispatcher and by every delivery handler: both hold an
/// `AppState` and neither has a [`LinearAdapter`].
pub(super) fn api_for_org(state: &AppState, org_id: &str) -> LinearAgentApi {
    LinearCredentials::from(&state.config.linear).api_for_org(state.db.clone(), org_id)
}

/// A client for the workspace a Binding belongs to.
///
/// Returns `None` — rather than an error — when the Binding's Adapter
/// connection has been deleted or cannot be read. Every caller is a
/// best-effort emitter on the event-consumer hot path, where the alternative to
/// skipping is bubbling a database error into event handling.
pub(super) async fn api_for_binding(
    state: &AppState,
    binding: &TaskBinding,
) -> Option<LinearAgentApi> {
    // Column is `external_account_id` on disk; the rest of the codebase reads
    // it via `external_account_id AS scope_value`. Use the DAO so we stay in
    // sync with future schema changes.
    // reason: session routing for an agent-side notification; no caller identity
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let connection = crate::adapters::dao::PgAdapterConnectionDao(state.db.clone())
        .find_by_id(&binding.connection_id, &sys)
        .await;
    match connection {
        Ok(Some(connection)) => Some(api_for_org(state, &connection.scope_value)),
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
/// [`AdapterDirectDestination::post_to_destination`]: crate::adapters::AdapterDirectDestination::post_to_destination
pub(super) async fn api_for_single_workspace(
    adapter: &LinearAdapter,
) -> Result<LinearAgentApi, AdapterError> {
    let db = adapter.db();
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT scope_value FROM integration_connections \
         WHERE provider = 'linear' LIMIT 2",
    )
    .fetch_all(db)
    .await
    .map_err(AdapterError::Db)?;
    match rows.len() {
        0 => Err(AdapterError::AdapterNotConfigured(
            "No Linear workspace installed — run the Linear OAuth install first".into(),
        )),
        1 => Ok(adapter.agent_api_for_org(&rows[0])),
        _ => Err(AdapterError::BadRequest(
            "Multiple Linear workspaces installed — use the /api/v1/linear/* \
             endpoints with an explicit connectionId"
                .into(),
        )),
    }
}

/// The Moatless user behind a Linear login, if that person has connected their
/// own Linear account.
///
/// Distinct from everything above: those answer "which workspace token do we
/// call as", this answers "who is the human that did this". A Linear actor with
/// no `provider_tokens` row is an ordinary outcome — most issue commenters have
/// never signed into Moatless — so `None` is not an error.
pub(super) async fn resolve_user(
    identity: &str,
    db: &DbPool,
) -> Result<Option<String>, AdapterError> {
    sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM provider_tokens WHERE provider = 'linear' AND provider_login = $1 LIMIT 1",
    )
    .bind(identity)
    .fetch_optional(db)
    .await
    .map_err(AdapterError::Db)
}
