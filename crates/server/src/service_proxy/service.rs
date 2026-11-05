//! Resolving a service proxy request to a policy decision and a credential.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_core::service_proxies::ServiceProxy;
use moatless_ports::identity::SecretResolver;
use moatless_sandbox_control::secrets::{
    resolve_brokered_secret, resolve_task_actor, resolve_workspace_credentials,
};
use moatless_types::http::HttpStreamTimeouts;
use moatless_workspace_ports::{WorkspaceCredentialReader, WorkspaceResolver};
use tracing::warn;

use super::access::ServiceAccess;
use crate::proxy::caller::{SandboxCallers, SandboxScope};

/// Why a request was answered without reaching the upstream. Each message is
/// returned to the tool, so it names the setting that would change it.
#[derive(Debug, thiserror::Error)]
pub enum ServiceProxyError {
    #[error("{0}")]
    Unauthorized(String),
    #[error("No service proxy named `{0}` is defined in SERVICE_PROXIES_JSON")]
    UnknownService(String),
    #[error("{0}")]
    NotEnabled(String),
    #[error("{0}")]
    BadPath(String),
    #[error("{0}")]
    Refused(String),
    #[error("The request body is larger than this service's {0}-byte limit")]
    TooLarge(usize),
    #[error("{0}")]
    NoCredential(String),
    #[error("{0}")]
    Upstream(String),
}

/// Holds the per-service HTTP clients and the ports a request resolves
/// through.
pub struct ServiceProxyGateway {
    pub(super) callers: Arc<SandboxCallers>,
    pub(super) access: Arc<ServiceAccess>,
    workspaces: Arc<dyn WorkspaceResolver>,
    workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
    secrets: Arc<dyn SecretResolver>,
    clients: HashMap<String, reqwest::Client>,
}

/// Redirects are returned to the tool rather than followed, so the credential
/// only ever reaches the definition's own origin.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts the definition holds, and fails as `Client::new` would"
)]
fn http_client(service: &ServiceProxy, connect: std::time::Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .read_timeout(service.limits.idle_timeout())
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(moatless_core::USER_AGENT)
        .build()
        .expect("the TLS backend initialises")
}

impl ServiceProxyGateway {
    pub(crate) fn new(
        callers: Arc<SandboxCallers>,
        access: Arc<ServiceAccess>,
        workspaces: Arc<dyn WorkspaceResolver>,
        workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
        secrets: Arc<dyn SecretResolver>,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        let clients = access
            .services()
            .iter()
            .map(|service| (service.name.clone(), http_client(service, timeouts.connect)))
            .collect();
        Self {
            callers,
            access,
            workspaces,
            workspace_credentials,
            secrets,
            clients,
        }
    }

    pub(super) fn client(&self, service: &str) -> Option<&reqwest::Client> {
        self.clients.get(service)
    }

    /// Refuses `service` unless `scope`'s Workspace has it on.
    pub(super) async fn check_enabled(
        &self,
        scope: &SandboxScope,
        service: &ServiceProxy,
    ) -> Result<(), ServiceProxyError> {
        let enabled = self.access.enabled(&scope.workspace).await.map_err(|e| {
            warn!(error = %e, workspace_id = %scope.workspace, "Failed to read a workspace's service proxies");
            ServiceProxyError::Upstream("Failed to read the workspace".to_string())
        })?;
        if enabled.contains(&service.name) {
            return Ok(());
        }
        Err(ServiceProxyError::NotEnabled(format!(
            "Workspace {} does not use the {name} service proxy: list \"{name}\" in `services` \
             in its .moatless/workspaces.json entry, and an administrator must add it to \
             `services` in this workspace's WORKSPACE_IDENTITY_GRANTS entry",
            scope.workspace,
            name = service.name,
        )))
    }

    /// The secret `service` presents for `scope`'s run-as identity.
    pub(super) async fn credential(
        &self,
        scope: &SandboxScope,
        service: &ServiceProxy,
    ) -> Result<String, ServiceProxyError> {
        // An unreadable pin fails the request rather than falling back to the
        // actor's own secret, as `GetCredentials` refuses to.
        let pinned = resolve_workspace_credentials(
            Some(&self.workspace_credentials),
            Some(&scope.workspace),
        )
        .await
        .map_err(|e| {
            warn!(error = %e, workspace_id = %scope.workspace, "Failed to read the workspace credentials");
            ServiceProxyError::Upstream("Failed to read the workspace credentials".to_string())
        })?;
        let actor = resolve_task_actor(
            Some(&self.workspaces),
            Some(&scope.workspace),
            scope.run_as.as_deref(),
            &scope.user,
        )
        .await;
        let key = &service.credential.secret;
        resolve_brokered_secret(&actor, key, Some(&self.secrets), &pinned)
            .await
            .ok_or_else(|| {
                ServiceProxyError::NoCredential(format!(
                    "No {key} secret resolves for the {} service proxy: workspace {} pins none, \
                     and Moatless user {} has no user or global secret of that name",
                    service.name,
                    scope.workspace,
                    actor.user_id(),
                ))
            })
    }
}
