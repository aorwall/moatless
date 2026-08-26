//! gRPC server implementing `BackendSandboxService`.
//!
//! Receives events from sandbox pods via gRPC and serves pending messages.

use std::pin::Pin;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use serde_json::Value;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, info, warn};

use moatless_types::agent::AgentSubType;
use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;
use sandbox_proto::{AcknowledgeMessagesRequest, AcknowledgeMessagesResponse, DownloadTranscriptRequest, DownloadUploadRequest, DownloadUploadResponse, GetCredentialsRequest, GetCredentialsResponse, GetPendingMessagesRequest, GetPendingMessagesResponse, GetSystemPromptRequest, GetSystemPromptResponse, GetWorkspaceSyncUrlsRequest, GetWorkspaceSyncUrlsResponse, InterruptedTurn as ProtoInterruptedTurn, PublishEventRequest, PublishEventResponse, RecoverInterruptedRunRequest, RecoverInterruptedRunResponse, ReportWorkspaceSyncRequest, ReportWorkspaceSyncResponse, TranscriptChunk, UpdateCredentialRequest, UpdateCredentialResponse, UploadTranscriptResponse};

use moatless_types::access_control::ScopeContext;

use crate::sandbox::event_dedup::Admission;
use crate::sandbox::jwt;
use crate::sandbox::lifecycle::orchestrator::LifecycleDao;
use crate::sandbox::secrets::{TaskCredentialInputs, resolve_server_env, resolve_task_actor, resolve_task_credential_targets, resolve_task_credentials, resolve_task_ssh_keys, resolve_workspace_credentials};
use crate::sandbox::storage::sandbox_key;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_core::file_storage::dao::FileDao;
use moatless_types::agent::AgentType;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_types::user::UserRole;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::auth::provider_tokens::ProviderTokenAuthDao;
use moatless_identity::auth::users::UserDao;
use moatless_identity::git_host_tokens::GitHostTokens;
use moatless_identity::secret::service::SecretService;
use moatless_s3 as file_s3;
use moatless_tasks::agent::prompt;
use moatless_backend_core::sandbox::CodexCredentials;
use moatless_tasks::credential_sync::CredentialSync;
use moatless_tasks::event_ingress::outbox::IngressOutbox;
use moatless_tasks::event_ingress::{AdmissionError, AdmissionRequest, EventAdmission};
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::task::dao::TaskDao;
use moatless_tasks::task::relations::TaskRelationDao;
use moatless_tasks::task_event::dao::{InsertTaskEvent, TaskEventDao};
use moatless_tasks::task_turn::dao::{TaskTurnDao, TurnStateEvent};
use moatless_tasks::task_turn::models::{TurnStatus, classify_turn_transition};
use moatless_workspaces::credentials::WriteSource;

/// Known credential env var names.
///
/// Used as map keys in `GetCredentialsResponse` for type-safe credential handling.
pub struct CredentialKey;

impl CredentialKey {
    /// Long-lived user auth token the sandbox uses to authenticate its gRPC
    /// calls back to the backend. Delivered over this channel (instead of the pod
    /// spec) and swapped in by the sandbox after its bootstrap-authenticated
    /// initial fetch; re-minted on every fetch so the 15-min refresher keeps it
    /// fresh.
    pub const USER_AUTH_TOKEN: &str = "USER_AUTH_TOKEN";
    pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
    pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
    pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
    pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";
    pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
    pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
    pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
}

/// Build the S3 key for a task's transcript archive:
/// `<prefix>/<task_id>/transcripts.tar`. The `<prefix>/<task_id>/` portion is the
/// same one the Kubernetes runtime removes, so uploads and cleanup stay aligned.
fn transcript_key(prefix: &str, task_id: &str) -> String {
    format!(
        "{}/{}/transcripts.tar",
        prefix.trim_end_matches('/'),
        task_id
    )
}

/// Lifetime of the presigned workspace-sync URLs. The sandbox re-mints them on
/// every sync tick, so this only needs to cover a single transfer. Kept short to
/// bound the blast radius of a leaked URL, but generous enough for a large tar
/// on a slow link.
const WORKSPACE_PRESIGN_TTL: std::time::Duration = std::time::Duration::from_mins(15);

/// Database-backed ports the request handlers read — the databaseless gate's set
/// minus the durable-admission writers ([`IngressPorts`]). Present-together as
/// one value because the single `ports` gate (a database connection) builds them
/// all at once; `None` as a whole when the backend runs databaseless.
pub struct HandlerDbPorts {
    pub messages: Arc<dyn MessageDao>,
    pub provider_tokens: Arc<dyn ProviderTokenDao>,
    pub tasks: Arc<dyn TaskDao>,
    pub task_relations: Arc<dyn TaskRelationDao>,
    /// Provider token auth persistence for persisting refreshed tokens.
    pub provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    /// Scoped secret resolver for global/team/user secrets.
    pub secrets: Arc<SecretService>,
    /// Codex credential orchestrator (9c) — used to route sandbox-pushed
    /// rotations through `store(.., WriteSource::SandboxRotation)` so the
    /// broadcast fan-out picks them up automatically.
    pub credential_sync: Arc<CredentialSync>,
    /// Workspace projection — supplies the primary repository a task's
    /// credentials are minted against, the run-as identity, and the
    /// `system_prompt` / `servers` the agent's append prompt is built from.
    pub workspace_resolver: Arc<dyn moatless_backend_core::workspace::WorkspaceResolver>,
    /// Sandbox lifecycle store — read for the Sandbox a task belongs to, whose
    /// workspace names the credentials the pod is handed.
    pub sandboxes: Arc<dyn LifecycleDao>,
    /// Provider credentials pinned to a workspace — the highest-precedence
    /// source in the credential set a sandbox is handed.
    pub workspace_credentials: Arc<dyn moatless_backend_core::workspace::WorkspaceCredentialReader>,
    /// The one way to a git-host token, for the identity the task acts as.
    pub git_host_tokens: Arc<GitHostTokens>,
    /// File metadata repository — supplies filename and mime type when
    /// fulfilling `download_upload` requests.
    pub files: Arc<dyn FileDao>,
    /// User repository — used to render `<sent_by>` headers on pending
    /// messages so the agent sees the inbound sender's moatless login.
    pub users: Arc<dyn UserDao>,
    /// The git author identity the pod's commits attribute to.
    pub commit_identity: Arc<dyn moatless_backend_core::identity::CommitIdentity>,
    /// The stored access tokens the agent harness runs with.
    pub harness_credentials: Arc<dyn moatless_backend_core::identity::HarnessCredentials>,
}

/// Durable-admission writers (commit-before-fan-out): the turn/event/outbox rows
/// the ingress path commits to Postgres before publishing to Redis. Present-
/// together off the same `ports` gate as [`HandlerDbPorts`], but a distinct value
/// because the ingress test harness ([`BackendSandboxServiceImpl::for_ingress_tests`])
/// wires exactly these three without the handler set.
pub struct IngressPorts {
    /// Authoritative turn-state writer. State-bearing events commit their turn /
    /// task-status / event rows here transactionally before being published to
    /// Redis (commit-before-fan-out ordering).
    pub task_turns: Arc<dyn TaskTurnDao>,
    /// Event-row writer for events that bear no turn state. Commits before the
    /// Redis publish so Postgres holds the full payload even when the live tail
    /// carries a truncated stub.
    pub task_events: Arc<dyn TaskEventDao>,
    /// Durable spool for the one payload kind `task_events` cannot hold in full:
    /// `agent.message` content, which the agent.message skeleton reduces to a skeleton there. A
    /// record is admitted here before the gRPC ack and deleted once the consumer
    /// commits `task_messages` (the durable-ingress decision).
    pub ingress_outbox: Arc<dyn IngressOutbox>,
}

/// gRPC server state — holds event publisher, DAO trait objects, and JWT secret.
pub struct BackendSandboxServiceImpl {
    pub events: Option<Arc<dyn EventPublisher>>,
    /// Database-backed ports the request handlers read; `None` databaseless.
    pub handlers: Option<HandlerDbPorts>,
    /// Durable-admission writers; `None` databaseless, `Some` on its own in the
    /// ingress test harness.
    pub ingress: Option<IngressPorts>,
    /// JWT signing key for *minting* the user token delivered over
    /// `GetCredentials`. Token *verification* lives entirely in `authorizer`.
    pub task_token_secret: Option<String>,
    /// The single verification layer: every RPC resolves its bearer token to a
    /// [`TaskGrant`] through this before touching task data. `pub` like its
    /// sibling fields because the composition root (`backend/src/startup.rs`)
    /// assembles this service by naming them across the crate boundary.
    pub authorizer: SandboxAuthorizer,
    /// GitHub App service for refreshing expired user access tokens
    /// (default user-auth App). Convenience handle to
    /// `github_apps.default_service()`.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// Registry of all configured GitHub Apps — used to resolve installation
    /// tokens and bot identities for bot-user `provider_tokens` rows.
    pub github_apps: Arc<GitHubAppRegistry>,
    /// Backend config. Read here once (`sandbox.user_token_ttl_secs` when minting
    /// the user auth token) and forwarded whole to the two prompt builders, which
    /// read `frontend_url` / `proxy.*` / `file_mount_path` from it. This service
    /// is the outermost caller of both builders, so there is no frame above to
    /// narrow the forward to; narrowing the builders belongs to their own module.
    pub backend_config: Arc<Config>,
    /// S3 handle used by `download_upload` and workspace sync.
    pub s3: Option<Arc<moatless_s3::S3Storage>>,
    /// S3 bucket name (`config.s3.bucket_name`).
    pub s3_bucket: Option<String>,
    /// S3 key prefix for uploaded files (`config.s3.key_prefix`).
    pub s3_key_prefix: String,
    /// S3 key prefix for sandbox Claude transcripts (`config.s3.transcript_prefix`).
    /// Objects land at `<prefix>/<task_id>/transcripts.tar` in the same bucket.
    pub transcript_s3_prefix: String,
    /// S3 key prefix for `s3`-backend workspace archives (`config.s3.workspace_prefix`).
    /// Objects land at `<prefix>/<task_id>/workspace.tar` in the same bucket;
    /// the sandbox uploads/downloads them directly via presigned URLs.
    pub workspace_s3_prefix: String,
    /// Ingress dedup for sandbox-published events. The sandbox outbox delivers
    /// at-least-once (retries reuse the same `event_id`); this drops retried
    /// duplicates before they reach Redis so SSE consumers don't see them twice.
    pub event_dedup: Arc<crate::sandbox::event_dedup::EventDedup>,
}

/// A resolved authorization to act on one task over the sandbox gRPC API.
///
/// Produced by [`SandboxAuthorizer`]. Carries the sandbox binding plus the
/// caller's scope context, so handlers can run scoped DB work as the caller.
#[derive(Debug)]
pub(crate) struct TaskGrant {
    /// The sandbox the task is a member of.
    pub sandbox_id: String,
    /// The caller's scope context (user id + platform-admin flag).
    pub ctx: ScopeContext,
    /// The caller's login. Needed by `GetCredentials` to mint the delivered user
    /// token and resolve provider credentials. Empty for the dev bypass.
    pub login: String,
}

/// Token-level verdict from verifying a bearer token's signature, before the
/// DB-backed task-grant resolution in [`SandboxAuthorizer::resolve_grant`].
enum VerifiedToken {
    /// No `TASK_TOKEN_SECRET` configured: dev/test bypass. Constructed only via
    /// [`SandboxAuthorizer::dev_unauthenticated`], so the bypass has one origin.
    DevBypass,
    /// A token carrying a `taskId` claim (the post-binding norm). The claim is
    /// authoritative; the request's `task_id` must equal it.
    TaskBound {
        claims: jwt::UserTokenClaims,
        task_id: String,
    },
    /// A token carrying a `sandboxId` claim. The requested task must be a
    /// member of that sandbox.
    SandboxBound {
        claims: jwt::UserTokenClaims,
        sandbox_id: String,
    },
    /// A claim-less token accepted under the grace flag. Its request `task_id`
    /// is authorized purely by the DB control check.
    Claimless { claims: jwt::UserTokenClaims },
}

/// The single verification layer for the sandbox gRPC API (Decision 7).
///
/// Every RPC begins by calling [`authorize`](Self::authorize) (or, for the one
/// `GetCredentials` exception, [`authorize_bootstrap`](Self::authorize_bootstrap)).
/// It verifies the bearer token in the request metadata, resolves it to a
/// [`TaskGrant`], and rejects any request whose in-band `task_id` disagrees with
/// the token's `taskId` claim. A concrete type, not a trait: there is exactly
/// one verification path, and the dev-mode bypass lives in exactly one
/// constructor ([`dev_unauthenticated`](Self::dev_unauthenticated)).
pub struct SandboxAuthorizer {
    /// JWT signing secret. Enforcing mode has one; the dev bypass has none.
    secret: Option<String>,
    /// Accept claim-less tokens via the DB fallback when the grace flag is set.
    accept_claimless: bool,
    /// Task DAO — the claim-less DB fallback and the control gate.
    tasks: Option<Arc<dyn TaskDao>>,
    /// User DAO — the platform-admin lookup.
    users: Option<Arc<dyn UserDao>>,
}

impl SandboxAuthorizer {
    /// Production constructor: verify every token against `secret`.
    pub(crate) fn enforcing(
        secret: String,
        accept_claimless: bool,
        tasks: Option<Arc<dyn TaskDao>>,
        users: Option<Arc<dyn UserDao>>,
    ) -> Self {
        Self {
            secret: Some(secret),
            accept_claimless,
            tasks,
            users,
        }
    }

    /// Dev/test constructor: no signing secret, so every request is trusted and
    /// mapped to a system-scope grant for whatever task it names. This is the
    /// **only** place the bypass is constructed; production refuses to boot
    /// without `TASK_TOKEN_SECRET` (see `Config::validate`), so it never runs
    /// there.
    pub(crate) fn dev_unauthenticated(
        tasks: Option<Arc<dyn TaskDao>>,
        users: Option<Arc<dyn UserDao>>,
    ) -> Self {
        Self {
            secret: None,
            accept_claimless: true,
            tasks,
            users,
        }
    }

    /// Select the mode from config: `Some(secret)` → enforcing, `None` → the dev
    /// bypass. The single branch that decides authenticated-vs-bypass.
    pub fn from_config(
        secret: Option<String>,
        accept_claimless: bool,
        tasks: Option<Arc<dyn TaskDao>>,
        users: Option<Arc<dyn UserDao>>,
    ) -> Self {
        match secret {
            Some(secret) => Self::enforcing(secret, accept_claimless, tasks, users),
            None => Self::dev_unauthenticated(tasks, users),
        }
    }

    /// Extract and verify the bearer token from request metadata.
    ///
    /// `allow_bootstrap` is set only for `GetCredentials`, which uniquely accepts
    /// both bootstrap and long-lived user tokens. Returns the token-level
    /// verdict; the task grant is resolved
    /// separately by [`resolve_grant`](Self::resolve_grant) once the request's
    /// `task_id` is known (it arrives in-band on the streaming RPCs).
    #[allow(clippy::result_large_err)] // reason: tonic::Status is the required gRPC error type
    fn verify_token<T>(
        &self,
        request: &Request<T>,
        allow_bootstrap: bool,
    ) -> Result<VerifiedToken, Status> {
        let Some(secret) = self.secret.as_deref() else {
            return Ok(VerifiedToken::DevBypass);
        };

        let auth_header = request
            .metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("Missing authorization header"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("Invalid authorization header"))?;
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| Status::unauthenticated("Invalid authorization format"))?;

        let claims = if allow_bootstrap {
            jwt::verify_bootstrap_or_user_token(token, secret)
        } else {
            jwt::verify_user_auth_token(token, secret)
        }
        .map_err(|e| Status::unauthenticated(format!("Invalid token: {e}")))?;

        if let Some(sandbox_id) = claims.sandbox_id.clone() {
            return Ok(VerifiedToken::SandboxBound { claims, sandbox_id });
        }
        match claims.task_id.clone() {
            Some(task_id) => Ok(VerifiedToken::TaskBound { claims, task_id }),
            None if self.accept_claimless => Ok(VerifiedToken::Claimless { claims }),
            None => Err(Status::unauthenticated(
                "Token missing sandbox binding (sandboxId claim)",
            )),
        }
    }

    /// Resolve a verified token to a [`TaskGrant`] for `request_task_id`.
    ///
    /// - `TaskBound`: the request's task must equal the claim's task, then the
    ///   claim's user must control it (owner or admin).
    /// - `Claimless`: the request's task is authorized by the DB control check.
    /// - `DevBypass`: trusted; a system-scope grant for the requested task.
    async fn resolve_grant(
        &self,
        verified: VerifiedToken,
        request_task_id: &str,
    ) -> Result<TaskGrant, Status> {
        match verified {
            VerifiedToken::DevBypass => {
                // reason: dev bypass grant — no TASK_TOKEN_SECRET configured, caller is
                // trusted (only reachable via SandboxAuthorizer::dev_unauthenticated).
                #[allow(clippy::disallowed_methods)]
                let ctx = ScopeContext::system();
                Ok(TaskGrant {
                    sandbox_id: request_task_id.to_string(),
                    ctx,
                    login: String::new(),
                })
            }
            VerifiedToken::TaskBound { claims, task_id } => {
                if task_id != request_task_id {
                    warn!(
                        claim_task_id = %task_id,
                        request_task_id,
                        user_id = %claims.user_id,
                        "Rejected sandbox gRPC call: token task binding does not match request"
                    );
                    return Err(Status::permission_denied(
                        "Token is not valid for this task",
                    ));
                }
                self.resolve_control(&claims, request_task_id).await
            }
            VerifiedToken::SandboxBound { claims, sandbox_id } => {
                self.resolve_sandbox_membership(&claims, request_task_id, &sandbox_id)
                    .await
            }
            VerifiedToken::Claimless { claims } => {
                self.resolve_control(&claims, request_task_id).await
            }
        }
    }

    /// Verify + resolve in one step: the standard entrypoint for task-scoped RPCs.
    async fn authorize<T>(
        &self,
        request: &Request<T>,
        request_task_id: &str,
    ) -> Result<TaskGrant, Status> {
        let verified = self.verify_token(request, false)?;
        self.resolve_grant(verified, request_task_id).await
    }

    /// Like [`authorize`](Self::authorize) but also accepts the bootstrap token —
    /// used by `GetCredentials` alone.
    async fn authorize_bootstrap<T>(
        &self,
        request: &Request<T>,
        request_task_id: &str,
    ) -> Result<TaskGrant, Status> {
        let verified = self.verify_token(request, true)?;
        self.resolve_grant(verified, request_task_id).await
    }

    /// The DB-backed control gate: load the task through the caller's scoped read
    /// (an invisible task is a plain NotFound, like the HTTP 404), then require
    /// `ctx.can_control` (owner or admin). Public *visibility* lets a task be
    /// viewed over HTTP but never grants sandbox control — credentials, messages,
    /// and workspace writes stay owner-only.
    async fn resolve_control(
        &self,
        claims: &jwt::UserTokenClaims,
        task_id: &str,
    ) -> Result<TaskGrant, Status> {
        let tasks = self
            .tasks
            .as_ref()
            .ok_or_else(|| Status::internal("Tasks repository not configured"))?;

        let ctx = self.scope_context_for(claims).await?;
        let task = tasks
            .find_by_id(task_id, &ctx)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id, "resolve_control: task lookup failed");
                Status::internal("Failed to load task")
            })?
            .ok_or_else(|| Status::not_found("Task not found"))?;

        if ctx.can_control(&task.scoped_resource()) {
            Ok(TaskGrant {
                sandbox_id: task.sandbox_id.unwrap_or_else(|| task_id.to_string()),
                login: claims.login.clone(),
                ctx,
            })
        } else {
            warn!(
                task_id,
                user_id = %claims.user_id,
                scope = %task.scope,
                "Rejected sandbox gRPC call: caller not authorized for task"
            );
            Err(Status::permission_denied("Not authorized for this task"))
        }
    }

    async fn resolve_sandbox_membership(
        &self,
        claims: &jwt::UserTokenClaims,
        task_id: &str,
        sandbox_id: &str,
    ) -> Result<TaskGrant, Status> {
        let tasks = self
            .tasks
            .as_ref()
            .ok_or_else(|| Status::internal("Tasks repository not configured"))?;

        let ctx = self.scope_context_for(claims).await?;
        let task = tasks
            .find_by_id(task_id, &ctx)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id, sandbox_id, "resolve_sandbox_membership: task lookup failed");
                Status::permission_denied("Sandbox membership could not be confirmed")
            })?
            .ok_or_else(|| Status::permission_denied("Task is not a member of this sandbox"))?;

        let task_sandbox_id = task.sandbox_id.as_deref().unwrap_or(&task.task_id);
        if task_sandbox_id != sandbox_id {
            warn!(
                task_id,
                task_sandbox_id,
                claim_sandbox_id = %sandbox_id,
                user_id = %claims.user_id,
                "Rejected sandbox gRPC call: token sandbox binding does not match task membership"
            );
            return Err(Status::permission_denied(
                "Token is not valid for this task's sandbox",
            ));
        }
        if !ctx.can_control(&task.scoped_resource()) {
            return Err(Status::permission_denied("Not authorized for this task"));
        }

        Ok(TaskGrant {
            sandbox_id: sandbox_id.to_string(),
            login: claims.login.clone(),
            ctx,
        })
    }

    /// Hydrate the caller's `ScopeContext` from the DB. The sandbox JWT carries
    /// only `user_id`/`login`; the platform-admin flag requires one DB lookup.
    async fn scope_context_for(
        &self,
        claims: &jwt::UserTokenClaims,
    ) -> Result<ScopeContext, Status> {
        let user_uuid = claims
            .user_id
            .parse::<uuid::Uuid>()
            .map_err(|_| Status::unauthenticated("Invalid user id in token"))?;
        Ok(ScopeContext {
            user_id: claims.user_id.clone(),
            is_admin: self.user_is_admin(&user_uuid).await,
        })
    }

    /// Platform-admin check for the control gate. Missing repo or a lookup error
    /// is treated as "not admin" (fail closed).
    async fn user_is_admin(&self, user_id: &uuid::Uuid) -> bool {
        match self.users.as_ref() {
            Some(users) => {
                matches!(users.get_user(user_id).await, Ok(Some(u)) if u.role == UserRole::Admin)
            }
            None => false,
        }
    }
}

/// Construction for the DB-backed integration harness, which lives outside this
/// crate and so cannot reach `authorizer` or build a [`SandboxAuthorizer`].
///
/// Gated on `db-tests`, a feature no shipping build enables, so the
/// unauthenticated bypass stays unconstructible from outside the crate.
#[cfg(feature = "db-tests")]
impl BackendSandboxServiceImpl {
    /// Ingress wired to real Postgres DAOs with token verification off.
    /// Everything admission does not touch is absent, which is also the point:
    /// a test that reaches past durable admission fails on `None` rather than
    /// quietly exercising a half-wired service.
    pub fn for_ingress_tests(
        events: Option<Arc<dyn EventPublisher>>,
        task_turns: Arc<dyn TaskTurnDao>,
        task_events: Arc<dyn TaskEventDao>,
        ingress_outbox: Arc<dyn IngressOutbox>,
    ) -> Self {
        Self {
            events,
            handlers: None,
            ingress: Some(IngressPorts {
                task_turns,
                task_events,
                ingress_outbox,
            }),
            authorizer: SandboxAuthorizer::dev_unauthenticated(None, None),
            github_apps: Arc::new(GitHubAppRegistry::default()),
            event_dedup: Arc::new(crate::sandbox::event_dedup::EventDedup::new()),
            task_token_secret: None,
            github_app_service: None,
            backend_config: Arc::new(Config::test_minimal()),
            s3: None,
            s3_bucket: None,
            s3_key_prefix: String::new(),
            transcript_s3_prefix: String::new(),
            workspace_s3_prefix: String::new(),
        }
    }
}

impl BackendSandboxServiceImpl {
    /// Mint the long-lived user auth token delivered over `GetCredentials`.
    ///
    /// The sandbox swaps this in for its bootstrap token and uses it for all
    /// subsequent gRPC calls. Bound to `task_id` (the grant's task) so the
    /// delivered token carries the same per-task binding the verification layer
    /// enforces. Returns `None` (with a warning) if the secret is unset — the
    /// sandbox then falls back to whatever token it already holds.
    fn mint_user_auth_token(&self, user_id: &str, login: &str, task_id: &str) -> Option<String> {
        let secret = self.task_token_secret.as_ref()?;
        let ttl = self.backend_config.sandbox.user_token_ttl_secs;
        match jwt::generate_user_auth_token(user_id, login, task_id, secret, ttl) {
            Ok(token) => Some(token),
            Err(e) => {
                warn!(error = %e, user_id, "Failed to mint user auth token for credentials");
                None
            }
        }
    }
    /// The turn an inbound event belongs to.
    ///
    /// An event published from inside an agent run carries that run's id, which
    /// resolves to its turn — allocating it if this is the run's first signal
    /// to arrive. That is what makes attribution exact: a terminal event
    /// delayed in the sandbox outbox past the start of the next run still
    /// closes its own turn rather than whichever turn is open on arrival.
    ///
    /// Events published outside a run carry no id and bear no turn state; they
    /// fall back to the task's latest turn.
    async fn resolve_turn_for_event(&self, task_id: &str, run_id: &str) -> Option<i32> {
        let turns = &self.ingress.as_ref()?.task_turns;
        if run_id.is_empty() {
            return None;
        }
        match turns.resolve_turn_for_run(task_id, run_id).await {
            Ok(turn_number) => Some(turn_number),
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    run_id = %run_id,
                    error = %e,
                    "Failed to resolve turn for run"
                );
                None
            }
        }
    }

    /// The task's latest turn — the fallback for a state-bearing event that
    /// reached the ingress without a run id.
    async fn latest_turn(&self, task_id: &str) -> Option<i32> {
        self.ingress
            .as_ref()?
            .task_turns
            .latest_turn_number(task_id)
            .await
            .unwrap_or_else(|e| {
                warn!(task_id = %task_id, error = %e, "Failed to read latest turn number");
                None
            })
    }

    /// The task's configured agent type, for stamping onto turn rows.
    async fn resolve_agent_type(&self, task_id: &str) -> Option<String> {
        // reason: event ingress reads the agent type for turn attribution across all owners
        #[allow(clippy::disallowed_methods)]
        let ctx = ScopeContext::system();
        self.handlers
            .as_ref()?
            .tasks
            .find_by_id(task_id, &ctx)
            .await
            .ok()
            .flatten()
            .map(|task| task.agent_type.0)
    }

    /// Resolve the turn an inbound event belongs to and, for a state-bearing
    /// event, the turn-state write that must commit before fan-out.
    async fn build_admission(
        &self,
        event: &PublishEventRequest,
        event_type: &'static str,
        payload: Value,
    ) -> AdmissionRequest {
        // Resolve the turn for every event published from inside an agent run,
        // not just the state-bearing ones: the Redis consumer then receives an
        // explicit turn number on `agent.message` too and never has to guess.
        // Events from outside a run (sandbox/init/heartbeat) bear no turn.
        let mut turn_number = self
            .resolve_turn_for_event(&event.task_id, &event.run_id)
            .await;

        // State-bearing events (taskTurn.*, agent.completed/error/stopped) commit
        // their turn / task-status / event rows in ONE transaction BEFORE the
        // fan-out. Everything else has no state to commit.
        let transition = classify_turn_transition(event_type, &event.sub_type);
        if transition.is_some() && turn_number.is_none() {
            // A state-bearing event that arrived without a run id — a publisher
            // that doesn't front the actor. Attribute it to the open turn.
            turn_number = self.latest_turn(&event.task_id).await;
        }
        let state = if let (Some(transition), Some(turn_number)) = (transition, turn_number) {
            let agent_type = self.resolve_agent_type(&event.task_id).await;
            Some(TurnStateEvent {
                event_id: event.event_id.clone(),
                task_id: event.task_id.clone(),
                event_type: event_type.to_string(),
                sub_type: event.sub_type.clone(),
                is_error: moatless_tasks::event_ingress::is_error_sub_type(&event.sub_type),
                agent_type,
                turn_number,
                payload: payload.clone(),
                timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                transition,
            })
        } else {
            None
        };

        AdmissionRequest {
            event_id: event.event_id.clone(),
            task_id: event.task_id.clone(),
            event_type,
            sub_type: event.sub_type.clone(),
            turn_number,
            payload,
            state,
        }
    }
}

/// Decode one `PublishEventRequest` into the parts admission needs, rejecting
/// wire-contract violations terminally.
///
/// The three rejections are all unfixable by repetition, so they are
/// [`AdmissionError::Invalid`] (gRPC `INVALID_ARGUMENT`) rather than retryable:
/// an unknown event-type discriminant, an absent event id — without which
/// neither deduplication nor idempotent persistence can converge (the durable-ingress decision
/// requires a *stable, non-empty* id) — and a payload that is not JSON.
fn decode_request(event: &PublishEventRequest) -> Result<(&'static str, Value), AdmissionError> {
    // `event_type` is an open integer on the wire. Validate it here, at
    // ingress, so an unknown discriminant is rejected instead of defaulting
    // to a variant. The dispatch strings come from the domain enum so they
    // cannot drift from the wire contract.
    let event_type = match sandbox_proto::EventType::try_from(event.event_type) {
        Ok(proto_event_type) => moatless_types::event::EventType::from(proto_event_type).as_str(),
        Err(e) => {
            warn!(
                event_type = event.event_type,
                task_id = %event.task_id,
                error = %e,
                "Unknown proto EventType value, rejecting"
            );
            return Err(AdmissionError::Invalid(format!(
                "Unknown event_type: {}",
                event.event_type
            )));
        }
    };

    if event.event_id.is_empty() {
        return Err(AdmissionError::Invalid(
            "event_id is required: retries must reuse a stable, non-empty id".to_string(),
        ));
    }

    // An empty payload is a legitimate "no fields" event; anything else must
    // parse, because an unparseable payload would be admitted as `null` and
    // silently lose whatever the sandbox meant to send.
    let payload = if event.payload_json.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&event.payload_json)
            .map_err(|e| AdmissionError::Invalid(format!("payload_json is not valid JSON: {e}")))?
    };

    Ok((event_type, payload))
}

#[tonic::async_trait]
impl BackendSandboxService for BackendSandboxServiceImpl {
    /// Admit an event from the sandbox and fan it out to Redis Streams.
    ///
    /// Success means the **complete payload is durably admitted** — committed
    /// state for a state-bearing event, a durable ingress-outbox record for full
    /// `agent.message` content, a `task_events` row for everything else
    /// (the durable-ingress decision). Anything short of that is a retryable `UNAVAILABLE` and the
    /// sandbox resends under the same id; the retry contract lives on
    /// [`AdmissionError`].
    ///
    /// Authenticated like every other RPC: the caller's task-bound token must
    /// match `event.task_id`.
    ///
    /// This does not risk event loss, but the reason is narrower than "the
    /// sandbox always holds a valid token". A sandbox boots with a *bootstrap*
    /// token and only swaps in the long-lived one after its first
    /// `GetCredentials`, and this path calls `authorize`, not
    /// `authorize_bootstrap` — so events published in that window are rejected.
    /// They survive because the outbox treats only `INVALID_ARGUMENT` as
    /// permanent and retries everything else with backoff until acknowledged,
    /// so a rejected event is redelivered once the swap lands.
    #[tracing::instrument(
        skip_all,
        fields(request_id = %uuid::Uuid::new_v4(), task_id = tracing::field::Empty)
    )]
    async fn publish_event(
        &self,
        request: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        let task_id = request.get_ref().task_id.clone();
        self.authorizer.authorize(&request, &task_id).await?;
        let event = request.into_inner();
        tracing::Span::current().record("task_id", event.task_id.as_str());

        let (event_type_str, payload) =
            decode_request(&event).map_err(|e| e.into_status(&event.task_id, &event.event_id))?;

        // Log sequence gaps/resets for observability (never gates admission).
        self.event_dedup
            .observe_sequence(&event.task_id, event.sequence)
            .await;

        // Claim admission for this id. A duplicate of an already-admitted event
        // is acknowledged without re-publishing (that is what makes a lost
        // response safe); a duplicate racing an in-flight admission is answered
        // retryable, because the winner's admission may still fail.
        let claim = match self
            .event_dedup
            .claim(&event.task_id, &event.event_id, event.sequence)
            .await
        {
            Admission::Claimed(claim) => claim,
            Admission::AlreadyAdmitted => {
                debug!(
                    task_id = %event.task_id,
                    event_id = %event.event_id,
                    sub_type = %event.sub_type,
                    "Duplicate of an admitted event acknowledged at ingress"
                );
                return Ok(Response::new(PublishEventResponse {
                    event_id: event.event_id,
                }));
            }
            Admission::InFlight => {
                return Err(
                    AdmissionError::AdmissionInFlight.into_status(&event.task_id, &event.event_id)
                );
            }
        };

        let request = self.build_admission(&event, event_type_str, payload).await;
        let ingress = self.ingress.as_ref();
        let admission = EventAdmission {
            task_turns: ingress.map(|i| &i.task_turns),
            task_events: ingress.map(|i| &i.task_events),
            ingress_outbox: ingress.map(|i| &i.ingress_outbox),
            publisher: self.events.as_ref(),
        };

        match admission.admit(request).await {
            Ok(()) => {
                claim.admitted().await;
                Ok(Response::new(PublishEventResponse {
                    event_id: event.event_id,
                }))
            }
            Err(e) => {
                // Release the claim so the sandbox's retry is a real second
                // attempt rather than a duplicate of a failed admission.
                claim.failed().await;
                Err(e.into_status(&event.task_id, &event.event_id))
            }
        }
    }

    /// Return pending messages for a task.
    ///
    /// The sandbox polls this instead of the HTTP endpoint. The response
    /// format matches what the sandbox consumer expects.
    #[tracing::instrument(
        skip_all,
        fields(
            request_id = %uuid::Uuid::new_v4(),
            task_id = tracing::field::Empty,
            user_id = tracing::field::Empty
        )
    )]
    async fn get_pending_messages(
        &self,
        request: Request<GetPendingMessagesRequest>,
    ) -> Result<Response<GetPendingMessagesResponse>, Status> {
        // Verify the token and confirm the caller may operate on this task before
        // we hand back its queued messages.
        let task_id = request.get_ref().task_id.clone();
        let grant = self.authorizer.authorize(&request, &task_id).await?;
        tracing::Span::current().record("user_id", grant.ctx.user_id.as_str());

        let req = request.into_inner();
        tracing::Span::current().record("task_id", req.task_id.as_str());

        let Some(messages) = self.handlers.as_ref().map(|h| &h.messages) else {
            return Ok(Response::new(GetPendingMessagesResponse {
                deliveries: vec![],
            }));
        };

        let task = if let Some(tasks) = self.handlers.as_ref().map(|h| &h.tasks) {
            // reason: pending-message gate reads task status across all owners (no user ctx)
            #[allow(clippy::disallowed_methods)]
            let sys = ScopeContext::system();
            tasks.find_by_id(&req.task_id, &sys).await.ok().flatten()
        } else {
            None
        };

        // Gate: a Closed task is terminal — never start a turn on it. The
        // sandbox dispatcher no-ops on an empty list, so returning empty here
        // means no `agent.started`, no turn — even if the pod is still finishing
        // its termination grace period. Fail open on unknown (None / DB-error,
        // both collapsed by `.ok().flatten()`): only a confirmed-Closed task is
        // gated, mirroring `send.rs`.
        if matches!(
            task.as_ref().map(|t| t.status),
            Some(moatless_types::task::TaskStatus::Closed)
        ) {
            debug!(task_id = %req.task_id, "get_pending_messages: task is Closed, returning no messages");
            return Ok(Response::new(GetPendingMessagesResponse {
                deliveries: vec![],
            }));
        }

        let task_agent_type = task.as_ref().map_or_else(
            || AgentType::CLAUDE_CODE.to_string(),
            |t| t.agent_type.0.clone(),
        );
        let task_model = task.as_ref().and_then(|t| t.model.clone());
        let task_skills = task.as_ref().map(|t| t.skills.clone()).unwrap_or_default();

        // Fetch pending user messages from DB
        let pending = messages.get_pending_user(&req.task_id).await.map_err(|e| {
            warn!(error = %e, task_id = %req.task_id, "Failed to fetch pending messages");
            Status::internal("Failed to fetch pending messages")
        })?;

        // Look up latest agent_session_id for --resume <id>
        let task_resume_session_id = messages
            .get_latest_agent_session_id(&req.task_id)
            .await
            .unwrap_or(None);

        // Check the last closed turn — a turn that ended `stopped` or `error`
        // cut its agent session off mid-flight, so the next run starts fresh
        // rather than `--resume` into a transcript ending on an unanswered tool
        // call. Turn state is authoritative here: routine stops leave no result
        // row behind (see `message::models::stop_is_reported`).
        let cut_off = match self.ingress.as_ref().map(|i| &i.task_turns) {
            Some(turns) => match turns.latest_closed_turn_status(&req.task_id).await {
                Ok(status) => status.is_some_and(TurnStatus::cut_off_session),
                Err(e) => {
                    warn!(error = %e, task_id = %req.task_id, "Failed to read last closed turn; assuming resumable");
                    false
                }
            },
            None => false,
        };

        // Check will_not_resume on the last result — if explicitly set,
        // don't pass a resume session ID. Also
        // auto-skip resume when the last result row was an error: retrying the
        // same resume session ID would loop the same failure.
        let task_resume_session_id = if task_resume_session_id.is_none() {
            None
        } else if cut_off {
            debug!(
                task_id = %req.task_id,
                "Last closed turn was stopped or errored, clearing resume_session_id"
            );
            None
        } else {
            match messages.get_last_result(&req.task_id).await {
                Ok(Some(last_result)) if last_result.will_not_resume == Some(true) => {
                    debug!(
                        task_id = %req.task_id,
                        "Last result has will_not_resume=true, clearing resume_session_id"
                    );
                    None
                }
                Ok(Some(last_result))
                    if last_result.message_type
                        == moatless_tasks::message::models::MessageType::Error =>
                {
                    debug!(
                        task_id = %req.task_id,
                        "Last result is an error, clearing resume_session_id to start fresh"
                    );
                    None
                }
                _ => task_resume_session_id,
            }
        };

        // Pre-resolve the logins the headers name. An internal row is keyed by
        // its `sent_by` user id for `<sent_by>`; an external row is keyed by
        // the writer's own Adapter id for `<from>`, which is the only field
        // that names the writer rather than the trigger owner.
        let mut login_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        if let Some(users) = self.handlers.as_ref().map(|h| &h.users) {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for row in &pending {
                if row.external_event_id.is_some() {
                    let (Some(adapter), Some(sender_id)) = (
                        row.external_adapter.as_deref(),
                        row.external_sender_id.as_deref(),
                    ) else {
                        continue;
                    };
                    let key = moatless_tasks::message::enricher::sender_key(adapter, sender_id);
                    if !seen.insert(key.clone()) {
                        continue;
                    }
                    match users.find_user_by_provider_id(adapter, sender_id).await {
                        Ok(Some(user)) => {
                            login_cache.insert(key, user.login);
                        }
                        // Not an error: an unmapped writer is named by their
                        // Adapter handle instead, which stays true.
                        Ok(None) => debug!(
                            adapter, sender_id, "sender is not mapped, <from> names their handle"
                        ),
                        Err(e) => warn!(
                            adapter, sender_id, error = %e,
                            "sender lookup failed, <from> names their handle"
                        ),
                    }
                    continue;
                }
                if let Some(uid) = row.sent_by.as_deref() {
                    if seen.insert(uid.to_string()) {
                        let Ok(uuid) = uid.parse::<uuid::Uuid>() else {
                            continue;
                        };
                        match users.get_user(&uuid).await {
                            Ok(Some(user)) => {
                                login_cache.insert(uid.to_string(), user.login);
                            }
                            Ok(None) => {
                                debug!(user_id = %uid, "sent_by user not found, omitting <sent_by> tag");
                            }
                            Err(e) => {
                                warn!(user_id = %uid, error = %e, "user lookup failed, omitting <sent_by> tag");
                            }
                        }
                    }
                }
            }
        }

        // The agent's view of a queued Message is rebuilt here, not read back.
        //
        // The row stores what the person wrote; the preview URL, the selected
        // element, the attribution to a sending Task and the attached files are
        // each persisted as their own row or column, and pre-rendering them into
        // the Message would only create a second copy that can disagree with
        // them.
        let ids: Vec<i32> = pending.iter().map(|row| row.id).collect();
        let contexts_by_message = messages
            .get_contexts_for_messages(&req.task_id, &ids)
            .await
            .unwrap_or_else(|e| {
                warn!(task_id = %req.task_id, error = %e, "Failed to load message contexts; delivering without them");
                std::collections::HashMap::new()
            });

        let builder = moatless_tasks::message::enricher::AgentPromptBuilder::new(
            self.handlers.as_ref().map(|h| h.files.as_ref()),
            Some(self.backend_config.as_ref()),
            task_agent_type,
            task_model,
            task_resume_session_id,
            task_skills,
            &login_cache,
        );
        let deliveries = builder
            .build_pending_deliveries(&pending, &contexts_by_message)
            .await
            .into_iter()
            .map(|delivery| sandbox_proto::PendingAgentDelivery {
                message_uuids: delivery.message_uuids,
                prompt_text: delivery.prompt_text,
                file_ids: delivery.file_ids,
                model: delivery.model,
                agent_type: delivery.agent_type,
                agent_mode: delivery.agent_mode,
                effort: delivery.effort,
                skills: delivery.skills,
                resume_session_id: delivery.resume_session_id,
                resume: delivery.resume,
                start_fresh: delivery.start_fresh,
                prompt_response: delivery.prompt_response.map(|response| {
                    sandbox_proto::PromptResponse {
                        tool_use_id: response.tool_use_id,
                        tool_name: response.tool_name,
                        answers: response.answers.into_iter().collect(),
                        approved: response.approved,
                        feedback: response.feedback,
                    }
                }),
            })
            .collect();

        Ok(Response::new(GetPendingMessagesResponse { deliveries }))
    }

    /// Acknowledge that pending messages have been delivered to an agent session.
    ///
    /// Sets `sent_at`, `session_id`, and `agent_session_id` on the specified
    /// messages, transitioning them from "queued" to "sent".
    async fn acknowledge_messages(
        &self,
        request: Request<AcknowledgeMessagesRequest>,
    ) -> Result<Response<AcknowledgeMessagesResponse>, Status> {
        // Cross-tenant guard: only the task's owner/admin may acknowledge
        // (mark sent) its queued messages.
        let task_id = request.get_ref().task_id.clone();
        self.authorizer.authorize(&request, &task_id).await?;

        let req = request.into_inner();

        let Some(messages) = self.handlers.as_ref().map(|h| &h.messages) else {
            return Ok(Response::new(AcknowledgeMessagesResponse {}));
        };

        // The acknowledging run decides the turn: whichever of the ack and the
        // run's `taskTurn.active` reaches the backend first allocates it, the
        // other joins. Messages injected into a run in flight ack with that
        // run's id and so share its turn.
        let Some(turn_number) = self.resolve_turn_for_event(&req.task_id, &req.run_id).await else {
            warn!(
                task_id = %req.task_id,
                run_id = %req.run_id,
                "Cannot acknowledge messages without a resolvable turn"
            );
            return Err(Status::internal("Failed to resolve turn for run"));
        };

        match messages
            .acknowledge_messages(
                &req.task_id,
                &req.message_uuids,
                &req.session_id,
                turn_number,
            )
            .await
        {
            Ok(count) => {
                info!(
                    task_id = %req.task_id,
                    session_id = %req.session_id,
                    run_id = %req.run_id,
                    turn_number,
                    requested = req.message_uuids.len(),
                    acknowledged = count,
                    "Messages acknowledged"
                );
                for uuid in &req.message_uuids {
                    moatless_tasks::message::hooks::emit_message_upsert_from_daos(
                        self.events.as_ref(),
                        messages,
                        self.handlers.as_ref().map(|h| &h.files),
                        &req.task_id,
                        uuid,
                    )
                    .await;
                }
            }
            Err(e) => {
                warn!(
                    task_id = %req.task_id,
                    error = %e,
                    "Failed to acknowledge messages"
                );
                return Err(Status::internal("Failed to acknowledge messages"));
            }
        }

        let answered_prompt = (!req.answered_prompt_tool_use_id.is_empty())
            .then(|| req.message_uuids.first())
            .flatten();
        if let Some(carrier) = answered_prompt {
            moatless_tasks::message::send::settle_answered_prompt(
                messages.as_ref(),
                &req.task_id,
                &req.answered_prompt_tool_use_id,
                carrier,
            )
            .await;
        }

        Ok(Response::new(AcknowledgeMessagesResponse {}))
    }

    /// Return credentials for sandbox initialization.
    ///
    /// Resolution chain, most-specific first:
    /// 1. Credentials pinned to the task's Workspace.
    /// 2. User-level provider_tokens.
    /// 3. Scoped global/user secrets.
    #[tracing::instrument(
        skip_all,
        fields(
            request_id = %uuid::Uuid::new_v4(),
            task_id = tracing::field::Empty,
            user_id = tracing::field::Empty
        )
    )]
    async fn get_credentials(
        &self,
        request: Request<GetCredentialsRequest>,
    ) -> Result<Response<GetCredentialsResponse>, Status> {
        // GetCredentials accepts bootstrap and long-lived user tokens. The grant
        // confirms the caller may operate on this task before we resolve and
        // return its provider credentials.
        let task_id = request.get_ref().task_id.clone();
        let grant = self
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

        let handlers = self.handlers.as_ref();
        let targets = resolve_task_credential_targets(
            handlers.map(|h| &h.sandboxes),
            handlers.map(|h| &h.workspace_resolver),
            &req.task_id,
        )
        .await;
        // A workspace credential that cannot be read must not degrade into the
        // run-as user's own token: the sandbox would come up authenticated as
        // the wrong account. Fail the fetch instead — the pod retries, and the
        // operator sees the cause in the backend log.
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
        let codex_credentials: Option<Arc<dyn CodexCredentials>> = handlers
            .map(|h| h.credential_sync.clone() as Arc<dyn CodexCredentials>);
        let actor = resolve_task_actor(
            handlers.map(|h| &h.workspace_resolver),
            targets.workspace_id.as_deref(),
            &grant.ctx.user_id,
        )
        .await;
        let mut credentials = resolve_task_credentials(TaskCredentialInputs {
            task_id: &req.task_id,
            user_id: actor.user_id(),
            secrets: handlers.map(|h| &h.secrets),
            provider_tokens: handlers.map(|h| &h.harness_credentials),
            git_host_tokens: handlers.map(|h| &h.git_host_tokens),
            repository: targets.repository.as_ref(),
            users: handlers.map(|h| &h.commit_identity),
            credential_sync: codex_credentials.as_ref(),
            workspace_credentials,
        })
        .await;

        // Deliver the long-lived user auth token over this channel rather than
        // the pod spec. The sandbox swaps its bootstrap token for this one and
        // re-fetches on the 15-min refresh loop, so a rotated token propagates.
        if let Some(user_token) =
            self.mint_user_auth_token(&grant.ctx.user_id, &grant.login, &grant.sandbox_id)
        {
            credentials.insert(CredentialKey::USER_AUTH_TOKEN.to_string(), user_token);
        }
        let server_env = resolve_server_env(&grant.ctx.user_id, handlers.map(|h| &h.secrets)).await;
        let ssh_keys =
            resolve_task_ssh_keys(handlers.map(|h| &h.secrets), &grant.ctx.user_id).await;

        info!(
            task_id = %req.task_id,
            user_id = %grant.ctx.user_id,
            has_github = credentials.contains_key(CredentialKey::GITHUB_TOKEN),
            has_gitlab = credentials.contains_key(CredentialKey::GITLAB_TOKEN),
            has_gitness = credentials.contains_key(CredentialKey::GITNESS_TOKEN),
            has_codex_auth = credentials.contains_key(CredentialKey::CODEX_AUTH_JSON),
            has_openai = credentials.contains_key(CredentialKey::OPENAI_API_KEY),
            has_anthropic = credentials.contains_key(CredentialKey::ANTHROPIC_API_KEY),
            has_claude_oauth = credentials.contains_key(CredentialKey::CLAUDE_CODE_OAUTH_TOKEN),
            server_env_count = server_env.len(),
            ssh_key_count = ssh_keys.len(),
            workspace_credential_count,
            "Credentials resolved"
        );

        Ok(Response::new(GetCredentialsResponse {
            credentials,
            server_env,
            ssh_keys,
        }))
    }

    /// Persist a credential rotated locally inside the sandbox back into
    /// `provider_tokens`.
    ///
    /// Called by the sandbox when a runtime (currently Codex) rotates its OAuth
    /// refresh token. Without write-back the next backend-driven refresh would
    /// overwrite the rotated value with the now-stale stored copy, permanently
    /// invalidating the credential.
    ///
    /// Always updates the user-level credential row.
    #[tracing::instrument(
        skip_all,
        fields(
            request_id = %uuid::Uuid::new_v4(),
            task_id = tracing::field::Empty,
            user_id = tracing::field::Empty
        )
    )]
    async fn update_credential(
        &self,
        request: Request<UpdateCredentialRequest>,
    ) -> Result<Response<UpdateCredentialResponse>, Status> {
        // Cross-tenant guard: the caller must own/have access to this task
        // before we persist a rotated credential against its user row.
        let task_id = request.get_ref().task_id.clone();
        let grant = self.authorizer.authorize(&request, &task_id).await?;
        let req = request.into_inner();
        tracing::Span::current().record("task_id", req.task_id.as_str());
        tracing::Span::current().record("user_id", grant.ctx.user_id.as_str());

        let Some(provider) = credential_key_to_provider(&req.key) else {
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

        let Some(provider_tokens) = self.handlers.as_ref().map(|h| &h.provider_tokens) else {
            return Err(Status::failed_precondition(
                "Provider token repository not configured",
            ));
        };

        // Route through the credential service (production path) so the broadcast
        // fan-out fires for every running sandbox the user owns — minus the
        // originating one (race-loop guard).
        let result: Result<(), Box<dyn std::error::Error + Send + Sync>> = if let Some(svc) =
            self.handlers.as_ref().map(|h| &h.credential_sync)
        {
            // Routes GITHUB_TOKEN into the encrypted store instead of writing
            // it plaintext to `provider_tokens.access_token`.
            svc.store(
                &grant.ctx.user_id,
                &req.key,
                &req.value,
                WriteSource::SandboxRotation {
                    task_id: req.task_id.clone(),
                },
            )
            .await
            .map_err(Into::into)
        } else if req.key == moatless_core::secret_keys::GITHUB_TOKEN {
            // No credential service means no encrypted store; refuse rather
            // than resurrect a plaintext GitHub token.
            warn!(
                task_id = %req.task_id,
                user_id = %grant.ctx.user_id,
                "UpdateCredential rejected: cannot persist a GitHub token without the credential service"
            );
            return Err(Status::failed_precondition(
                "Cannot persist a GitHub token without the credential service",
            ));
        } else {
            provider_tokens
                .upsert_token(&grant.ctx.user_id, provider, &req.value, None, None, None)
                .await
                .map_err(Into::into)
        };

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

    /// Build and return the assembled append-system-prompt for a task.
    ///
    /// The sandbox calls this once per agent run (in `start_session`) and
    /// caches the result for fallback on subsequent failures. Assembly
    /// reads `Task.name`, `Task.append_system_prompt`, the workspace's
    /// `system_prompt` + `servers`, and the configured frontend/proxy URLs.
    async fn get_system_prompt(
        &self,
        request: Request<GetSystemPromptRequest>,
    ) -> Result<Response<GetSystemPromptResponse>, Status> {
        // Cross-tenant guard: the assembled system prompt embeds task/repo
        // details, so only the task's owner/admin may fetch it.
        let task_id = request.get_ref().task_id.clone();
        let grant = self.authorizer.authorize(&request, &task_id).await?;
        let req = request.into_inner();

        // Load the task for prompt assembly through the caller's scope context.
        let task = match self.handlers.as_ref().map(|h| &h.tasks) {
            Some(tasks) => tasks
                .find_by_id(&req.task_id, &grant.ctx)
                .await
                .ok()
                .flatten(),
            None => None,
        };

        let system_prompt = prompt::build_for_task(
            &req.task_id,
            task.as_ref(),
            self.handlers.as_ref().map(|h| &h.workspace_resolver),
            self.handlers.as_ref().map(|h| &h.task_relations),
            Some(&self.backend_config),
        )
        .await;

        Ok(Response::new(GetSystemPromptResponse { system_prompt }))
    }

    /// Stream uploaded-file bytes to the sandbox.
    ///
    /// The sandbox calls this for every `file_id` carried on a pending
    /// message before invoking the agent, so the on-disk path advertised in
    /// the prompt ("Saved to: …") is valid by the time the agent reads it.
    ///
    /// Auth: task-scoped JWT via gRPC metadata.
    /// Defense in depth: the file_id must be referenced by a message
    /// belonging to this task — prevents one task's sandbox from grabbing
    /// another's uploads.
    async fn download_upload(
        &self,
        request: Request<DownloadUploadRequest>,
    ) -> Result<Response<DownloadUploadResponse>, Status> {
        let task_id = request.get_ref().task_id.clone();
        self.authorizer.authorize(&request, &task_id).await?;
        let req = request.into_inner();

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };
        let Some(files) = self.handlers.as_ref().map(|h| &h.files) else {
            return Err(Status::failed_precondition(
                "File repository not configured",
            ));
        };
        let Some(messages) = self.handlers.as_ref().map(|h| &h.messages) else {
            return Err(Status::failed_precondition(
                "Message repository not configured",
            ));
        };

        // Scope check: the file must be referenced by a message belonging to
        // this task. This is defense in depth on top of the JWT — even a
        // valid token shouldn't be able to pull another task's uploads.
        let used = messages
            .file_id_used_in_task(&req.task_id, &req.file_id)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, file_id = %req.file_id, "file_id_used_in_task query failed");
                Status::internal("Failed to check file scope")
            })?;
        if !used {
            return Err(Status::not_found("File not attached to this task"));
        }

        let meta = files
            .find_by_id(&req.file_id)
            .await
            .map_err(|e| {
                warn!(error = %e, file_id = %req.file_id, "file metadata lookup failed");
                Status::internal("Failed to look up file metadata")
            })?
            .ok_or_else(|| Status::not_found("File metadata not found"))?;

        let s3_key = format!("{}/{}", self.s3_key_prefix, req.file_id);
        let (data, _content_type) = file_s3::get_object(&s3_client.client, bucket, &s3_key)
            .await
            .map_err(|e| {
                warn!(error = %e, file_id = %req.file_id, "S3 get_object failed");
                Status::internal("Failed to fetch file from S3")
            })?;

        debug!(
            task_id = %req.task_id,
            file_id = %req.file_id,
            bytes = data.len(),
            "Served file upload to sandbox"
        );

        Ok(Response::new(DownloadUploadResponse {
            data,
            filename: meta.filename,
            mime_type: meta.mime_type,
        }))
    }

    /// Receive a streamed transcript tar from the sandbox and store it in S3.
    ///
    /// The sandbox is AWS-credential-less, so it proxies all transcript I/O
    /// through here. Client-streaming so a transcript bigger than tonic's unary
    /// cap still round-trips; the chunks are reassembled in memory (a task's tar
    /// is a few MB) and `put_object`ed to `<bucket>/<prefix>/<task_id>/transcripts.tar`.
    ///
    /// Auth: task-scoped JWT via gRPC metadata, like `download_upload`. The
    /// `task_id` is taken from the first chunk.
    async fn upload_transcript(
        &self,
        request: Request<Streaming<TranscriptChunk>>,
    ) -> Result<Response<UploadTranscriptResponse>, Status> {
        // Verify the token up front (from metadata), but defer the task grant:
        // the task_id arrives in-band from the stream below.
        let verified = self.authorizer.verify_token(&request, false)?;

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };

        let mut stream = request.into_inner();
        let mut task_id = String::new();
        let mut data: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.message().await? {
            if task_id.is_empty() && !chunk.task_id.is_empty() {
                task_id = chunk.task_id;
            }
            data.extend_from_slice(&chunk.data);
        }

        if task_id.is_empty() {
            return Err(Status::invalid_argument(
                "task_id missing from transcript stream",
            ));
        }

        // Now that the task is known, resolve the grant — the token's binding
        // must match it and the caller must control it.
        self.authorizer.resolve_grant(verified, &task_id).await?;

        let bytes_received = data.len() as i64;
        let key = transcript_key(&self.transcript_s3_prefix, &task_id);
        file_s3::put_object(&s3_client.client, bucket, &key, data, "application/x-tar")
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %task_id, "S3 put_object for transcript failed");
                Status::internal("Failed to store transcript in S3")
            })?;

        debug!(task_id = %task_id, bytes = bytes_received, key = %key, "Stored transcript in S3");
        Ok(Response::new(UploadTranscriptResponse { bytes_received }))
    }

    type DownloadTranscriptStream =
        Pin<Box<dyn Stream<Item = Result<TranscriptChunk, Status>> + Send>>;

    /// Stream the task's stored transcript tar back to the sandbox.
    ///
    /// Returns an empty stream when no transcript has been stored yet (first run,
    /// or the ephemeral tier's first boot) — the sandbox treats that as "nothing
    /// to restore". Reuses `AppState.s3` + the uploads bucket like `download_upload`.
    async fn download_transcript(
        &self,
        request: Request<DownloadTranscriptRequest>,
    ) -> Result<Response<Self::DownloadTranscriptStream>, Status> {
        let task_id = request.get_ref().task_id.clone();
        self.authorizer.authorize(&request, &task_id).await?;
        let req = request.into_inner();

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };

        let key = transcript_key(&self.transcript_s3_prefix, &req.task_id);
        let data = file_s3::get_object_opt(&s3_client.client, bucket, &key)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, "S3 get_object for transcript failed");
                Status::internal("Failed to fetch transcript from S3")
            })?
            .unwrap_or_default();

        debug!(task_id = %req.task_id, bytes = data.len(), "Serving transcript to sandbox");

        // Chunk the in-memory tar so each gRPC message stays well under the default
        // limit. An empty tar yields no chunks → an empty stream.
        const CHUNK_SIZE: usize = 1024 * 1024; // 1 MiB
        let chunks: Vec<TranscriptChunk> = data
            .chunks(CHUNK_SIZE)
            .map(|c| TranscriptChunk {
                task_id: String::new(),
                data: c.to_vec(),
            })
            .collect();

        let stream = futures::stream::iter(chunks).map(Ok::<TranscriptChunk, Status>);
        Ok(Response::new(Box::pin(stream)))
    }

    /// Mint presigned PUT + GET URLs for the task's workspace archive.
    ///
    /// The `s3` storage backend uses this instead of the transcript-style gRPC
    /// proxy: the workspace tar can be hundreds of MB, so rather than stream it
    /// through the backend, we hand the sandbox short-lived URLs scoped to
    /// exactly `<prefix>/<task_id>/workspace.tar` and it transfers directly to
    /// S3. The sandbox stays AWS-credential-less; the URL is the only capability
    /// it holds and it expires. Auth: task-scoped JWT via gRPC metadata.
    async fn get_workspace_sync_urls(
        &self,
        request: Request<GetWorkspaceSyncUrlsRequest>,
    ) -> Result<Response<GetWorkspaceSyncUrlsResponse>, Status> {
        let task_id = request.get_ref().task_id.clone();
        self.authorizer.authorize(&request, &task_id).await?;
        let req = request.into_inner();

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };

        let key = sandbox_key(&self.workspace_s3_prefix, &req.task_id);

        let put_url = file_s3::presign_put(s3_client, bucket, &key, WORKSPACE_PRESIGN_TTL)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, "presign PUT for workspace failed");
                Status::internal("Failed to presign workspace upload URL")
            })?;
        let get_url = file_s3::presign_get(s3_client, bucket, &key, WORKSPACE_PRESIGN_TTL)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, "presign GET for workspace failed");
                Status::internal("Failed to presign workspace download URL")
            })?;

        debug!(task_id = %req.task_id, key = %key, "Minted workspace sync URLs");
        Ok(Response::new(GetWorkspaceSyncUrlsResponse {
            expires_in_secs: put_url.expires_in.min(get_url.expires_in).as_secs() as i64,
            put_url: put_url.url,
            get_url: get_url.url,
        }))
    }

    /// Record a workspace-sync outcome reported by the sandbox as Prometheus
    /// metrics. The `s3` backend transfers directly to S3, so the backend never
    /// sees the bytes and must be told the size/duration/outcome to build the
    /// EBS-vs-S3 comparison. Best-effort: it only touches metrics, never fails
    /// the sandbox's sync. Auth: task-scoped JWT via gRPC metadata.
    async fn report_workspace_sync(
        &self,
        request: Request<ReportWorkspaceSyncRequest>,
    ) -> Result<Response<ReportWorkspaceSyncResponse>, Status> {
        let task_id = request.get_ref().task_id.clone();
        self.authorizer.authorize(&request, &task_id).await?;
        let req = request.into_inner();

        crate::sandbox::metrics::record_workspace_sync(
            &req.direction,
            req.bytes.max(0) as u64,
            std::time::Duration::from_millis(req.duration_ms.max(0) as u64),
            req.ok,
        );

        if req.ok && req.direction == "upload" {
            if let Some(task_events) = self.ingress.as_ref().map(|i| &i.task_events) {
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let event = InsertTaskEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    task_id: req.task_id.clone(),
                    event_type: "sandbox".to_string(),
                    sub_type: "workspace_synced".to_string(),
                    is_error: false,
                    agent_type: None,
                    turn_number: None,
                    payload: serde_json::json!({
                        "direction": req.direction,
                        "bytes": req.bytes.max(0),
                        "durationMs": req.duration_ms.max(0),
                        "ok": true,
                    }),
                    timestamp: now,
                };
                if let Err(e) = task_events.insert_event(&event).await {
                    warn!(
                        task_id = %req.task_id,
                        error = %e,
                        "Failed to persist successful workspace sync marker"
                    );
                }
            }
        }

        debug!(
            task_id = %req.task_id,
            direction = %req.direction,
            bytes = req.bytes,
            duration_ms = req.duration_ms,
            ok = req.ok,
            "Recorded workspace sync report"
        );
        Ok(Response::new(ReportWorkspaceSyncResponse {}))
    }

    /// Detect an interrupted agent run and return the data needed to replay it.
    ///
    /// Called by the sandbox once on startup, **before** pending-message
    /// dispatch. An interrupted run is a user-turn whose `agent_session_id`
    /// is set (the sandbox acknowledged it) but whose turn never reached a
    /// terminal `agent.completed/error/stopped` event row — i.e. the prior
    /// pod was killed mid-run.
    ///
    /// Note: turns whose terminal event was `agent.error` are excluded by
    /// `find_interrupted_turn` (the SQL filters on `sub_type IN
    /// ('completed', 'error', 'stopped')`), so a turn that already errored
    /// will never be replayed by the recovery path. Pair this with the
    /// auto-suppression of `resume_session_id` when the last result was an
    /// `Error` row in `get_pending_messages` to ensure the next user message
    /// also starts a fresh session instead of looping the same resume failure.
    ///
    /// Bumps `recovery_attempts` on each successful return. When the
    /// `MAX_RECOVERY_ATTEMPTS` cap is hit, inserts a synthetic Aborted Result
    /// row and publishes `agent.recovery_exhausted` so the UI can surface the
    /// failure clearly. This stops OOM-loop pods from hammering recovery.
    async fn recover_interrupted_run(
        &self,
        request: Request<RecoverInterruptedRunRequest>,
    ) -> Result<Response<RecoverInterruptedRunResponse>, Status> {
        // Cross-tenant guard: recovery replays another turn's messages and burns
        // a recovery attempt on the task, so gate it on task access.
        let task_id = request.get_ref().task_id.clone();
        self.authorizer.authorize(&request, &task_id).await?;

        let req = request.into_inner();

        let Some(messages) = self.handlers.as_ref().map(|h| &h.messages) else {
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        };

        // Load the task once: gate Closed tasks and tasks that no longer desire
        // a running sandbox (terminal/intentional stop — no replay, and don't
        // burn a recovery attempt) and reuse the row for `agent_type` below.
        // Placed before `find_interrupted_turn` / `increment_recovery_attempts`.
        // Fail open on unknown (None / DB-error): only confirmed terminal or
        // stopped intent is gated.
        let task = if let Some(tasks) = self.handlers.as_ref().map(|h| &h.tasks) {
            // reason: recovery gate reads task status across all owners (no user ctx)
            #[allow(clippy::disallowed_methods)]
            let sys = ScopeContext::system();
            tasks.find_by_id(&req.task_id, &sys).await.ok().flatten()
        } else {
            None
        };
        if matches!(
            task.as_ref().map(|t| t.status),
            Some(moatless_types::task::TaskStatus::Closed)
        ) {
            debug!(task_id = %req.task_id, "recover_interrupted_run: task is Closed, skipping recovery");
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        }
        // No Sandbox row means the Task never requested runtime resources;
        // only explicit Stopped or Removed intent gates recovery.
        let sandbox_desired_state = if let Some(tasks) = self.handlers.as_ref().map(|h| &h.tasks) {
            match tasks
                .get_sandbox_intents(std::slice::from_ref(&req.task_id))
                .await
            {
                Ok(mut intents) => intents
                    .remove(&req.task_id)
                    .filter(moatless_tasks::task::models::SandboxIntentView::has_explicit_intent)
                    .map(|intent| intent.desired_state),
                Err(e) => {
                    warn!(
                        error = %e,
                        task_id = %req.task_id,
                        "Failed to load Sandbox lifecycle intent for recovery gate"
                    );
                    None
                }
            }
        } else {
            None
        };
        if matches!(
            sandbox_desired_state,
            Some(moatless_types::sandbox::SandboxDesiredState::Stopped)
                | Some(moatless_types::sandbox::SandboxDesiredState::Removed)
        ) {
            debug!(
                task_id = %req.task_id,
                "recover_interrupted_run: task does not desire a running sandbox, skipping recovery"
            );
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        }

        let interrupted = match messages.find_interrupted_turn(&req.task_id).await {
            Ok(Some(turn)) => turn,
            Ok(None) => {
                debug!(task_id = %req.task_id, "RecoverInterruptedRun: no interrupted turn");
                return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
            }
            Err(e) => {
                warn!(task_id = %req.task_id, error = %e, "find_interrupted_turn failed");
                return Err(Status::internal("Failed to query interrupted turn"));
            }
        };

        const MAX_RECOVERY_ATTEMPTS: i32 = 3;
        if interrupted.recovery_attempts >= MAX_RECOVERY_ATTEMPTS {
            warn!(
                task_id = %req.task_id,
                turn_number = interrupted.turn_number,
                attempts = interrupted.recovery_attempts,
                "RecoverInterruptedRun: cap reached, marking aborted"
            );
            // No transcript row: a run outcome is not something somebody said.
            // The turn's own status says the run ended, and the
            // `agent.recoveryExhausted` event below says why — with the attempt
            // count and the session it gave up on, which the sentence it
            // replaced did not carry.
            if let Some(ref publisher) = self.events {
                publisher
                    .publish_agent_event(
                        &req.task_id,
                        AgentSubType::RecoveryExhausted,
                        None,
                        serde_json::json!({
                            "turn_number": interrupted.turn_number,
                            "session_id": interrupted.session_id,
                            "attempts": interrupted.recovery_attempts,
                        }),
                    )
                    .await;
            }
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        }

        let Some(task_turns) = self.ingress.as_ref().map(|i| &i.task_turns) else {
            warn!(task_id = %req.task_id, "Cannot record a recovery attempt without task_turns");
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        };
        let new_attempts = match task_turns
            .increment_recovery_attempts(&req.task_id, interrupted.turn_number)
            .await
        {
            Ok(n) => n,
            Err(e) => {
                warn!(error = %e, task_id = %req.task_id, "increment_recovery_attempts failed");
                return Err(Status::internal("Failed to increment recovery attempts"));
            }
        };

        let task_agent_type = task.as_ref().map(|t| t.agent_type.0.clone());

        if let Some(ref publisher) = self.events {
            publisher
                .publish_agent_event(
                    &req.task_id,
                    AgentSubType::RecoveryStarted,
                    task_agent_type.as_deref(),
                    serde_json::json!({
                        "turn_number": interrupted.turn_number,
                        "session_id": interrupted.session_id,
                        "attempt": new_attempts,
                        "max_attempts": MAX_RECOVERY_ATTEMPTS,
                    }),
                )
                .await;
        }

        info!(
            task_id = %req.task_id,
            session_id = %interrupted.session_id,
            turn_number = interrupted.turn_number,
            attempt = new_attempts,
            "RecoverInterruptedRun: replaying interrupted turn"
        );

        Ok(Response::new(RecoverInterruptedRunResponse {
            turn: Some(ProtoInterruptedTurn {
                session_id: interrupted.session_id,
                turn_number: i64::from(interrupted.turn_number),
                prompt_text: interrupted.prompt_text,
                agent_type: task_agent_type,
                agent_mode: interrupted.agent_mode,
                effort: interrupted.effort,
            }),
        }))
    }
}

/// Map a credential env-var key (the public namespace shared with sandbox env)
/// to the provider name used in the `provider_tokens` table.
///
/// Returns `None` for keys that are not write-back-eligible — only OAuth-style
/// rotating credentials need this path. Static API keys are managed entirely
/// in the backend.
fn credential_key_to_provider(key: &str) -> Option<&'static str> {
    match key {
        CredentialKey::CODEX_AUTH_JSON => Some("codex"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `publish_event` derives its dispatch string from the domain enum, and
    /// `classify_turn_transition` plus the Redis consumer key off that exact
    /// string. Pin the wire integer → dispatch string contract so a rename in
    /// the shared crate cannot silently re-route ingress.
    #[test]
    fn every_wire_event_type_keeps_its_dispatch_string() {
        for (wire, expected) in [
            (sandbox_proto::EventType::Task, "task"),
            (sandbox_proto::EventType::TaskTurn, "taskTurn"),
            (sandbox_proto::EventType::Sandbox, "sandbox"),
            (sandbox_proto::EventType::Agent, "agent"),
            (sandbox_proto::EventType::Server, "server"),
            (sandbox_proto::EventType::Resources, "resources"),
        ] {
            let decoded = sandbox_proto::EventType::try_from(wire as i32)
                .expect("known discriminant decodes");
            assert_eq!(
                moatless_types::event::EventType::from(decoded).as_str(),
                expected,
                "dispatch string changed for {wire:?}"
            );
        }
    }

    /// Ingress rejects unknown discriminants instead of defaulting to a variant;
    /// the handler turns this failure into `InvalidArgument`.
    #[test]
    fn unknown_wire_event_type_integers_do_not_decode() {
        for unknown in [-1, 6, 99] {
            assert!(sandbox_proto::EventType::try_from(unknown).is_err());
        }
    }

    // =========================================================================
    // publish_event request decoding — the terminal half of the retry contract
    // =========================================================================

    use moatless_tasks::event_ingress::AdmissionError;
    use moatless_tasks::task_turn::models::TurnStatus;

    fn request(event_type: i32, event_id: &str, payload_json: &str) -> PublishEventRequest {
        PublishEventRequest {
            task_id: "task-1".to_string(),
            event_type,
            sub_type: "message".to_string(),
            payload_json: payload_json.to_string(),
            event_id: event_id.to_string(),
            sequence: 1,
            run_id: String::new(),
        }
    }

    #[test]
    fn decodes_a_well_formed_request() {
        let (event_type, payload) = decode_request(&request(
            sandbox_proto::EventType::Agent as i32,
            "evt-1",
            r#"{"uuid":"msg-1"}"#,
        ))
        .expect("decodes");
        assert_eq!(event_type, "agent");
        assert_eq!(payload["uuid"], "msg-1");
    }

    #[test]
    fn an_empty_payload_is_a_legitimate_no_field_event() {
        let (_, payload) = decode_request(&request(
            sandbox_proto::EventType::Sandbox as i32,
            "evt-1",
            "",
        ))
        .expect("decodes");
        assert_eq!(payload, Value::Null);
    }

    #[test]
    fn wire_contract_violations_are_terminal() {
        // An unknown discriminant, a missing event id, and an unparseable
        // payload can never be admitted — repeating them cannot help, so each
        // must be terminal rather than a retry loop.
        for (case, request) in [
            ("unknown event type", request(42, "evt-1", "{}")),
            (
                "empty event id",
                request(sandbox_proto::EventType::Agent as i32, "", "{}"),
            ),
            (
                "malformed payload",
                request(sandbox_proto::EventType::Agent as i32, "evt-1", "not json"),
            ),
        ] {
            let error = decode_request(&request).expect_err(case);
            assert!(!error.is_retryable(), "{case} must be terminal");
            assert!(matches!(error, AdmissionError::Invalid(_)), "{case}");
            assert_eq!(
                error.into_status("task-1", "evt-1").code(),
                tonic::Code::InvalidArgument,
                "{case}"
            );
        }
    }

    #[test]
    fn terminal_status_str_roundtrips() {
        // Guards the DB status strings the projection writes.
        assert_eq!(TurnStatus::Completed.as_str(), "completed");
        assert_eq!(TurnStatus::Error.as_str(), "error");
        assert_eq!(TurnStatus::Stopped.as_str(), "stopped");
    }

    // ---------------------------------------------------------------------
    // SandboxAuthorizer — the single verification layer shared by every
    // task-scoped gRPC handler (credentials, messages, transcripts, uploads,
    // workspace-sync URLs). `resolve_control` = scoped read + `ctx.can_control`
    // (admin or owner); `resolve_grant` additionally enforces the token's task
    // binding. The scoped read's SQL filter is covered by `scope.rs` tests;
    // these exercise the control-gate composition and the binding check.
    // ---------------------------------------------------------------------

    use moatless_types::access_control::Scope;

    use moatless_identity::auth::users::{MockUserDao, User, UserDao};
    use moatless_tasks::task::dao::{MockTaskDao, TaskDao};
    use moatless_types::task::Visibility;
    use moatless_tasks::task::models::Task;

    fn epoch() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(0, 0).expect("epoch is a valid timestamp")
    }

    /// Deterministic uuid for a readable test name: distinct names map to
    /// distinct ids, equal names to equal ids.
    fn uid(name: &str) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    }

    /// A `Task` fixture with the given scope/owner; all other columns are
    /// inert defaults irrelevant to the access check.
    fn task_fixture(scope: Scope, owner_user_id: Option<&str>) -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: chrono::Utc::now(),
            last_message_at: chrono::Utc::now(),
            agent_type: moatless_types::agent::AgentType("claude-code".to_string()),
            repository_id: "repo-1".to_string(),
            status: moatless_types::task::TaskStatus::Open,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            model: None,
            created_by: owner_user_id.map(|u| uid(u).to_string()),
            visibility: moatless_types::task::Visibility::Private,
            scope,
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 0,
            inactive_ttl_days: 0,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            skills: Vec::new(),
        }
    }

    fn user_fixture(id: &uuid::Uuid, role: UserRole) -> User {
        User {
            id: *id,
            provider_user_id: None,
            login: id.to_string(),
            email: None,
            name: None,
            avatar_url: None,
            provider: "test".to_string(),
            role,
            created_at: epoch(),
            last_login_at: epoch(),
            settings: serde_json::json!({}),
            is_bot: false,
        }
    }

    fn claims_for(user_id: &str) -> jwt::UserTokenClaims {
        jwt::UserTokenClaims {
            user_id: uid(user_id).to_string(),
            login: user_id.to_string(),
            task_id: None,
            sandbox_id: None,
            iat: 0,
            exp: u64::MAX,
            iss: "moatless-vibe-backend".to_string(),
            aud: None,
        }
    }

    /// Build a service whose authorizer has only the repositories the control
    /// gate consults wired; everything else is `None`/default.
    fn service_with(tasks: MockTaskDao, users: MockUserDao) -> BackendSandboxServiceImpl {
        let tasks: Arc<dyn TaskDao> = Arc::new(tasks);
        let users: Arc<dyn UserDao> = Arc::new(users);
        BackendSandboxServiceImpl {
            events: None,
            handlers: None,
            ingress: None,
            task_token_secret: None,
            authorizer: SandboxAuthorizer::dev_unauthenticated(Some(tasks), Some(users)),
            github_app_service: None,
            github_apps: Arc::new(GitHubAppRegistry::default()),
            backend_config: Arc::new(Config::test_minimal()),
            s3: None,
            s3_bucket: None,
            s3_key_prefix: String::new(),
            transcript_s3_prefix: String::new(),
            workspace_s3_prefix: String::new(),
            event_dedup: Arc::new(crate::sandbox::event_dedup::EventDedup::default()),
        }
    }

    /// Mock task repo whose scoped `find_by_id` returns `task` for any caller —
    /// i.e. "the scope filter let the read through". Pass `None` to model a
    /// task that is missing or invisible to the caller.
    fn tasks_returning(task: Option<Task>) -> MockTaskDao {
        let mut m = MockTaskDao::new();
        m.expect_find_by_id()
            .returning(move |_, _| Ok(task.clone()));
        m
    }

    fn users_regular() -> MockUserDao {
        let mut m = MockUserDao::new();
        m.expect_get_user()
            .returning(|id| Ok(Some(user_fixture(id, UserRole::User))));
        m
    }

    fn users_admin() -> MockUserDao {
        let mut m = MockUserDao::new();
        m.expect_get_user()
            .returning(|id| Ok(Some(user_fixture(id, UserRole::Admin))));
        m
    }

    #[tokio::test]
    async fn assert_task_access_allows_user_task_for_owner() {
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner")))),
            users_regular(),
        );
        assert!(
            svc.authorizer
                .resolve_control(&claims_for("owner"), "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_task_access_denies_user_task_for_non_owner() {
        // A user-scoped task belongs only to its owner; a different caller with a
        // valid JWT must be rejected even though the token itself is genuine.
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner")))),
            users_regular(),
        );
        let err = svc
            .authorizer
            .resolve_control(&claims_for("attacker"), "task-1")
            .await
            .expect_err("non-owner must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn assert_task_access_public_visibility_never_grants_control() {
        // A public task is *viewable* by anyone (the scoped read's visibility
        // bypass, modelled here by the mock returning the row), but sandbox
        // control — credentials, messages, workspace writes — stays with the
        // owner. This is the regression test for reusing the plain read gate.
        let mut task = task_fixture(Scope::User, Some("owner"));
        task.visibility = Visibility::Public;
        let svc = service_with(tasks_returning(Some(task)), users_regular());
        let err = svc
            .authorizer
            .resolve_control(&claims_for("stranger"), "task-1")
            .await
            .expect_err("public visibility must not grant sandbox control");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn assert_task_access_global_task_requires_admin() {
        // Global-scoped tasks are admin-created (`can_create_scope`) and
        // admin-controlled; being globally *visible* doesn't hand out control.
        let denied = service_with(
            tasks_returning(Some(task_fixture(Scope::Global, None))),
            users_regular(),
        );
        let err = denied
            .authorizer
            .resolve_control(&claims_for("stranger"), "task-1")
            .await
            .expect_err("non-admin must not control a global task");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);

        let allowed = service_with(
            tasks_returning(Some(task_fixture(Scope::Global, None))),
            users_admin(),
        );
        assert!(
            allowed
                .authorizer
                .resolve_control(&claims_for("admin"), "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_task_access_admin_bypasses_scope() {
        // A platform admin controls any user-scoped task, even one they don't own.
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("other-user")))),
            users_admin(),
        );
        assert!(
            svc.authorizer
                .resolve_control(&claims_for("admin"), "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_task_access_missing_task_is_not_found() {
        // Covers both a nonexistent task and one the scoped read filtered out —
        // an invisible task is indistinguishable from a missing one, like the
        // HTTP 404.
        let svc = service_with(tasks_returning(None), users_regular());
        let err = svc
            .authorizer
            .resolve_control(&claims_for("anyone"), "ghost")
            .await
            .expect_err("missing task must error");
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    /// Build a `TaskBound` verdict for the given owner + claimed task, as
    /// `verify_token` would produce from a task-bound JWT.
    fn task_bound(user_id: &str, claim_task_id: &str) -> VerifiedToken {
        let mut claims = claims_for(user_id);
        claims.task_id = Some(claim_task_id.to_string());
        VerifiedToken::TaskBound {
            claims,
            task_id: claim_task_id.to_string(),
        }
    }

    #[tokio::test]
    async fn task_bound_token_mismatch_is_denied() {
        // A token minted for task-1 must not act on task-2, even if its user
        // owns task-2 — the binding is the first, request-independent gate. The
        // control check never runs (no task load), so the repo returns nothing.
        let svc = service_with(tasks_returning(None), users_regular());
        let err = svc
            .authorizer
            .resolve_grant(task_bound("owner", "task-1"), "task-2")
            .await
            .expect_err("token bound to a different task must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn task_bound_token_match_allows_owner() {
        // Matching binding + owner control → granted, with the sandbox binding
        // echoed back on the grant.
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner")))),
            users_regular(),
        );
        let grant = svc
            .authorizer
            .resolve_grant(task_bound("owner", "task-1"), "task-1")
            .await
            .expect("matching binding + owner must be granted");
        assert_eq!(grant.sandbox_id, "task-1");
    }
}
