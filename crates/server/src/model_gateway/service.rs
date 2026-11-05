//! Authenticating a pod's task token and reading what the gateway resolves a
//! model credential from.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_core::config::ModelGatewayConfig;
use moatless_core::secret_keys::CODEX_AUTH_JSON;
use moatless_ports::identity::{HarnessCredentials, SecretResolver};
use moatless_ports::sandbox::CodexCredentials;
use moatless_sandbox_control::secrets::{
    resolve_task_actor, resolve_task_anthropic_credentials, resolve_task_openai_credentials,
    resolve_workspace_credentials, resolve_workspace_env_vars,
};
use moatless_types::access_control::Actor;
use moatless_types::http::HttpStreamTimeouts;
use moatless_workspace_ports::{WorkspaceCredentialReader, WorkspaceResolver};
use tracing::warn;

use super::openai;
pub use crate::proxy::caller::SandboxScope;
use crate::proxy::caller::{CallerError, SandboxCallers};

/// Anthropic's API root, which a task's own Anthropic credential is sent to.
pub const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";

/// The OpenAI API root, which a task's own OpenAI API key is sent to.
pub const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

/// The root Codex itself sends a ChatGPT login's requests to.
pub const CHATGPT_CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex";

/// The model roots a [`ModelGateway`] presents a task's own credential to,
/// each without a trailing `/`; a test points them at a fake.
#[derive(Debug, Clone)]
pub struct ProviderBases {
    pub anthropic: String,
    pub openai: String,
    pub chatgpt: String,
}

impl Default for ProviderBases {
    fn default() -> Self {
        Self {
            anthropic: ANTHROPIC_API_BASE.to_string(),
            openai: OPENAI_API_BASE.to_string(),
            chatgpt: CHATGPT_CODEX_BASE.to_string(),
        }
    }
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
    callers: Arc<SandboxCallers>,
    workspaces: Arc<dyn WorkspaceResolver>,
    workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
    secrets: Arc<dyn SecretResolver>,
    harness_credentials: Arc<dyn HarnessCredentials>,
    codex: Arc<dyn CodexCredentials>,
    pub(super) config: ModelGatewayConfig,
    pub(super) bases: ProviderBases,
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
        callers: Arc<SandboxCallers>,
        workspaces: Arc<dyn WorkspaceResolver>,
        workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
        secrets: Arc<dyn SecretResolver>,
        harness_credentials: Arc<dyn HarnessCredentials>,
        codex: Arc<dyn CodexCredentials>,
        config: ModelGatewayConfig,
        bases: ProviderBases,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        Self {
            callers,
            workspaces,
            workspace_credentials,
            secrets,
            harness_credentials,
            codex,
            config,
            bases,
            http: http_client(timeouts),
        }
    }

    /// Verify `task_token` and read the sandbox it was minted for.
    pub async fn authenticate(
        &self,
        task_token: Option<&str>,
    ) -> Result<SandboxScope, GatewayError> {
        self.callers
            .authenticate(task_token)
            .await
            .map_err(|e| match e {
                CallerError::Unauthorized(_) | CallerError::SandboxGone => {
                    GatewayError::Unauthorized(e.to_string())
                }
                CallerError::Upstream(message) => GatewayError::Upstream(message),
            })
    }

    /// The Anthropic credentials `scope`'s actor calls upstream with.
    pub(super) async fn anthropic_credentials(
        &self,
        scope: &SandboxScope,
        mode: super::auth_mode::AuthMode,
    ) -> Result<HashMap<String, String>, GatewayError> {
        let inputs = self.task_inputs(scope).await?;
        Ok(resolve_task_anthropic_credentials(
            &inputs.actor,
            Some(&self.secrets),
            (mode == super::auth_mode::AuthMode::Default).then_some(&self.harness_credentials),
            inputs.pinned,
            inputs.env_vars,
        )
        .await)
    }

    /// The OpenAI credentials `scope`'s actor calls upstream with, its Codex
    /// login refreshed only when that login is selected for the request.
    pub(super) async fn openai_credentials(
        &self,
        scope: &SandboxScope,
        mode: super::auth_mode::AuthMode,
    ) -> Result<HashMap<String, String>, GatewayError> {
        let inputs = self.task_inputs(scope).await?;
        let login_pinned = inputs.pinned.contains_key(CODEX_AUTH_JSON)
            || inputs.env_vars.contains_key(CODEX_AUTH_JSON);
        let user_id = inputs.actor.user_id().to_string();
        let mut credentials = resolve_task_openai_credentials(
            &inputs.actor,
            Some(&self.secrets),
            Some(&self.harness_credentials),
            inputs.pinned,
            inputs.env_vars,
        )
        .await;
        if mode == super::auth_mode::AuthMode::Default
            && !login_pinned
            && openai::reads_codex_login(&credentials, &self.bases)
        {
            match self.codex.ensure_fresh_codex_for(&user_id).await {
                Ok(Some(blob)) => {
                    credentials.insert(CODEX_AUTH_JSON.to_string(), blob);
                }
                Ok(None) => {}
                Err(e) => warn!(error = %e, user_id, "ensure_fresh_codex_for failed"),
            }
        }
        Ok(credentials)
    }

    async fn task_inputs(&self, scope: &SandboxScope) -> Result<TaskInputs, GatewayError> {
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
        Ok(TaskInputs {
            actor,
            pinned,
            env_vars,
        })
    }
}

/// What a task's credentials resolve from, read once per request.
struct TaskInputs {
    actor: Actor,
    pinned: HashMap<String, String>,
    env_vars: HashMap<String, String>,
}
