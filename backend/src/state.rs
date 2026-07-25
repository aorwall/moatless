//! Unified application state for API routes.
//!
//! All handler-facing database access goes through repository trait objects,
//! enabling full unit-test mocking without a real database.

use std::sync::Arc;

use crate::adapters::linear::active_turns::ActiveLinearTurns;
use crate::adapters::Adapters;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
use crate::auth::provider_tokens::{PgProviderTokenAuthRepository, ProviderTokenAuthRepository};
use crate::auth::users::{PgUserRepository, UserRepository};
use crate::bindings::recently_sent::RecentlySent;
use crate::config::Config;
use crate::credentials::{BroadcastContext, CredentialService};
use crate::db::DbPool;
use crate::events::EventPublisher;
use crate::feedback::dao::{FeedbackRepository, PgFeedbackRepository};
use crate::file_storage::dao::{FileRepository, PgFileRepository};
use crate::loops::store::LoopStore;
use crate::message::dao::{MessageRepository, PgMessageRepository};
use crate::plugin_repo::dao::{PgPluginRepoRepository, PluginRepoRepository};
use crate::repository::dao::{PgRepoRepository, RepoRepository};
use crate::sandbox::access::SandboxConnector;
use crate::sandbox::lifecycle::runtime::SandboxRuntime;
use crate::sandbox::lifecycle::{
    FleetObserver, GrpcAgentStop, Lifecycle, NotifyPort, OutboxEventSink, PgLifecycleLock,
    PgLifecycleStore, ProvisionDeps, Reconciler, ReconcilerConfig, SystemWallClock,
};
use crate::sandbox::preflight::{PreflightCredentials, RealGithubValidator};
use crate::secret::audit::{PgSecretAuditRepository, SecretAuditRepository};
use crate::secret::dao::{PgSecretRepository, SecretRepository};
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretService};
use crate::skill::fetcher::{GitHubSkillFetcher, SkillFetcher};
use crate::skill::service::SkillService;
use crate::tag::dao::{PgTagRepository, TagRepository};
use crate::task::creation::{RealPreflight, TaskCreationService};
use crate::task::dao::{PgTaskRepository, TaskRepository};
use crate::task::reaper::AgentStatusPort;
use crate::task::relations::{PgTaskRelationRepository, TaskRelationRepository};
use crate::task_event::dao::{PgTaskEventRepository, TaskEventRepository};
use crate::task_turn::dao::{PgTaskTurnRepository, TaskTurnRepository};
use crate::team::dao::{PgTeamRepository, TeamRepository};
use crate::workspace::dao::{PgWorkspaceRepository, WorkspaceRepository};

/// Shared application state for task/message/agent/file routes.
///
/// All fields are cheap to clone (Arc-wrapped or internally Arc'd).
/// Repository trait objects enable handler unit tests without Postgres.
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool — kept for auth routes and infrastructure
    /// code that constructs its own state (gRPC server, idle reaper, etc.).
    pub db: DbPool,
    /// Immutable configuration shared across handlers.
    pub config: Arc<Config>,
    /// The one Sandbox value on `AppState`: the durable lifecycle handle. Callers
    /// request transitions (`start`/`stop`/`restart`/`redeploy`/`remove`) and read
    /// desired state through it rather than driving a provider directly.
    pub sandbox: Arc<Lifecycle>,
    /// The reaper's idle-eligibility read of live agent status, behind a
    /// consumer-owned one-method port. The provider handle is not on `AppState`.
    pub agent_status: Arc<dyn AgentStatusPort>,
    /// Message notification port. Address construction stays inside the
    /// connector-backed adapter rather than on `AppState`.
    pub sandbox_notify: Arc<dyn NotifyPort>,
    /// Optional event publisher for real-time event streaming (trait object).
    pub events: Option<Arc<dyn EventPublisher>>,
    /// Optional S3 client for file uploads/downloads.
    pub s3: Option<aws_sdk_s3::Client>,
    /// Optional external adapters.
    pub adapters: Option<Arc<Adapters>>,
    /// Optional GitHub App service for installation tokens and user token refresh.
    ///
    /// Convenience handle to the default (user-auth) App. Equivalent to
    /// `github_apps.default_service()`. Kept for back-compat with existing
    /// call sites; new code that needs to pick an App by key should go
    /// through `github_apps` instead.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// Registry of all configured GitHub Apps (the default user-auth App plus
    /// any additional bot Apps from `GITHUB_APPS_JSON`). Bot users' provider
    /// tokens reference an App by `key` (stored in `provider_user_id`).
    pub github_apps: Arc<GitHubAppRegistry>,
    /// Generic credential service — serializes refreshes per `(user, key)` and
    /// broadcasts rotations to running sandboxes.
    pub credentials_svc: Arc<CredentialService>,
    /// Short-lived in-memory cache of `(task_id, external_event_id)` we
    /// just sent outbound — closes the race where the inbound webhook lands
    /// before `set_external_event_id` commits. See [`RecentlySent`].
    pub recently_sent: Arc<RecentlySent>,
    /// Per-task gate that suppresses stray Linear activity emission after a
    /// turn has terminated. See [`ActiveLinearTurns`].
    pub linear_active_turns: Arc<ActiveLinearTurns>,
    /// Maps `tool_use_id` → bucketed tool-type label, populated when an
    /// assistant `tool_use` block is observed so the later (separate)
    /// `tool_result` event can be labelled by tool type for the
    /// `moatless_agent_tool_calls_total` metric without a DB round-trip.
    /// Bounded LRU; a `messages.find_tool_call` lookup covers cross-process /
    /// post-restart misses, falling back to `unknown`.
    pub tool_call_names: Arc<moka::future::Cache<String, &'static str>>,

    // =========================================================================
    // Repository trait objects
    // =========================================================================
    pub tasks: Arc<dyn TaskRepository>,
    /// Single owner of the create-then-provision sequence (short-id, preflight,
    /// insert, tags, relations, provision, first message, event).
    pub task_creation: Arc<TaskCreationService>,
    pub task_relations: Arc<dyn TaskRelationRepository>,
    /// Single owner of persistence for the four Loop tables (`triggers`,
    /// `trigger_rules`, `adapter_event_trigger_sources`,
    /// `schedule_trigger_sources`).
    pub loops: Arc<LoopStore>,
    pub messages: Arc<dyn MessageRepository>,
    pub feedback: Arc<dyn FeedbackRepository>,
    pub repositories: Arc<dyn RepoRepository>,
    pub tags: Arc<dyn TagRepository>,
    pub files: Arc<dyn FileRepository>,
    pub users: Arc<dyn UserRepository>,
    pub provider_tokens: Arc<dyn ProviderTokenRepository>,
    pub provider_token_auth: Arc<dyn ProviderTokenAuthRepository>,
    pub secret_records: Arc<dyn SecretRepository>,
    pub secret_audits: Arc<dyn SecretAuditRepository>,
    pub secrets: Arc<SecretService>,
    pub teams: Arc<dyn TeamRepository>,
    pub task_events: Arc<dyn TaskEventRepository>,
    /// Authoritative per-turn lifecycle state (`task_turns`). Written
    /// transactionally at the gRPC ingress; the reaper reads it as the backstop.
    pub task_turns: Arc<dyn TaskTurnRepository>,
    pub plugin_repos: Arc<dyn PluginRepoRepository>,
    /// Workspace composition + run-config (`workspaces` / `workspace_repos`).
    /// Provisioning resolves a task's repository composition through this.
    pub workspaces: Arc<dyn WorkspaceRepository>,
    /// Skill discovery service — reads `.claude/skills/` from workspace
    /// and plugin repositories, parses frontmatter, caches results.
    pub skills: Arc<SkillService>,
}

/// Bounded per-Task lock-acquire attempts before a lifecycle call surfaces
/// `OperationInProgress`. The inline retry is only a latency fast path — the
/// reconciler is the durable retry owner — so this stays small.
const LIFECYCLE_LOCK_ATTEMPTS: u32 = 3;

/// Build the shared durable Sandbox lifecycle handle over its Postgres and
/// runtime adapters. One instance is shared by `AppState`, the sandbox API, and
/// the pod watcher so Sandbox intent writes and per-Task locks stay
/// single-sourced. `pool` backs the store + plan repositories; `lock_pool` must
/// be the dedicated lock-only pool.
pub fn build_lifecycle(
    pool: DbPool,
    lock_pool: DbPool,
    plan: ProvisionDeps,
    runtime: Arc<dyn SandboxRuntime>,
    connector: Arc<dyn SandboxConnector>,
    default_sandbox_image: String,
    events: Option<Arc<dyn EventPublisher>>,
) -> Arc<Lifecycle> {
    let store = Arc::new(PgLifecycleStore::new(pool.clone()));
    Arc::new(
        Lifecycle::new(
            store,
            runtime,
            Arc::new(GrpcAgentStop::new(connector)),
            Arc::new(OutboxEventSink::new(events)),
            Arc::new(PgLifecycleLock::new(lock_pool)),
            Arc::new(SystemWallClock),
        )
        .with_plan_knowledge(plan, default_sandbox_image)
        .with_lock_attempts(LIFECYCLE_LOCK_ATTEMPTS),
    )
}

/// Assemble the desired-state reconciler over the shared pool, the lifecycle
/// handle it drives operations through, and a fleet observer. Keeps the
/// concrete `Pg*` adapters private to this constructor.
pub fn build_reconciler(
    pool: DbPool,
    lifecycle: Arc<Lifecycle>,
    observer: Arc<dyn FleetObserver>,
    notify: Arc<dyn NotifyPort>,
    config: ReconcilerConfig,
) -> Reconciler {
    Reconciler::new(
        (*lifecycle).clone(),
        Arc::new(PgLifecycleStore::new(pool.clone())),
        Arc::new(PgMessageRepository(pool)),
        observer,
        notify,
        Arc::new(SystemWallClock),
        config,
    )
}

impl AppState {
    /// Build `AppState` with Postgres-backed repositories.
    ///
    /// Wires all `Arc<dyn …Repository>` from a single connection pool,
    /// keeping the concrete `Pg*` types private to this constructor.
    // reason: independent infrastructure handles (pool, lifecycle, connector,
    // events, s3, adapters, apps); bundling them behind a one-use struct only
    // shuffles the same fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: DbPool,
        lifecycle: Arc<Lifecycle>,
        config: Arc<Config>,
        connector: Arc<dyn SandboxConnector>,
        agent_status: Arc<dyn AgentStatusPort>,
        sandbox_notify: Arc<dyn NotifyPort>,
        events: Option<Arc<dyn EventPublisher>>,
        s3: Option<aws_sdk_s3::Client>,
        adapters: Option<Arc<Adapters>>,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Self {
        let github_app_service = github_apps.default_service();

        let tasks: Arc<dyn TaskRepository> = Arc::new(PgTaskRepository(pool.clone()));
        let workspaces: Arc<dyn WorkspaceRepository> =
            Arc::new(PgWorkspaceRepository(pool.clone()));
        let repositories: Arc<dyn RepoRepository> = Arc::new(PgRepoRepository(pool.clone()));
        let plugin_repos: Arc<dyn PluginRepoRepository> =
            Arc::new(PgPluginRepoRepository(pool.clone()));
        let provider_tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(PgProviderTokenRepository(pool.clone()));
        let provider_token_auth: Arc<dyn ProviderTokenAuthRepository> =
            Arc::new(PgProviderTokenAuthRepository(pool.clone()));
        let secret_records: Arc<dyn SecretRepository> = Arc::new(PgSecretRepository(pool.clone()));
        let secret_audits: Arc<dyn SecretAuditRepository> =
            Arc::new(PgSecretAuditRepository(pool.clone()));
        let secret_cipher = LocalEnvelopeSecretCipher::from_config(&config.secrets);
        let secrets = Arc::new(SecretService::new(
            secret_records.clone(),
            secret_audits.clone(),
            secret_cipher,
        ));

        // Shared HTTP client for Codex token refresh — kept warm across calls.
        let codex_http = Arc::new(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        "Failed to build reqwest client for Codex refresh, falling back to default"
                    );
                    reqwest::Client::new()
                }),
        );
        let credentials_svc = Arc::new(
            CredentialService::new(
                provider_tokens.clone(),
                Some(provider_token_auth.clone()),
                github_app_service.clone(),
                Some(BroadcastContext {
                    connector,
                    tasks: Arc::new(PgTaskRepository(pool.clone())),
                }),
            )
            .with_codex_refresh(pool.clone(), codex_http)
            .with_secrets(secrets.clone()),
        );

        let skill_http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("moatless-vibe-backend")
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "Failed to build reqwest client for skill fetcher, falling back to default"
                );
                reqwest::Client::new()
            });
        let skill_fetcher: Arc<dyn SkillFetcher> = Arc::new(GitHubSkillFetcher::new(skill_http));
        let skills = Arc::new(SkillService::new(
            repositories.clone(),
            plugin_repos.clone(),
            provider_tokens.clone(),
            provider_token_auth.clone(),
            github_app_service.clone(),
            github_apps.clone(),
            secrets.clone(),
            skill_fetcher,
            std::time::Duration::from_secs(config.skill.cache_ttl_secs),
        ));

        // The shared durable Sandbox lifecycle handle is built once by the caller
        // (`build_lifecycle`) so the sandbox API and pod watcher share this
        // instance — one intent writer, one per-Task lock.

        let tags: Arc<dyn TagRepository> = Arc::new(PgTagRepository(pool.clone()));
        let task_relations: Arc<dyn TaskRelationRepository> =
            Arc::new(PgTaskRelationRepository(pool.clone()));
        let users: Arc<dyn UserRepository> = Arc::new(PgUserRepository(pool.clone()));

        // Wire GitHub-token preflight once here so every task-creation path
        // (HTTP, triggers, adapters) gets the same check for free.
        let preflight = Arc::new(RealPreflight::new(
            PreflightCredentials {
                secrets: secrets.clone(),
                provider_tokens: provider_tokens.clone(),
                github_app_service: github_app_service.clone(),
                provider_token_auth: provider_token_auth.clone(),
                github_apps: github_apps.clone(),
                users: users.clone(),
                credentials_svc: credentials_svc.clone(),
            },
            Arc::new(RealGithubValidator),
        ));

        let task_creation = Arc::new(TaskCreationService::new(
            tasks.clone(),
            tags.clone(),
            task_relations.clone(),
            repositories.clone(),
            events.clone(),
            lifecycle.clone(),
            preflight,
        ));

        Self {
            tasks,
            task_creation,
            task_relations,
            loops: Arc::new(LoopStore::new(pool.clone())),
            messages: Arc::new(PgMessageRepository(pool.clone())),
            feedback: Arc::new(PgFeedbackRepository(pool.clone())),
            repositories,
            tags,
            files: Arc::new(PgFileRepository(pool.clone())),
            users,
            provider_tokens,
            provider_token_auth,
            secret_records,
            secret_audits,
            secrets,
            teams: Arc::new(PgTeamRepository(pool.clone())),
            task_events: Arc::new(PgTaskEventRepository(pool.clone())),
            task_turns: Arc::new(PgTaskTurnRepository(pool.clone())),
            plugin_repos,
            workspaces,
            skills,
            db: pool,
            config,
            sandbox: lifecycle,
            agent_status,
            sandbox_notify,
            events,
            s3,
            adapters,
            github_app_service,
            github_apps,
            credentials_svc,
            recently_sent: Arc::new(RecentlySent::new()),
            linear_active_turns: Arc::new(ActiveLinearTurns::new()),
            tool_call_names: Arc::new(
                moka::future::Cache::builder()
                    .max_capacity(50_000)
                    .time_to_live(std::time::Duration::from_mins(30))
                    .build(),
            ),
        }
    }
}
