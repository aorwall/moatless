//! The composition root.
//!
//! [`compose`] answers "given process configuration, what ports exist", and is
//! where the ports a process holds are constructed. A caller that receives
//! [`Ports`] takes the ports it needs rather than a pool and a `Pg*Dao`
//! constructor. Holding a [`DbPool`] is a separate question — raw-pool
//! services keep their own and are unaffected.

use std::sync::Arc;

use crate::adapter_apps::dao::{AdapterAppDao, PgAdapterAppDao};
use crate::adapters::dao::{AdapterConnectionDao, PgAdapterConnectionDao};
use crate::adapters::delivery::AdapterDeliveryHandlers;
use crate::auth::api_keys::{ApiKeyDao, PgApiKeyDao};
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::pairing::{DevicePairing, PgPairingApiKeyIssuer, PgPairingCodeDao};
use crate::auth::password::dao::{PgUserCredentialDao, UserCredentialDao};
use crate::auth::provider_token_dao::{PgProviderTokenDao, ProviderTokenDao};
use crate::auth::provider_tokens::{PgProviderTokenAuthDao, ProviderTokenAuthDao};
use crate::auth::sessions::{PgSessionDao, SessionDao};
use crate::auth::users::{PgUserDao, UserDao};
use crate::bindings::dao::{PgTaskBindingDao, TaskBindingDao};
use moatless_core::config::Config;
use moatless_core::db::DbPool;
use crate::event_ingress::outbox::{IngressOutbox, PgIngressOutbox};
use moatless_core::events::EventPublisher;
use crate::feedback::dao::{FeedbackDao, PgFeedbackDao};
use crate::file_storage::dao::{FileDao, PgFileDao};
use crate::loops::dao::LoopDao;
use crate::loops::firing::receipts::{EventReceiptDao, PgEventReceiptDao};
use crate::message::dao::{MessageDao, PgMessageDao};
use crate::plugin::dao::{PgPluginDao, PluginDao};
use crate::repository::dao::{PgRepositoryDao, RepositoryDao};
use crate::sandbox::lifecycle::orchestrator::LifecycleDao;
use crate::sandbox::lifecycle::runtime::SandboxRuntime;
use crate::sandbox::lifecycle::{
    FleetObserver, GrpcAgentStop, Lifecycle, NotifyPort, OutboxEventSink, PgLifecycleDao,
    PgLifecycleLock, ProvisionDeps, Reconciler, ReconcilerConfig, SystemWallClock,
};
use crate::secret::audit::{PgSecretAuditDao, SecretAuditDao};
use crate::secret::dao::{PgSecretDao, SecretDao};
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretCipher, SecretService};
use crate::state::AppState;
use moatless_core::tag::dao::{PgTagDao, TagDao};
use crate::task::dao::{PgTaskDao, TaskDao};
use crate::task::relations::{PgTaskRelationDao, TaskRelationDao};
use crate::task_event::dao::{PgTaskEventDao, TaskEventDao};
use crate::task_turn::dao::{PgTaskTurnDao, TaskTurnDao};
use crate::team::dao::{PgTeamDao, TeamDao};
use crate::workspace::dao::{PgWorkspaceDao, WorkspaceDao};
use moatless_backend_core::sandbox::SandboxConnector;

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
    /// ForwardAuth gate needs, over `TaskDao`, so identity holds no `task` import.
    pub task_access: Arc<dyn crate::extractors::TaskAccess>,
    pub task_relations: Arc<dyn TaskRelationDao>,
    /// Task↔external-subject bindings (`task_bindings`).
    pub task_bindings: Arc<dyn TaskBindingDao>,
    pub task_events: Arc<dyn TaskEventDao>,
    /// Authoritative per-turn lifecycle state (`task_turns`).
    pub task_turns: Arc<dyn TaskTurnDao>,
    pub messages: Arc<dyn MessageDao>,
    pub feedback: Arc<dyn FeedbackDao>,
    pub repositories: Arc<dyn RepositoryDao>,
    /// Workspace composition + run-config (`workspaces` / `workspace_repos`).
    pub workspaces: Arc<dyn WorkspaceDao>,
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

    pub users: Arc<dyn UserDao>,
    /// Local password credentials (`user_credentials`).
    pub user_credentials: Arc<dyn UserCredentialDao>,
    pub sessions: Arc<dyn SessionDao>,
    pub api_keys: Arc<dyn ApiKeyDao>,
    pub provider_tokens: Arc<dyn ProviderTokenDao>,
    pub provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    /// Verifies sandbox-issued user tokens behind a port, so the token secret
    /// and `sandbox::jwt` stay out of the auth handlers that consume it.
    pub token_verifier: Arc<dyn crate::extractors::UserTokenVerifier>,
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

    let api_keys: Arc<dyn ApiKeyDao> = Arc::new(PgApiKeyDao(pool.clone()));
    let pairing = Arc::new(DevicePairing::new(
        Arc::new(PgPairingCodeDao(pool.clone())),
        Arc::new(PgPairingApiKeyIssuer(api_keys.clone())),
    ));

    let adapter_apps: Arc<dyn AdapterAppDao> = Arc::new(PgAdapterAppDao(pool.clone()));
    let github_apps = github_app_registry(config, Some(adapter_apps), cipher);
    let github_app_service = github_apps.default_service();

    // One PgTaskDao serves both the TaskDao port and the E-(b) TaskAccess port.
    let task_dao = Arc::new(PgTaskDao(pool.clone()));

    Ports {
        tasks: task_dao.clone(),
        task_access: task_dao,
        task_relations: Arc::new(PgTaskRelationDao(pool.clone())),
        task_bindings: Arc::new(PgTaskBindingDao::new(pool.clone())),
        task_events: Arc::new(PgTaskEventDao(pool.clone())),
        task_turns: Arc::new(PgTaskTurnDao(pool.clone())),
        messages: Arc::new(PgMessageDao(pool.clone())),
        feedback: Arc::new(PgFeedbackDao(pool.clone())),
        repositories: Arc::new(PgRepositoryDao(pool.clone())),
        workspaces: Arc::new(PgWorkspaceDao(pool.clone())),
        tags: Arc::new(PgTagDao(pool.clone())),
        files: Arc::new(PgFileDao(pool.clone())),
        teams: Arc::new(PgTeamDao(pool.clone())),
        plugins: Arc::new(PgPluginDao(pool.clone())),
        loops: Arc::new(LoopDao::new(pool.clone())),
        adapter_connections: Arc::new(PgAdapterConnectionDao::new(pool.clone())),
        event_receipts: Arc::new(PgEventReceiptDao::new(pool.clone())),
        ingress_outbox: Arc::new(PgIngressOutbox(pool.clone())),

        users: Arc::new(PgUserDao(pool.clone())),
        user_credentials: Arc::new(PgUserCredentialDao(pool.clone())),
        sessions: Arc::new(PgSessionDao(pool.clone())),
        api_keys,
        provider_tokens: Arc::new(PgProviderTokenDao(pool.clone())),
        provider_token_auth: Arc::new(PgProviderTokenAuthDao(pool.clone())),
        token_verifier: Arc::new(crate::sandbox::jwt::JwtUserTokenVerifier::new(
            config.sandbox.task_token_secret.clone(),
        )),
        secret_records,
        secret_audits,

        secrets,
        pairing,
        github_apps,
        github_app_service,

        ui_rpc_conversations: Arc::new(moatless_t3code::dao::PgConversationDao::new(pool.clone())),

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

/// Register every built-in provider's durable delivery handlers.
///
/// Each provider's `delivery::register` is named here at the composition root
/// rather than in the delivery layer, so the layer stays unaware of which
/// concrete providers exist.
pub fn register_builtin_delivery_handlers(
    handlers: &mut AdapterDeliveryHandlers,
    state: &AppState,
) {
    crate::adapters::slack::delivery::register(handlers, state);
    crate::adapters::telegram::delivery::register(handlers, state);
    crate::adapters::github_pr::delivery::register(handlers, state);
    crate::adapters::linear::delivery::register(handlers, state);
    crate::adapters::webhook::delivery::register(handlers, state);
}

impl Ports {
    /// Build the shared durable Sandbox lifecycle handle over its Postgres and
    /// runtime adapters. One instance is shared by `AppState`, the sandbox API,
    /// and the pod watcher so Sandbox intent writes and per-Task locks stay
    /// single-sourced. `lock_pool` must be the dedicated lock-only pool.
    pub fn build_lifecycle(
        &self,
        lock_pool: DbPool,
        plan: ProvisionDeps,
        runtime: Arc<dyn SandboxRuntime>,
        connector: Arc<dyn SandboxConnector>,
        default_sandbox_image: String,
        events: Option<Arc<dyn EventPublisher>>,
    ) -> Arc<Lifecycle> {
        Arc::new(
            Lifecycle::new(
                self.lifecycle_store.clone(),
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

    /// Assemble the desired-state reconciler over the lifecycle handle it drives
    /// operations through, and a fleet observer.
    pub fn build_reconciler(
        &self,
        lifecycle: Arc<Lifecycle>,
        observer: Arc<dyn FleetObserver>,
        notify: Arc<dyn NotifyPort>,
        config: ReconcilerConfig,
    ) -> Reconciler {
        Reconciler::new(
            (*lifecycle).clone(),
            self.lifecycle_store.clone(),
            self.messages.clone(),
            observer,
            notify,
            Arc::new(SystemWallClock),
            config,
        )
    }
}
