//! Authenticating a pod's task token and reading what the gateway resolves a
//! model credential from.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_core::config::ModelGatewayConfig;
use moatless_core::sandbox_jwt::{
    GenerationError, TokenGenerations, check_generation, verify_user_auth_token,
};
use moatless_ports::identity::{HarnessCredentials, SecretResolver};
use moatless_sandbox_control::secrets::{
    resolve_task_actor, resolve_task_anthropic_credentials, resolve_workspace_credentials,
    resolve_workspace_env_vars,
};
use moatless_types::http::HttpStreamTimeouts;
use moatless_workspace_ports::{WorkspaceCredentialReader, WorkspaceResolver};
use tracing::warn;

use crate::sandbox::lifecycle::SandboxIngress;

/// Anthropic's API root, which a task's own Anthropic credential is sent to.
pub const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";

/// The sandbox a verified task token was minted for.
#[derive(Debug, Clone)]
pub struct SandboxScope {
    pub sandbox: String,
    /// The Sandbox's driver, else its owner, which a run-as identity
    /// overrides.
    pub user: String,
    pub workspace: String,
    /// The Sandbox row's run-as identity, ahead of the Workspace's.
    pub run_as: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// Every rejection of the caller's own task token.
    #[error("{0}")]
    Unauthorized(String),
    /// The task resolved, but holds no credential this gateway may use.
    #[error("{0}")]
    NoCredential(String),
    /// A store or the upstream failed; the request may succeed later.
    #[error("{0}")]
    Upstream(String),
}

/// Resolves a pod's model request to an upstream and the credential to
/// present there.
pub struct ModelGateway {
    sandboxes: Arc<SandboxIngress>,
    token_generations: Arc<dyn TokenGenerations>,
    workspaces: Arc<dyn WorkspaceResolver>,
    workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
    secrets: Arc<dyn SecretResolver>,
    harness_credentials: Arc<dyn HarnessCredentials>,
    /// `None` in a deployment that mints no task tokens, which refuses every
    /// request rather than admitting an unverifiable one.
    task_token_secret: Option<String>,
    pub(super) config: ModelGatewayConfig,
    /// Anthropic's root, without a trailing `/`; a test points it at a fake.
    pub(super) anthropic_base: String,
    pub(super) http: reqwest::Client,
}

/// Idle, not total: a turn streams for as long as the model writes.
///
/// Redirects are returned rather than followed, so a credential only ever
/// reaches the upstream it was resolved for.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
fn http_client(timeouts: HttpStreamTimeouts) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .read_timeout(timeouts.idle)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(moatless_core::USER_AGENT)
        .build()
        .expect("the TLS backend initialises")
}

impl ModelGateway {
    #[expect(
        clippy::too_many_arguments,
        reason = "one port per lookup the credential resolution makes"
    )]
    pub(crate) fn new(
        sandboxes: Arc<SandboxIngress>,
        token_generations: Arc<dyn TokenGenerations>,
        workspaces: Arc<dyn WorkspaceResolver>,
        workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
        secrets: Arc<dyn SecretResolver>,
        harness_credentials: Arc<dyn HarnessCredentials>,
        task_token_secret: Option<String>,
        config: ModelGatewayConfig,
        anthropic_base: &str,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        Self {
            sandboxes,
            token_generations,
            workspaces,
            workspace_credentials,
            secrets,
            harness_credentials,
            task_token_secret,
            config,
            anthropic_base: anthropic_base.trim_end_matches('/').to_string(),
            http: http_client(timeouts),
        }
    }

    /// Verify `task_token` and read the sandbox it was minted for.
    pub async fn authenticate(
        &self,
        task_token: Option<&str>,
    ) -> Result<SandboxScope, GatewayError> {
        let task_token = task_token
            .ok_or_else(|| GatewayError::Unauthorized("No task token was presented".to_string()))?;
        let secret = self.task_token_secret.as_deref().ok_or_else(|| {
            GatewayError::Unauthorized(
                "This deployment verifies no task tokens, so it serves no model gateway"
                    .to_string(),
            )
        })?;
        let claims = verify_user_auth_token(task_token, secret)
            .map_err(|e| GatewayError::Unauthorized(format!("Invalid task token: {e}")))?;
        let sandbox_id = claims.sandbox_id.clone().ok_or_else(|| {
            GatewayError::Unauthorized("The task token names no sandbox".to_string())
        })?;
        check_generation(
            self.token_generations.as_ref(),
            &sandbox_id,
            claims.generation,
        )
        .await
        .map_err(|e| match e {
            GenerationError::Stale { .. } | GenerationError::NoSandbox => {
                GatewayError::Unauthorized(e.to_string())
            }
            GenerationError::Lookup(_) => GatewayError::Upstream(e.to_string()),
        })?;

        let workspace_id = self
            .sandboxes
            .workspace_id(&sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox for a model request");
                GatewayError::Upstream("Failed to read the sandbox".to_string())
            })?
            .ok_or_else(|| {
                GatewayError::Unauthorized(
                    "The task token names a sandbox that no longer exists".to_string(),
                )
            })?;
        // Fails closed rather than degrading to the token's user: that is the
        // driver only while the read agrees.
        let identity = self
            .sandboxes
            .identity(&sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox identity");
                GatewayError::Upstream("Failed to read the sandbox".to_string())
            })?
            .unwrap_or_default();
        Ok(SandboxScope {
            sandbox: sandbox_id,
            user: identity.acting_user_id.unwrap_or(claims.user_id),
            workspace: workspace_id,
            run_as: identity.pinned_run_as,
        })
    }

    /// The Anthropic credentials `GetCredentials` would deliver to `scope`'s
    /// pod.
    pub async fn anthropic_credentials(
        &self,
        scope: &SandboxScope,
    ) -> Result<HashMap<String, String>, GatewayError> {
        // An unreadable pin fails the request rather than falling back to the
        // actor's own credential, as `GetCredentials` refuses to.
        let pinned = resolve_workspace_credentials(
            Some(&self.workspace_credentials),
            Some(&scope.workspace),
        )
        .await
        .map_err(|e| {
            warn!(error = %e, workspace_id = %scope.workspace, "Failed to read the workspace credentials");
            GatewayError::Upstream("Failed to read the workspace credentials".to_string())
        })?;
        let env_vars =
            resolve_workspace_env_vars(Some(&self.workspaces), Some(&scope.workspace)).await;
        let actor = resolve_task_actor(
            Some(&self.workspaces),
            Some(&scope.workspace),
            scope.run_as.as_deref(),
            &scope.user,
        )
        .await;
        Ok(resolve_task_anthropic_credentials(
            &actor,
            Some(&self.secrets),
            Some(&self.harness_credentials),
            pinned,
            env_vars,
        )
        .await)
    }
}
