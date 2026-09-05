//! The composition root.
//!
//! [`compose`] answers "given process configuration, what ports exist", and is
//! where the ports a process holds are constructed. A caller that receives
//! [`Ports`] takes the ports it needs rather than a pool and a `Pg*Dao`
//! constructor. Holding a [`DbPool`] is a separate question — raw-pool
//! services keep their own and are unaffected.

use std::sync::Arc;

use crate::adapters::delivery::AdapterDeliveryStore;
use crate::file_storage::dao::{FileDao, PgFileDao};
use crate::sandbox::lifecycle::orchestrator::LifecycleDao;
use crate::sandbox::lifecycle::{
    GrpcAgentStop, GrpcWorkspaceSync, Lifecycle, MemberRegistryPort, NotifyPort, OutboxEventSink,
    PgLifecycleDao, PgLifecycleLock, ProvisionDeps, Reconciler, ReconcilerConfig, ReconcilerDeps,
    S3WorkspaceArchive, SystemWallClock,
};
use moatless_adapter_model::dao::{AdapterConnectionDao, PgAdapterConnectionDao};
use moatless_core::config::Config;
use moatless_core::db::DbPool;
use moatless_core::events::EventPublisher;
use moatless_identity::adapter_apps::dao::AdapterAppDao;
use moatless_identity::adapter_apps::dao::PgAdapterAppDao;
use moatless_identity::auth::api_keys::ApiKeyDao;
use moatless_identity::auth::api_keys::PgApiKeyDao;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_identity::auth::pairing::PgPairingApiKeyIssuer;
use moatless_identity::auth::pairing::PgPairingCodeDao;
use moatless_identity::auth::password::dao::PgUserCredentialDao;
use moatless_identity::auth::password::dao::UserCredentialDao;
use moatless_identity::auth::provider_token_dao::PgProviderTokenDao;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::auth::provider_tokens::PgProviderTokenAuthDao;
use moatless_identity::auth::provider_tokens::ProviderTokenAuthDao;
use moatless_identity::auth::sessions::PgSessionDao;
use moatless_identity::auth::sessions::SessionDao;
use moatless_identity::auth::users::PgUserDao;
use moatless_identity::auth::users::UserDao;
use moatless_identity::git_host_tokens::GitHostTokens;
use moatless_identity::secret::audit::PgSecretAuditDao;
use moatless_identity::secret::audit::SecretAuditDao;
use moatless_identity::secret::dao::PgSecretDao;
use moatless_identity::secret::dao::SecretDao;
use moatless_identity::secret::service::LocalEnvelopeSecretCipher;
use moatless_identity::secret::service::SecretCipher;
use moatless_identity::secret::service::SecretService;
use moatless_identity::team::dao::PgTeamDao;
use moatless_identity::team::dao::TeamDao;
use moatless_loops::bindings::dao::PgTaskBindingDao;
use moatless_loops::bindings::dao::TaskBindingDao;
use moatless_loops::bindings::ports::BindingDirectory;
use moatless_loops::loops::dao::LoopDao;
use moatless_loops::loops::firing::receipts::EventReceiptDao;
use moatless_loops::loops::firing::receipts::PgEventReceiptDao;
use moatless_sandbox_runtime::fleet::FleetObserver;
use moatless_sandbox_runtime::runtime::SandboxRuntime;
use moatless_tasks::event_ingress::outbox::IngressOutbox;
use moatless_tasks::event_ingress::outbox::PgIngressOutbox;
use moatless_tasks::feedback::dao::FeedbackDao;
use moatless_tasks::feedback::dao::PgFeedbackDao;
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::message::dao::PgMessageDao;
use moatless_tasks::tag::dao::PgTagDao;
use moatless_tasks::tag::dao::TagDao;
use moatless_tasks::task::dao::PgTaskDao;
use moatless_tasks::task::dao::TaskDao;
use moatless_tasks::task::fork::PgTaskForkDao;
use moatless_tasks::task::fork::TaskForkDao;
use moatless_tasks::task::relations::PgTaskRelationDao;
use moatless_tasks::task::relations::TaskRelationDao;
use moatless_tasks::task_event::dao::PgTaskEventDao;
use moatless_tasks::task_event::dao::TaskEventDao;
use moatless_tasks::task_turn::dao::PgTaskTurnDao;
use moatless_tasks::task_turn::dao::TaskTurnDao;
use moatless_workspaces::plugin::dao::PgPluginDao;
use moatless_workspaces::plugin::dao::PluginDao;
use moatless_workspaces::repository::dao::PgRepositoryDao;
use moatless_workspaces::repository::dao::RepositoryDao;
use moatless_workspaces::workspace::credentials::dao::PgWorkspaceCredentialDao;
use moatless_workspaces::workspace::credentials::dao::WorkspaceCredentialDao;
use moatless_workspaces::workspace::credentials::service::WorkspaceCredentialService;
use moatless_workspaces::workspace::dao::PgWorkspaceDao;
use moatless_workspaces::workspace::dao::WorkspaceDao;
use sandbox_proto::client::SandboxConnector;

/// Bounded per-Task lock-acquire attempts before a lifecycle call surfaces
/// `OperationInProgress`. The inline retry is only a latency fast path — the
/// reconciler is the durable retry owner — so this stays small.
const LIFECYCLE_LOCK_ATTEMPTS: u32 = 3;

/// The ports one process has, plus the pool they were built from. Cheap to
/// clone — every field is `Arc`-wrapped or internally `Arc`'d, so each clone
/// shares the same instance and the same caches.
///
/// Configuration is the input to [`compose`], not part of its answer: a
/// consumer that needs a config value takes that value, so holding ports does
/// not widen what it can read.
#[derive(Clone)]
pub struct Ports {
    /// The main connection pool. Raw-pool services take it from here rather
    /// than opening their own.
    pub pool: DbPool,

    pub tasks: Arc<dyn TaskDao>,
    /// E-(b) session-gate port: the one `may_access_task` bool the auth
    /// `ForwardAuth` gate needs, over `TaskDao`, so identity holds no `task` import.
    pub task_access: Arc<dyn moatless_core::http_auth::TaskAccess>,
    pub task_relations: Arc<dyn TaskRelationDao>,
    /// Where a fork's first turn branches from. Read by the creation service
    /// that writes it and by the delivery path that spends it.
    pub task_forks: Arc<dyn TaskForkDao>,
    /// Task↔external-subject bindings (`task_bindings`).
    pub task_bindings: Arc<dyn TaskBindingDao>,
    /// The binding store and the Loops that label its rows, behind the
    /// `TaskBindings` seam the Task band reads. Also serves `BindingProbe`.
    pub binding_directory: Arc<BindingDirectory>,
    pub task_events: Arc<dyn TaskEventDao>,
    /// Authoritative per-turn lifecycle state (`task_turns`).
    pub task_turns: Arc<dyn TaskTurnDao>,
    pub messages: Arc<dyn MessageDao>,
    pub feedback: Arc<dyn FeedbackDao>,
    pub repositories: Arc<dyn RepositoryDao>,
    /// The narrowed read the task and loop bands take, over the same store.
    pub repository_reader: Arc<dyn moatless_ports::workspace::RepositoryReader>,
    /// Workspace composition + run-config (`workspaces` / `workspace_repos`).
    /// The `tasks` table behind the published `TaskReader` seam.
    pub task_reader: Arc<moatless_tasks::ports::TaskProjection>,
    /// The `task_messages` table behind the published `TranscriptAppend` seam.
    pub transcript: Arc<moatless_tasks::ports::Transcript>,
    /// The `task_turns` table behind the published `TurnCounter` seam.
    pub turn_counter: Arc<moatless_tasks::ports::TurnLedger>,
    /// The `tags` table behind the published `TagResolver` seam.
    pub tag_resolver: Arc<moatless_tasks::ports::TagCatalog>,
    pub workspaces: Arc<dyn WorkspaceDao>,
    /// The workspace, placement, and plugin stores behind the
    /// `WorkspaceResolver` port.
    pub workspace_resolver: Arc<moatless_workspaces::ports::WorkspaceProjection>,
    /// Provider credentials pinned to a Workspace (`workspace_credentials`).
    /// The highest-precedence source of a sandbox's provider credentials.
    pub workspace_credentials: Arc<WorkspaceCredentialService>,
    pub tags: Arc<dyn TagDao>,
    pub files: Arc<dyn FileDao>,
    pub teams: Arc<dyn TeamDao>,
    pub plugins: Arc<dyn PluginDao>,
    /// Single owner of persistence for the four Loop tables (`loops`,
    /// `loop_configs`, `subscriptions`, `schedules`).
    pub loops: Arc<LoopDao>,
    /// Adapter connections (`adapter_connections`).
    pub adapter_connections: Arc<dyn AdapterConnectionDao>,
    /// Durable adapter-event deduplication (`adapter_event_receipts`) — the
    /// binding resolver records a delivery here so a redelivered webhook does
    /// not double-post.
    pub event_receipts: Arc<dyn EventReceiptDao>,
    /// Transactional ingress outbox (the durable-ingress decision).
    pub ingress_outbox: Arc<dyn IngressOutbox>,
    /// Durable admission for inbound Adapter deliveries. Shared by Adapter
    /// ingress and the worker-facing application state.
    pub adapter_delivery: Arc<AdapterDeliveryStore>,

    pub users: Arc<dyn UserDao>,
    /// The `users` table behind the three identity ports peer crates read:
    /// `UserDisplay`, `UserResolver`, `CommitIdentity`.
    pub user_directory: Arc<moatless_identity::ports::UserDirectory>,
    /// The `provider_tokens` table behind the `HarnessCredentials` port.
    pub harness_credentials: Arc<moatless_identity::ports::HarnessTokens>,
    /// The `adapter_apps` tables behind the `AdapterAppReader` port an Adapter
    /// resolves its provider credentials through.
    pub adapter_apps: Arc<dyn moatless_adapter_model::apps::AdapterAppReader>,
    /// Local password credentials (`user_credentials`).
    pub user_credentials: Arc<dyn UserCredentialDao>,
    pub sessions: Arc<dyn SessionDao>,
    pub api_keys: Arc<dyn ApiKeyDao>,
    pub provider_tokens: Arc<dyn ProviderTokenDao>,
    pub provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    /// Verifies sandbox-issued user tokens behind a port, so the token secret
    /// and `sandbox::jwt` stay out of the auth handlers that consume it.
    pub token_verifier: Arc<dyn moatless_core::http_auth::UserTokenVerifier>,
    pub secret_records: Arc<dyn SecretDao>,
    pub secret_audits: Arc<dyn SecretAuditDao>,

    /// Secret validation, encryption and resolution over the two secret DAOs.
    pub secrets: Arc<SecretService>,
    /// Mints and redeems the codes that pair a client on another device.
    pub pairing: Arc<DevicePairing>,
    /// Every configured GitHub App. One instance per process, so every caller
    /// shares its installation-token cache.
    pub github_apps: Arc<GitHubAppRegistry>,
    /// Convenience handle to the default (user-auth) App — equivalent to
    /// `github_apps.default_service()`.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// The one way to a git-host token: a named identity and a repository,
    /// with no scope walk. Built here so the five credential ports it reads are
    /// supplied in one place.
    pub git_host_tokens: Arc<GitHostTokens>,

    /// The T3 UI-RPC read model (workspaces, conversations, tasks).
    pub ui_rpc_conversations: Arc<moatless_t3code::dao::PgConversationDao>,

    /// Durable Sandbox intent and per-Task operation state. Assembled into a
    /// [`Lifecycle`] by [`Ports::build_lifecycle`].
    pub(crate) lifecycle_store: Arc<dyn LifecycleDao>,
}

/// Build every port this process has from its pool and configuration.
pub fn compose(pool: DbPool, config: &Config) -> Ports {
    let cipher = LocalEnvelopeSecretCipher::from_config(&config.secrets);

    let secret_records: Arc<dyn SecretDao> = Arc::new(PgSecretDao(pool.clone()));
    let secret_audits: Arc<dyn SecretAuditDao> = Arc::new(PgSecretAuditDao(pool.clone()));
    let secrets = Arc::new(SecretService::new(
        secret_records.clone(),
        secret_audits.clone(),
        cipher.clone(),
    ));

    let workspaces: Arc<dyn WorkspaceDao> = Arc::new(PgWorkspaceDao(pool.clone()));
    let workspace_credential_records: Arc<dyn WorkspaceCredentialDao> =
        Arc::new(PgWorkspaceCredentialDao(pool.clone()));
    let workspace_credentials = Arc::new(WorkspaceCredentialService::new(
        workspace_credential_records,
        workspaces.clone(),
        secret_audits.clone(),
        cipher.clone(),
    ));

    let repositories: Arc<dyn RepositoryDao> = Arc::new(PgRepositoryDao(pool.clone()));
    let repository_reader: Arc<dyn moatless_ports::workspace::RepositoryReader> = Arc::new(
        moatless_workspaces::ports::RepositoryProjection::new(repositories.clone()),
    );
    let plugins: Arc<dyn PluginDao> = Arc::new(PgPluginDao(pool.clone()));
    let workspace_resolver = Arc::new(moatless_workspaces::ports::WorkspaceProjection::new(
        workspaces.clone(),
        repositories.clone(),
        plugins.clone(),
    ));

    let api_keys: Arc<dyn ApiKeyDao> = Arc::new(PgApiKeyDao(pool.clone()));
    let pairing = Arc::new(DevicePairing::new(
        Arc::new(PgPairingCodeDao(pool.clone())),
        Arc::new(PgPairingApiKeyIssuer(api_keys.clone())),
    ));

    let adapter_apps: Arc<dyn AdapterAppDao> = Arc::new(PgAdapterAppDao(pool.clone()));
    let github_apps = github_app_registry(config, Some(adapter_apps), cipher);
    let github_app_service = github_apps.default_service();

    let provider_tokens: Arc<dyn ProviderTokenDao> = Arc::new(PgProviderTokenDao(pool.clone()));
    let harness_credentials = Arc::new(moatless_identity::ports::HarnessTokens::new(
        provider_tokens.clone(),
    ));
    let provider_token_auth: Arc<dyn ProviderTokenAuthDao> =
        Arc::new(PgProviderTokenAuthDao(pool.clone()));
    let git_host_tokens = Arc::new(
        GitHostTokens::new(
            provider_tokens.clone(),
            provider_token_auth.clone(),
            github_app_service.clone(),
            github_apps.clone(),
            secrets.clone(),
        )
        .with_workspace_credentials(workspace_credentials.clone()),
    );

    // One PgTaskDao serves the TaskDao port, the E-(b) TaskAccess port, and the
    // published `TaskReader` seam peer crates read.
    let task_dao = Arc::new(PgTaskDao::new(
        pool.clone(),
        Arc::new(moatless_workspaces::ports::ShadowWorkspaces::new(
            pool.clone(),
        )),
    ));
    let task_reader = Arc::new(moatless_tasks::ports::TaskProjection::new(task_dao.clone()));

    let messages: Arc<dyn MessageDao> = Arc::new(PgMessageDao(pool.clone()));
    let transcript = Arc::new(moatless_tasks::ports::Transcript::new(messages.clone()));
    let task_turns: Arc<dyn TaskTurnDao> = Arc::new(PgTaskTurnDao(pool.clone()));
    let turn_counter = Arc::new(moatless_tasks::ports::TurnLedger::new(task_turns.clone()));
    let tags: Arc<dyn TagDao> = Arc::new(PgTagDao(pool.clone()));
    let tag_resolver = Arc::new(moatless_tasks::ports::TagCatalog::new(tags.clone()));

    // One PgUserDao serves the UserDao port and the three published identity
    // ports the `UserDirectory` narrows it to.
    let users: Arc<dyn UserDao> = Arc::new(PgUserDao(pool.clone()));

    // One PgTaskBindingDao serves the store the Adapters that drive the binding
    // lifecycle name, the `BindingProbe` predicate, and — joined to the Loop
    // store — the `TaskBindings` seam the Task band reads.
    let task_bindings: Arc<dyn TaskBindingDao> = Arc::new(PgTaskBindingDao::new(pool.clone()));
    let loops = Arc::new(LoopDao::new(pool.clone()));
    let binding_directory = Arc::new(BindingDirectory::new(task_bindings.clone(), loops.clone()));

    Ports {
        tasks: task_dao.clone(),
        task_access: task_dao,
        task_reader,
        task_relations: Arc::new(PgTaskRelationDao(pool.clone())),
        task_forks: Arc::new(PgTaskForkDao(pool.clone())),
        task_bindings,
        binding_directory,
        task_events: Arc::new(PgTaskEventDao(pool.clone())),
        task_turns,
        turn_counter,
        messages,
        transcript,
        feedback: Arc::new(PgFeedbackDao(pool.clone())),
        repositories,
        repository_reader,
        workspaces,
        workspace_resolver,
        workspace_credentials,
        tags,
        tag_resolver,
        files: Arc::new(PgFileDao(pool.clone())),
        teams: Arc::new(PgTeamDao(pool.clone())),
        plugins,
        loops,
        adapter_connections: Arc::new(PgAdapterConnectionDao::new(pool.clone())),
        event_receipts: Arc::new(PgEventReceiptDao::new(pool.clone())),
        ingress_outbox: Arc::new(PgIngressOutbox(pool.clone())),
        adapter_delivery: Arc::new(AdapterDeliveryStore::new(pool.clone())),

        users: users.clone(),
        user_directory: Arc::new(moatless_identity::ports::UserDirectory::new(
            users,
            provider_tokens.clone(),
        )),
        harness_credentials,
        adapter_apps: Arc::new(moatless_identity::ports::AdapterApps::new(
            pool.clone(),
            Arc::new(config.clone()),
        )),
        user_credentials: Arc::new(PgUserCredentialDao(pool.clone())),
        sessions: Arc::new(PgSessionDao(pool.clone())),
        api_keys,
        provider_tokens,
        provider_token_auth,
        token_verifier: Arc::new(moatless_core::sandbox_jwt::JwtUserTokenVerifier::new(
            config.sandbox.task_token_secret.clone(),
        )),
        secret_records,
        secret_audits,

        secrets,
        pairing,
        github_apps,
        github_app_service,
        git_host_tokens,

        ui_rpc_conversations: Arc::new(moatless_t3code::dao::PgConversationDao::new(
            pool.clone(),
            Arc::new(moatless_tasks::ports::Activities::new(pool.clone())),
        )),

        lifecycle_store: Arc::new(PgLifecycleDao::new(pool.clone())),

        pool,
    }
}

/// The registry of configured GitHub Apps: the deployment's login App, plus the
/// admin-registered Apps in `store`. A process with no database passes `None`
/// and gets the login App alone. `cipher` opens a stored App's signing key.
pub fn github_app_registry(
    config: &Config,
    store: Option<Arc<dyn AdapterAppDao>>,
    cipher: Option<Arc<dyn SecretCipher>>,
) -> Arc<GitHubAppRegistry> {
    Arc::new(GitHubAppRegistry::new(
        GitHubAppService::from_config(config).map(Arc::new),
        store,
        cipher,
    ))
}

/// The service-gated handles a lifecycle is built over, grouped because they
/// arrive from the same place — `main`'s Redis and S3 gates — and are each
/// `None` in a deployment that runs without the backing service.
pub struct LifecycleOptions {
    /// Event publication; `None` without Redis.
    pub events: Option<Arc<dyn EventPublisher>>,
    /// The workspace-archive reader the cold transition proves itself against;
    /// `None` without S3, which leaves the transition off.
    pub workspace_archive: Option<Arc<S3WorkspaceArchive>>,
}

impl Ports {
    /// Build the shared durable Sandbox lifecycle handle over its Postgres and
    /// runtime adapters. One instance is shared by `AppState`, the sandbox API,
    /// and the pod watcher so Sandbox intent writes and per-Task locks stay
    /// single-sourced. `lock_pool` must be the dedicated lock-only pool.
    /// The durable Sandbox store, for the plan knowledge and gRPC handlers that
    /// read a Sandbox row directly.
    pub fn lifecycle_store(&self) -> Arc<dyn LifecycleDao> {
        self.lifecycle_store.clone()
    }

    pub fn build_lifecycle(
        &self,
        lock_pool: DbPool,
        plan: ProvisionDeps,
        runtime: Arc<dyn SandboxRuntime>,
        connector: Arc<dyn SandboxConnector>,
        default_sandbox_image: String,
        optional: LifecycleOptions,
    ) -> Arc<Lifecycle> {
        let lifecycle = Lifecycle::new(
            self.lifecycle_store.clone(),
            runtime,
            Arc::new(GrpcAgentStop::new(connector.clone())),
            Arc::new(OutboxEventSink::new(optional.events)),
            Arc::new(PgLifecycleLock::new(lock_pool)),
            Arc::new(SystemWallClock),
        )
        .with_workspace_sync(Arc::new(GrpcWorkspaceSync::new(connector)))
        .with_plan_knowledge(plan, default_sandbox_image)
        .with_lock_attempts(LIFECYCLE_LOCK_ATTEMPTS);
        // Without S3 the cold transition stays off: `with_workspace_archive` is
        // what tells lifecycle a durable copy exists to reclaim toward.
        Arc::new(match optional.workspace_archive {
            Some(archive) => lifecycle.with_workspace_archive(archive),
            None => lifecycle,
        })
    }

    /// Assemble the desired-state reconciler over the lifecycle handle it drives
    /// operations through, and a fleet observer.
    pub fn build_reconciler(
        &self,
        lifecycle: Arc<Lifecycle>,
        observer: Arc<dyn FleetObserver>,
        notify: Arc<dyn NotifyPort>,
        member_registry: Arc<dyn MemberRegistryPort>,
        config: ReconcilerConfig,
    ) -> Reconciler {
        Reconciler::new(
            (*lifecycle).clone(),
            ReconcilerDeps {
                store: self.lifecycle_store.clone(),
                observer,
                notify,
                // The Task band owns `task_messages`, so it answers which
                // members of a shared Sandbox a wake has to reach.
                pending_work: Arc::new(moatless_tasks::ports::PendingWork::new(self.pool.clone())),
                member_registry,
                clock: Arc::new(SystemWallClock),
            },
            config,
        )
    }

    /// Build the auth capsule over the application-owned ports.
    pub fn build_authentication(
        &self,
        config: Arc<Config>,
    ) -> Result<moatless_identity::auth::Authentication, moatless_identity::auth::error::AuthError>
    {
        moatless_identity::auth::Authentication::build(
            config,
            self.pool.clone(),
            moatless_identity::auth::AuthenticationPorts {
                sessions: self.sessions.clone(),
                api_keys: self.api_keys.clone(),
                pairing: self.pairing.clone(),
                users: self.users.clone(),
                credentials: self.user_credentials.clone(),
                provider_tokens: self.provider_token_auth.clone(),
                provider_tokens_dao: self.provider_tokens.clone(),
                adapter_connections: self.adapter_connections.clone(),
                github_app_service: self.github_app_service.clone(),
                github_apps: self.github_apps.clone(),
                git_host_tokens: self.git_host_tokens.clone(),
                secrets: self.secrets.clone(),
                task_access: self.task_access.clone(),
                token_verifier: self.token_verifier.clone(),
            },
        )
    }
}
