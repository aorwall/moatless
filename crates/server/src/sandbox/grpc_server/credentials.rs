//! The credentials a sandbox is issued, and the one kind it writes back.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

use sandbox_proto::{
    GetCredentialsRequest, GetCredentialsResponse, UpdateCredentialRequest,
    UpdateCredentialResponse,
};

use crate::sandbox::secrets::{
    TaskCredentialInputs, resolve_task_actor, resolve_task_credential_targets,
    resolve_task_credentials, resolve_task_ssh_keys, resolve_workspace_credentials,
    resolve_workspace_env_vars,
};
use moatless_core::sandbox_jwt as jwt;
use moatless_core::secret_keys;
use moatless_identity::credentials::WriteSource;
use moatless_ports::sandbox::CodexCredentials;
use moatless_types::credential_key;

use super::authorize::TaskGrant;
use super::state::BackendSandboxServiceImpl;

/// Known credential env var names, used as the map keys in
/// `GetCredentialsResponse`.
pub(crate) struct CredentialKey;

impl CredentialKey {
    /// Re-minted every fetch, so the sandbox's 15-minute refresh loop
    /// propagates a rotation.
    pub const USER_AUTH_TOKEN: &str = credential_key::USER_AUTH_TOKEN;
    pub const GITHUB_TOKEN: &str = secret_keys::GITHUB_TOKEN;
    pub const GITLAB_TOKEN: &str = secret_keys::GITLAB_TOKEN;
    pub const CODEX_AUTH_JSON: &str = secret_keys::CODEX_AUTH_JSON;
    pub const OPENAI_API_KEY: &str = secret_keys::OPENAI_API_KEY;
    pub const ANTHROPIC_API_KEY: &str = secret_keys::ANTHROPIC_API_KEY;
    pub const CLAUDE_CODE_OAUTH_TOKEN: &str = secret_keys::CLAUDE_CODE_OAUTH_TOKEN;
}

/// The provider a sandbox may write a rotated credential back to, if any.
///
/// `None` refuses the write: only an OAuth-style rotating credential has a
/// write-back path; a static API key is backend-managed. Distinct from
/// `credential_key_to_provider`, which names every key a person may write.
fn sandbox_rotatable_provider(key: &str) -> Option<&'static str> {
    match key {
        CredentialKey::CODEX_AUTH_JSON => Some("codex"),
        _ => None,
    }
}

impl BackendSandboxServiceImpl {
    /// Mint the long-lived user auth token delivered over `GetCredentials`.
    ///
    /// Bound to the grant's Sandbox at the generation the presented token
    /// carries, which the grant has just checked is current; returns `None`
    /// when the secret is unset, so the sandbox keeps its existing token.
    fn mint_user_auth_token(&self, grant: &TaskGrant) -> Option<String> {
        let secret = self.task_token_secret.as_ref()?;
        let ttl = self.backend_config.sandbox.user_token_ttl_secs;
        let user_id = grant.ctx.user_id.as_str();
        match jwt::generate_user_auth_token(
            user_id,
            &grant.login,
            &grant.sandbox_id,
            grant.token_generation,
            secret,
            ttl,
        ) {
            Ok(token) => Some(token),
            Err(e) => {
                warn!(error = %e, user_id, "Failed to mint user auth token for credentials");
                None
            }
        }
    }
}

/// Return credentials for sandbox initialization.
///
/// Resolution chain, most-specific first: Workspace-pinned credentials, then
/// user-level `provider_tokens`, then scoped global/user secrets.
#[tracing::instrument(
    skip_all,
    fields(
        request_id = %uuid::Uuid::new_v4(),
        task_id = tracing::field::Empty,
        user_id = tracing::field::Empty
    )
)]
pub(super) async fn get_credentials(
    service: &BackendSandboxServiceImpl,
    request: Request<GetCredentialsRequest>,
) -> Result<Response<GetCredentialsResponse>, Status> {
    // Both the bootstrap token and the long-lived user token are accepted here.
    let task_id = request.get_ref().task_id.clone();
    let grant = service
        .authorizer
        .authorize_bootstrap(&request, &task_id)
        .await?;
    let req = request.into_inner();
    tracing::Span::current().record("task_id", req.task_id.as_str());
    tracing::Span::current().record("user_id", grant.ctx.user_id.as_str());

    debug!(
        task_id = %req.task_id,
        user_id = %grant.ctx.user_id,
        "GetCredentials request"
    );

    let handlers = service.handlers.as_ref();
    let targets = resolve_task_credential_targets(
        handlers.map(|h| &h.sandboxes),
        handlers.map(|h| &h.workspace_resolver),
        &req.task_id,
    )
    .await;
    // A workspace credential that cannot be read must not degrade into the
    // run-as user's own token: the sandbox would come up authenticated as the
    // wrong account. Failing the fetch leaves the pod retrying instead.
    let workspace_credentials = resolve_workspace_credentials(
        handlers.map(|h| &h.workspace_credentials),
        targets.workspace_id.as_deref(),
    )
    .await
    .map_err(|e| {
        warn!(
            error = %e,
            task_id = %req.task_id,
            workspace_id = ?targets.workspace_id,
            "Failed to resolve workspace credentials"
        );
        Status::internal("Failed to resolve workspace credentials")
    })?;
    let workspace_credential_count = workspace_credentials.len();
    // The credential set the sandbox is handed needs only the Codex refresh
    // half of the orchestrator, so it takes the port rather than the
    // concrete service; `store` below still needs the whole thing.
    let codex_credentials: Option<Arc<dyn CodexCredentials>> =
        handlers.map(|h| h.credential_sync.clone() as Arc<dyn CodexCredentials>);
    let actor = resolve_task_actor(
        handlers.map(|h| &h.workspace_resolver),
        targets.workspace_id.as_deref(),
        targets.run_as_user_id.as_deref(),
        &grant.ctx.user_id,
    )
    .await;
    // Workspace-scoped configuration travels on this channel and not only the
    // pod spec: the per-turn map overlays the pod env, so a workspace var
    // delivered solely through the plan loses to a user- or global-scoped secret.
    let workspace_env_vars = resolve_workspace_env_vars(
        handlers.map(|h| &h.workspace_resolver),
        targets.workspace_id.as_deref(),
    )
    .await;
    let mut credentials = resolve_task_credentials(TaskCredentialInputs {
        task_id: &req.task_id,
        actor: &actor,
        secrets: handlers.map(|h| &h.secrets),
        provider_tokens: handlers.map(|h| &h.harness_credentials),
        git_host_tokens: handlers.map(|h| &h.git_host_tokens),
        repository: targets.repository.as_ref(),
        users: handlers.map(|h| &h.commit_identity),
        credential_sync: codex_credentials.as_ref(),
        workspace_credentials,
        workspace_env_vars,
    })
    .await;

    if let Some(user_token) = service.mint_user_auth_token(&grant) {
        credentials.insert(CredentialKey::USER_AUTH_TOKEN.to_string(), user_token);
    }
    let ssh_keys = resolve_task_ssh_keys(handlers.map(|h| &h.secrets), &grant.ctx.user_id).await;

    info!(
        task_id = %req.task_id,
        user_id = %grant.ctx.user_id,
        has_github = credentials.contains_key(CredentialKey::GITHUB_TOKEN),
        has_gitlab = credentials.contains_key(CredentialKey::GITLAB_TOKEN),
        has_codex_auth = credentials.contains_key(CredentialKey::CODEX_AUTH_JSON),
        has_openai = credentials.contains_key(CredentialKey::OPENAI_API_KEY),
        has_anthropic = credentials.contains_key(CredentialKey::ANTHROPIC_API_KEY),
        has_claude_oauth = credentials.contains_key(CredentialKey::CLAUDE_CODE_OAUTH_TOKEN),
        ssh_key_count = ssh_keys.len(),
        workspace_credential_count,
        "Credentials resolved"
    );

    Ok(Response::new(GetCredentialsResponse {
        credentials,
        ssh_keys,
    }))
}

/// Persist a credential rotated locally inside the sandbox back into
/// `provider_tokens`.
///
/// Called when a runtime (currently Codex) rotates its OAuth refresh token;
/// without this, the next backend-driven refresh overwrites it with the stale
/// stored copy. Always updates the user-level row.
#[tracing::instrument(
    skip_all,
    fields(
        request_id = %uuid::Uuid::new_v4(),
        task_id = tracing::field::Empty,
        user_id = tracing::field::Empty
    )
)]
pub(super) async fn update_credential(
    service: &BackendSandboxServiceImpl,
    request: Request<UpdateCredentialRequest>,
) -> Result<Response<UpdateCredentialResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    let grant = service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();
    tracing::Span::current().record("task_id", req.task_id.as_str());
    tracing::Span::current().record("user_id", grant.ctx.user_id.as_str());

    let Some(provider) = sandbox_rotatable_provider(&req.key) else {
        warn!(
            task_id = %req.task_id,
            key = %req.key,
            "UpdateCredential rejected: unsupported credential key"
        );
        return Err(Status::invalid_argument(format!(
            "Unsupported credential key: {}",
            req.key
        )));
    };

    let Some(credential_sync) = service.handlers.as_ref().map(|h| &h.credential_sync) else {
        return Err(Status::failed_precondition(
            "Provider token repository not configured",
        ));
    };

    // The fan-out reaches every running sandbox the user owns minus the
    // originating one, and a `GITHUB_TOKEN` lands in the encrypted store
    // rather than plaintext in `provider_tokens.access_token`.
    let result = credential_sync
        .store(
            &grant.ctx.user_id,
            &req.key,
            &req.value,
            WriteSource::SandboxRotation {
                task_id: req.task_id.clone(),
            },
        )
        .await;

    if let Err(e) = result {
        warn!(
            error = %e,
            task_id = %req.task_id,
            user_id = %grant.ctx.user_id,
            provider,
            "Failed to persist rotated credential"
        );
        return Err(Status::internal("Failed to persist credential"));
    }

    info!(
        task_id = %req.task_id,
        user_id = %grant.ctx.user_id,
        provider,
        scope = "user",
        "Persisted rotated credential"
    );

    Ok(Response::new(UpdateCredentialResponse {}))
}
