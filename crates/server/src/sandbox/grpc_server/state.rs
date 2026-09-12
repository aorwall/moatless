//! The service handle every RPC on this surface is answered from.

use std::sync::Arc;

use crate::sandbox::lifecycle::SandboxIngress;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_core::file_storage::Files;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_ports::identity::{
    CommitIdentity, GitHostTokenIssuer, ProviderIdentityLink, UserResolver,
};
use moatless_tasks::credential_sync::CredentialSync;
use moatless_tasks::event_ingress::EventAdmission;
use moatless_tasks::message::operations::Messages;
use moatless_tasks::task::fork::TaskForks;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::relations::TaskRelations;
use moatless_tasks::task_event::operations::TaskEvents;
use moatless_tasks::task_turn::operations::TaskTurns;

use super::authorize::SandboxAuthorizer;

/// Database-backed ports the request handlers read — the databaseless gate's set
/// minus the durable-admission writers ([`IngressPorts`]). Present-together as
/// one value because the single `ports` gate (a database connection) builds them
/// all at once; `None` as a whole when the backend runs databaseless.
pub struct HandlerDbPorts {
    pub messages: Arc<Messages>,
    /// The append seam the git-push claim writes its Message through.
    pub transcript: Arc<moatless_tasks::ports::Transcript>,
    pub tasks: Arc<TaskCommands>,
    pub task_relations: Arc<TaskRelations>,
    /// Where a fork's first turn branches from. Read on every delivery built
    /// for a fork until the fork's own session exists — by primary key, and on
    /// a table with a row per fork rather than per Task.
    pub task_forks: Arc<TaskForks>,
    /// Scoped secret resolver for global/team/user secrets.
    pub secrets: Arc<dyn moatless_ports::identity::SecretResolver>,
    /// Codex credential orchestrator (9c) — used to route sandbox-pushed
    /// rotations through `store(.., WriteSource::SandboxRotation)` so the
    /// broadcast fan-out picks them up automatically.
    pub credential_sync: Arc<CredentialSync>,
    /// Workspace projection — supplies the primary repository a task's
    /// credentials are minted against, the run-as identity, and the
    /// `system_prompt` / `servers` the agent's append prompt is built from.
    pub workspace_resolver: Arc<dyn moatless_ports::workspace::WorkspaceResolver>,
    /// Sandbox lifecycle store — read for the Sandbox a task belongs to, whose
    /// workspace names the credentials the pod is handed.
    pub sandboxes: Arc<SandboxIngress>,
    /// Provider credentials pinned to a workspace — the highest-precedence
    /// source in the credential set a sandbox is handed.
    pub workspace_credentials: Arc<dyn moatless_ports::workspace::WorkspaceCredentialReader>,
    /// The one way to a git-host token, for the identity the task acts as.
    pub git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// File metadata repository — supplies filename and mime type when
    /// fulfilling `download_upload` requests.
    pub files: Arc<Files>,
    /// The moatless login behind an inbound writer, for the `<from>` header on
    /// a pending message.
    pub provider_link: Arc<dyn ProviderIdentityLink>,
    /// The moatless login behind a `sent_by` user id, for the `<sent_by>`
    /// header on a pending message.
    pub user_logins: Arc<dyn UserResolver>,
    /// The git author identity the pod's commits attribute to.
    pub commit_identity: Arc<dyn CommitIdentity>,
    /// The stored access tokens the agent harness runs with.
    pub harness_credentials: Arc<dyn moatless_ports::identity::HarnessCredentials>,
}

/// Durable-admission writers (commit-before-fan-out): the turn/event/outbox rows
/// the ingress path commits to Postgres before publishing to Redis. Present-
/// together off the same `ports` gate as [`HandlerDbPorts`], but a distinct value
/// because the ingress test harness ([`BackendSandboxServiceImpl::for_ingress_tests`])
/// wires exactly these three without the handler set.
pub struct IngressPorts {
    /// The one durable write every inbound event goes through, over the turn,
    /// event and outbox stores: state-bearing events commit their turn /
    /// task-status / event rows transactionally, and `agent.message` content is
    /// spooled, both before the Redis publish (commit-before-fan-out ordering).
    pub admission: Arc<EventAdmission>,
    /// Turn attribution and resume state, read outside admission: which turn an
    /// event belongs to, and how the last turn closed.
    pub task_turns: Arc<TaskTurns>,
    /// Event-row writer for the markers the handlers record outside admission.
    pub task_events: Arc<TaskEvents>,
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
    /// (default user-auth App). See
    /// [`crate::state::AppState::github_app_service`] for what it aliases.
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
    /// Mints what `GetWorkspaceRepositoryAccess` hands out. `None` unless the
    /// repository role, master secret and bucket are all configured, which is
    /// what makes that RPC answer `FailedPrecondition`.
    pub workspace_repository:
        Option<Arc<crate::sandbox::workspace_repository::WorkspaceRepositoryAccess>>,
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
        ingress: IngressPorts,
    ) -> Self {
        Self {
            events,
            handlers: None,
            ingress: Some(ingress),
            authorizer: SandboxAuthorizer::dev_unauthenticated(None, None),
            github_apps: Arc::new(GitHubAppRegistry::default()),
            event_dedup: Arc::new(crate::sandbox::event_dedup::EventDedup::new()),
            task_token_secret: None,
            github_app_service: None,
            backend_config: Arc::new(Config::test_minimal()),
            s3: None,
            workspace_repository: None,
            s3_bucket: None,
            s3_key_prefix: String::new(),
            transcript_s3_prefix: String::new(),
            workspace_s3_prefix: String::new(),
        }
    }
}
