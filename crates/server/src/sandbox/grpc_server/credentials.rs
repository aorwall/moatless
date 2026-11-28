//! The credentials a sandbox is issued.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

use sandbox_proto::{GetCredentialsRequest, GetCredentialsResponse};

use crate::sandbox::secrets::{
    TaskCredentialInputs, TaskCredentialTargets, resolve_task_actor,
    resolve_task_credential_targets, resolve_task_credentials, resolve_workspace_credentials,
    resolve_workspace_env_vars,
};
use moatless_core::sandbox_jwt as jwt;
use moatless_core::secret_keys;
use moatless_types::access_control::{Actor, ScopeContext};
use moatless_types::attribution::Attribution;
use moatless_types::credential_key;
use moatless_workspace_ports::WorkspaceResolver;
use moatless_workspace_types::repository::{GitHostRepo, RepositoryProvider};

use super::authorize::TaskGrant;
use super::state::BackendSandboxServiceImpl;

/// Known credential env var names, used as the map keys in
/// `GetCredentialsResponse`.
pub(crate) struct CredentialKey;

impl CredentialKey {
    /// Re-minted every fetch, so the sandbox's 15-minute refresh loop
    /// propagates a rotation.
    pub const USER_AUTH_TOKEN: &str = credential_key::USER_AUTH_TOKEN;
    pub const GITLAB_TOKEN: &str = secret_keys::GITLAB_TOKEN;
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

/// The identity a task's credentials resolve for, on behalf of the Sandbox's
/// driver, else its owner. The token's user, which the grant checked is that
/// person, stands in only where no Sandbox row was read.
async fn task_actor(
    service: &BackendSandboxServiceImpl,
    targets: &TaskCredentialTargets,
    grant: &TaskGrant,
) -> Actor {
    let person = targets
        .identity
        .acting_user_id
        .as_deref()
        .unwrap_or(&grant.ctx.user_id);
    resolve_task_actor(
        service.handlers.as_ref().map(|h| &h.workspace_resolver),
        targets.workspace_id.as_deref(),
        targets.identity.pinned_run_as.as_deref(),
        person,
    )
    .await
}

/// The Workspace's primary repository, its first placement, when it is on
/// GitHub; `None` when it is elsewhere or unreadable, which commits as the
/// run-as identity.
async fn primary_github_repository(
    workspaces: &Arc<dyn WorkspaceResolver>,
    workspace_id: &str,
) -> Option<GitHostRepo> {
    let placements = workspaces
        .placements(workspace_id)
        .await
        .map_err(|error| {
            warn!(%error, workspace_id, "Failed to read the workspace placements for the commit author");
        })
        .ok()?;
    placements
        .into_iter()
        .next()?
        .repository?
        .remote?
        .host_repo
        .filter(|repo| repo.provider() == RepositoryProvider::Github)
}

/// Return credentials for sandbox initialization: the task token, the git
/// identity and the actor's secrets, never a model credential (see
/// [`resolve_task_credentials`]).
///
/// Resolution chain, most-specific first: Workspace-pinned credentials, then
/// Workspace `env_vars`, then scoped global/user secrets.
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
    let targets =
        resolve_task_credential_targets(handlers.map(|h| &h.sandboxes), &req.task_id).await;
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
    let actor = task_actor(service, &targets, &grant).await;
    let task = match handlers {
        Some(handlers) => {
            #[expect(
                clippy::disallowed_methods,
                reason = "the Sandbox grant already authorized this Task"
            )]
            let scope = ScopeContext::system();
            handlers
                .tasks
                .find_by_id(&req.task_id, &scope)
                .await
                .map_err(|error| {
                    warn!(%error, task_id = %req.task_id, "Failed to read task reporter");
                    Status::internal("Failed to resolve task reporter")
                })?
        }
        None => None,
    };
    let attribution = Attribution {
        reporter: task.as_ref().and_then(|t| t.reporter_user_id.clone()),
        ..targets.identity.attribution()
    };
    let github_repository = match (handlers, targets.workspace_id.as_deref()) {
        (Some(handlers), Some(workspace_id)) => {
            primary_github_repository(&handlers.workspace_resolver, workspace_id).await
        }
        _ => None,
    };
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
        attribution: &attribution,
        github_repository: github_repository.as_ref(),
        secrets: handlers.map(|h| &h.secrets),
        git_host_tokens: handlers.map(|h| &h.git_host_tokens),
        users: handlers.map(|h| &h.commit_identity),
        workspace_credentials,
        workspace_env_vars,
        gitlab_token_to_pods: handlers.is_none_or(|h| h.gitlab_token_to_pods),
    })
    .await;

    // After resolution so a Task's own setting wins over a Workspace env var of
    // the same name.
    if let Some(task) = &task {
        for (name, value) in task.agent_properties.agent_env() {
            credentials.insert(name.to_string(), value.to_string());
        }
    }

    if let Some(handlers) = handlers {
        // Brokered secrets are dropped before the Workspace is read, so a
        // failed read leaves the pod without its service URLs, never with a
        // secret.
        if let Err(error) = handlers
            .service_access
            .shape_pod_credentials(targets.workspace_id.as_deref(), &mut credentials)
            .await
        {
            warn!(%error, task_id = %req.task_id, "Failed to resolve the workspace's service proxies");
        }
    }

    if let Some(user_token) = service.mint_user_auth_token(&grant) {
        credentials.insert(CredentialKey::USER_AUTH_TOKEN.to_string(), user_token);
    }

    info!(
        task_id = %req.task_id,
        user_id = %grant.ctx.user_id,
        has_gitlab = credentials.contains_key(CredentialKey::GITLAB_TOKEN),
        workspace_credential_count,
        "Credentials resolved"
    );

    Ok(Response::new(GetCredentialsResponse {
        credentials,
        model_gateway: true,
    }))
}
