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
use moatless_tasks::event_ingress::EventAdmission;
use moatless_tasks::message::operations::Messages;
use moatless_tasks::task::fork::TaskForks;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::relations::TaskRelations;
use moatless_tasks::task_event::operations::TaskEvents;
use moatless_tasks::task_turn::operations::TaskTurns;
use moatless_tasks::trajectory::operations::Trajectories;

use super::authorize::SandboxAuthorizer;

/// Database-backed ports the request handlers read — the databaseless gate's
/// set minus the durable-admission writers ([`IngressPorts`]).
///
/// Present together because the single `ports` gate (a DB connection) builds
/// them all at once; `None` as a whole when the backend runs databaseless.
pub struct HandlerDbPorts {
    /// Hands the Sandbox to the person whose message starts a turn.
    pub drivers: Arc<crate::sandbox_driver::SandboxDrivers>,
    pub messages: Arc<Messages>,
    /// The append seam the git-push claim writes its Message through.
    pub transcript: Arc<moatless_tasks::ports::Transcript>,
    pub tasks: Arc<TaskCommands>,
    pub task_relations: Arc<TaskRelations>,
    /// Read for the memory index a Loop's `memory.preload` puts in the prompt.
    pub memories: Arc<moatless_tasks::memory::operations::Memories>,
    /// Where a fork's first turn branches from.
    pub task_forks: Arc<TaskForks>,
    /// Scoped secret resolver for global/team/user secrets.
    pub secrets: Arc<dyn moatless_ports::identity::SecretResolver>,
    /// Workspace projection: the primary repository, the run-as identity, and
    /// the `system_prompt` an append prompt is built from.
    pub workspace_resolver: Arc<dyn moatless_workspace_ports::WorkspaceResolver>,
    /// Sandbox lifecycle store, read for the Sandbox a task belongs to.
    pub sandboxes: Arc<SandboxIngress>,
    /// Provider credentials pinned to a workspace, the highest-precedence
    /// source in the set a sandbox is handed.
    pub workspace_credentials: Arc<dyn moatless_workspace_ports::WorkspaceCredentialReader>,
    /// The one way to a git-host token, for the identity the task acts as.
    pub git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// File metadata repository, for `download_upload_chunked`'s filename and mime
    /// type.
    pub files: Arc<Files>,
    /// The moatless login behind an inbound writer, for the `<from>` header on
    /// a pending message.
    pub provider_link: Arc<dyn ProviderIdentityLink>,
    /// The moatless login of the user a task's system prompt names.
    pub user_logins: Arc<dyn UserResolver>,
    /// The git author identity the pod's commits attribute to.
    pub commit_identity: Arc<dyn CommitIdentity>,
    /// The trajectory store `AppendTrajectory` and `ReadTrajectory` answer
    /// from.
    pub trajectory: Arc<Trajectories>,
    /// The `skill-<name>` tags an appended `Skill` call propagates.
    pub tags: Arc<moatless_tasks::tag::operations::Tags>,
    /// The cache `FetchPlugin` serves plugin files from.
    pub plugin_archives: Arc<moatless_workspaces::plugin::archive::PluginArchives>,
    /// Hands a pod its enabled services' relay URLs, and withholds the secrets
    /// they broker.
    pub service_access: Arc<crate::service_proxy::ServiceAccess>,
    /// [`moatless_core::config::Config::gitlab_token_to_pods`].
    pub gitlab_token_to_pods: bool,
}

/// Durable-admission writers.
///
/// Off the same `ports` gate as [`HandlerDbPorts`] but a distinct value, since
/// [`BackendSandboxServiceImpl::for_ingress_tests`] wires exactly these three
/// without the handler set.
pub struct IngressPorts {
    /// The one durable write every inbound event goes through. Its turn,
    /// task-status and event rows commit before the publish.
    pub admission: Arc<EventAdmission>,
    /// Turn attribution and resume state, read outside admission: which turn an
    /// event belongs to, and how the last turn closed.
    pub task_turns: Arc<TaskTurns>,
    /// Event-row writer for the markers the handlers record outside admission.
    pub task_events: Arc<TaskEvents>,
}

/// What every RPC on this surface reads its ports and its verification layer
/// from.
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
    /// [`TaskGrant`] through this before touching task data.
    pub authorizer: SandboxAuthorizer,
    /// Refreshes an expired user access token. See
    /// [`crate::state::AppState::github_app_service`] for what it aliases.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// Every configured GitHub App, for installation tokens and bot identities.
    pub github_apps: Arc<GitHubAppRegistry>,
    /// Backend config. Read here for `sandbox.user_token_ttl_secs`, and
    /// forwarded whole to the two prompt builders.
    pub backend_config: Arc<Config>,
    /// S3 handle used by `download_upload_chunked` and workspace sync.
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
    /// S3 key prefix for sandbox Claude transcripts
    /// (`config.s3.transcript_prefix`). Objects land at
    /// `<prefix>/<task_id>/transcripts.tar` in the same bucket.
    pub transcript_s3_prefix: String,
    /// S3 key prefix for `s3`-backend workspace archives
    /// (`config.s3.workspace_prefix`). Objects land at
    /// `<prefix>/<task_id>/workspace.tar` in the same bucket.
    pub workspace_s3_prefix: String,
    /// The sandbox outbox delivers at-least-once, reusing the `event_id` on a
    /// retry. This drops the duplicate before it reaches the feed.
    pub event_dedup: Arc<crate::sandbox::event_dedup::EventDedup>,
    /// In-progress `publish_event` admissions, which outlive a cancelled call.
    /// The gRPC server closes and drains it on shutdown.
    pub admissions: tokio_util::task::TaskTracker,
}

/// Construction for the DB-backed integration harness, which lives outside this
/// crate and so cannot reach `authorizer` or build a [`SandboxAuthorizer`].
///
/// Gated on `db-tests`, a feature no shipping build enables, so the
/// unauthenticated bypass stays unconstructible from outside the crate.
#[cfg(feature = "db-tests")]
impl BackendSandboxServiceImpl {
    /// Ingress wired to real Postgres DAOs with token verification off.
    /// Everything admission does not touch is `None`, so a test that reaches
    /// past durable admission fails rather than exercising a half-wired
    /// service.
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
            admissions: tokio_util::task::TaskTracker::new(),
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
