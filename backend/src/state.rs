//! Unified application state for API routes.
//!
//! All handler-facing database access goes through DAO trait objects,
//! enabling full unit-test mocking without a real database. They are built by
//! the composition root ([`crate::compose`]) and handed to [`AppState::new`].

use std::sync::Arc;

use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::delivery::AdapterDeliveryStore;
use crate::adapters::providers::linear::active_turns::ActiveLinearTurns;
use crate::adapters::Adapters;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::ProviderTokenAuthDao;
use crate::auth::users::UserDao;
use crate::task::binding_dao::TaskBindingDao;
use crate::bindings::recently_sent::RecentlySent;
use crate::compose::Ports;
use crate::credential_sync::CredentialSync;
use crate::credentials::CredentialService;
use crate::event_ingress::outbox::IngressOutbox;
use crate::feedback::dao::FeedbackDao;
use crate::file_storage::dao::FileDao;
use crate::loops::dao::LoopDao;
use crate::loops::sync::LoopSyncService;
use crate::message::dao::MessageDao;
use crate::adapters::self_echo::SelfEchoFilter;
use crate::message::send::MessageSendService;
use crate::plugin::dao::PluginDao;
use crate::plugin::sync::PluginSyncService;
use crate::repository::access::RepositoryTokenResolver;
use crate::repository::dao::RepositoryDao;
use crate::sandbox::lifecycle::{Lifecycle, NotifyPort};
use crate::sandbox::preflight::{PreflightCredentials, RealGithubValidator, RealPreflight};
use crate::secret::dao::SecretDao;
use crate::secret::service::SecretService;
use crate::skill::fetcher::{GitHubSkillFetcher, SkillFetcher};
use crate::skill::service::SkillService;
use crate::task::closer::TaskCloser;
use crate::task::creation::{PreflightPort, TaskCreationService};
use crate::task::dao::TaskDao;
use crate::task::presenter::TaskPresenter;
use crate::task::relations::TaskRelationDao;
use crate::task::sandbox_lifecycle::SandboxLifecycle;
use crate::task_event::dao::TaskEventDao;
use crate::task_turn::dao::TaskTurnDao;
use crate::team::dao::TeamDao;
use crate::workspace::dao::WorkspaceDao;
use crate::workspace::git_sync::GitSyncService;
use moatless_backend_core::sandbox::SandboxConnector;
use moatless_core::config::{AuthConfig, CodexOAuthConfig, Config, S3Config, SecretsConfig};
use moatless_core::db::DbPool;
use moatless_core::events::EventPublisher;
use moatless_core::tag::dao::TagDao;

/// The concrete per-provider handles the composition root owns, projected to a
/// provider's own handlers by `FromRef<AppState>`.
///
/// A provider's handlers need methods its concrete adapter has and the `Adapter`
/// trait does not, so they take `State(p): State<Option<Arc<ConcreteAdapter>>>`
/// off the root rather than reaching back through the registry that holds the
/// trait object. Each is `Some` exactly when the registry has that provider;
/// `from_config` fills all four, and a build with no adapters leaves all four
/// `None`.
///
/// This lives in the server band, not the loops-band registry: naming a
/// concrete provider type is the composition root's privilege, so the registry
/// that holds only `dyn Adapter` never reaches up to a provider band.
#[derive(Clone, Default)]
pub struct ProviderHandles {
    pub github_pr: Option<Arc<crate::adapters::providers::github_pr::adapter::GithubPrAdapter>>,
    pub linear: Option<Arc<crate::adapters::providers::linear::adapter::LinearAdapter>>,
    pub slack: Option<Arc<crate::adapters::providers::slack::adapter::SlackAdapter>>,
    pub telegram: Option<Arc<crate::adapters::providers::telegram::adapter::TelegramAdapter>>,
}

/// Shared application state for task/message/agent/file routes.
///
/// All fields are cheap to clone (Arc-wrapped or internally Arc'd).
/// DAO trait objects enable handler unit tests without Postgres.
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool — kept for auth routes and infrastructure
    /// code that constructs its own state (gRPC server, idle reaper, etc.).
    pub db: DbPool,
    /// Immutable configuration shared across handlers.
    pub config: Arc<Config>,
    /// Config sections handed to migrated handlers as `Arc<Section>` ports,
    /// each built once here. Handing out `Arc<Section>` (not the section by
    /// value) keeps a secret-bearing section — `AuthConfig`, `SecretsConfig` —
    /// from being re-cloned on every request. One field per section a handler
    /// names.
    pub auth_config: Arc<AuthConfig>,
    pub codex_oauth_config: Arc<CodexOAuthConfig>,
    pub secrets_config: Arc<SecretsConfig>,
    pub s3_config: Arc<S3Config>,
    /// Verifies sandbox-issued `X-User-Token` JWTs. The auth extractors reach
    /// this through [`crate::extractors::UserTokenVerifier`] rather than reading
    /// the token secret off `config` directly.
    pub token_verifier: Arc<dyn crate::extractors::UserTokenVerifier>,
    /// The one Sandbox value on `AppState`: the durable lifecycle handle. Callers
    /// request transitions (`start`/`stop`/`restart`/`redeploy`/`remove`) and read
    /// desired state through it rather than driving a provider directly.
    pub sandbox: Arc<Lifecycle>,
    /// Consumer-owned wake port: ensure a task's sandbox is running and notify
    /// it of new work. Declared in the tasks band
    /// ([`crate::task::sandbox_waker::SandboxWaker`]) and implemented in
    /// sandbox-control ([`crate::sandbox::notify::SandboxWakeService`]), so the
    /// message-delivery consumer wakes a sandbox through
    /// [`crate::task::sandbox_waker::wake_for_message`] without naming the sandbox
    /// layer above it. Built in [`AppState::new`] from the connector-backed notify
    /// port, which `AppState` takes as a constructor parameter but does not hold as
    /// a field — nothing reads the raw notifier off state.
    pub sandbox_waker: Arc<dyn crate::task::sandbox_waker::SandboxWaker>,
    /// Optional event publisher for real-time event streaming (trait object).
    pub events: Option<Arc<dyn EventPublisher>>,
    /// Optional S3 handle for file uploads/downloads.
    pub s3: Option<crate::file_storage::s3::S3Storage>,
    /// Optional external adapters.
    pub adapters: Option<Arc<Adapters>>,
    /// The concrete per-provider adapter handles, projected to each provider's
    /// own handlers by `FromRef<AppState>`. Each field is `Some` exactly when
    /// `adapters` holds that provider; all four are `None` when there are no
    /// adapters. See [`ProviderHandles`].
    pub provider_handles: ProviderHandles,
    /// Optional GitHub App service for installation tokens and user token refresh.
    ///
    /// Convenience handle to the default (user-auth) App. Equivalent to
    /// `github_apps.default_service()`. Kept for back-compat with existing
    /// call sites; new code that needs to pick an App by key should go
    /// through `github_apps` instead.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// Registry of all configured GitHub Apps (the default user-auth App plus
    /// any admin-registered bot Apps, read from storage). Bot users' provider
    /// tokens reference an App by `key` (stored in `provider_user_id`).
    pub github_apps: Arc<GitHubAppRegistry>,
    /// Codex credential orchestrator (tasks band) — wraps the pure credential
    /// service with the in-flight guard and the rotation broadcast to running
    /// sandboxes (9c).
    pub credential_sync: Arc<CredentialSync>,
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
    // DAO trait objects
    // =========================================================================
    pub tasks: Arc<dyn TaskDao>,
    /// Single owner of the create-then-provision sequence (short-id, preflight,
    /// insert, tags, relations, provision, first message, event).
    pub task_creation: Arc<TaskCreationService>,
    /// GitHub-token preflight, assembled once at the composition root. The
    /// task-start handler calls the same port `task_creation` runs rather than
    /// reassembling the credential handles and validator per request.
    pub preflight: Arc<dyn PreflightPort>,
    pub task_relations: Arc<dyn TaskRelationDao>,
    /// Task↔external-subject bindings (`task_bindings`) — the return channel a
    /// task replies on. Handlers read and write bindings through this port.
    pub task_bindings: Arc<dyn TaskBindingDao>,
    /// Adapter connections (`adapter_connections`) — the per-provider
    /// credential/webhook-secret rows inbound and outbound calls resolve
    /// against. Handlers read and write connections through this port.
    pub adapter_connections: Arc<dyn AdapterConnectionDao>,
    /// The inbound-routing subgraph and the ten ports it reads — including the
    /// durable adapter-event dedup (`adapter_event_receipts`), which nothing
    /// outside routing reads, so it lives here rather than as its own field. The
    /// durable inbound-delivery handler and the GitHub PR subject router route
    /// through this handle instead of naming each port. See
    /// [`crate::bindings::runtime::RoutingService`].
    pub routing: Arc<crate::bindings::runtime::RoutingService>,
    pub external_reply: Arc<crate::bindings::reply::ExternalReplyService>,
    /// Single owner of persistence for the four Loop tables (`loops`,
    /// `loop_configs`, `subscriptions`,
    /// `schedules`).
    pub loops: Arc<LoopDao>,
    pub messages: Arc<dyn MessageDao>,
    pub feedback: Arc<dyn FeedbackDao>,
    pub repositories: Arc<dyn RepositoryDao>,
    /// Resolves a repository's GitHub token from the credential ports, for the
    /// operations that carry no run-as identity (the repository scope selects).
    pub repo_token_resolver: Arc<RepositoryTokenResolver>,
    pub tags: Arc<dyn TagDao>,
    pub files: Arc<dyn FileDao>,
    pub users: Arc<dyn UserDao>,
    pub provider_tokens: Arc<dyn ProviderTokenDao>,
    pub provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    pub secret_records: Arc<dyn SecretDao>,
    pub secrets: Arc<SecretService>,
    pub teams: Arc<dyn TeamDao>,
    pub task_events: Arc<dyn TaskEventDao>,
    /// Transactional ingress outbox (the durable-ingress decision). The event-handler consumer
    /// deletes a record here once it has committed the message that record
    /// carries — the only confirmation of delivery in the system.
    pub ingress_outbox: Arc<dyn IngressOutbox>,
    /// Authoritative per-turn lifecycle state (`task_turns`). Written
    /// transactionally at the gRPC ingress; the reaper reads it as the backstop.
    pub task_turns: Arc<dyn TaskTurnDao>,
    pub plugins: Arc<dyn PluginDao>,
    /// The one path from a plugin's git content to its stored skills. Every
    /// reader of plugin skills goes through this.
    pub plugin_sync: Arc<PluginSyncService>,
    /// Workspace composition + run-config (`workspaces` / `workspace_repos`).
    /// Provisioning resolves a task's repository composition through this.
    pub workspaces: Arc<dyn WorkspaceDao>,
    /// Reconciles a repository's git-declared workspaces; holds the ports the
    /// reconcile tree would otherwise thread through every frame.
    pub git_sync: Arc<GitSyncService>,
    /// Reconciles a repository's git-declared loops; holds the four ports the
    /// reconcile tree would otherwise thread through every frame.
    pub loop_sync: Arc<LoopSyncService>,
    /// The message-send flows over the ports they read; holds the self-echo
    /// dedup filter so the message domain no longer names the adapter registry.
    pub message_send: Arc<MessageSendService>,
    /// Presents a task row as a `TaskResponse`; holds the eight ports the
    /// response is assembled from so presenting handlers name one service
    /// instead of eight DAOs.
    pub task_presenter: Arc<TaskPresenter>,
    pub task_closer: Arc<TaskCloser>,
    /// Skill discovery service — reads `.claude/skills/` from workspace
    /// and plugins, parses frontmatter, caches results.
    pub skills: Arc<SkillService>,
    /// Durable admission for every inbound Adapter delivery, whose work cannot
    /// finish on the HTTP response path. A webhook handler admits through this
    /// before it acknowledges its sender; the supervised
    /// `adapter-delivery-worker` owns what was admitted. See
    /// [`crate::adapters::delivery`].
    pub adapter_delivery: Arc<AdapterDeliveryStore>,
}

impl AppState {
    /// Build `AppState` over the ports the composition root assembled.
    ///
    /// The services wired here are the ones that need a runtime handle the
    /// composition root does not have — the sandbox connector and the lifecycle
    /// handle — so they are assembled from `ports` rather than in it.
    #[expect(
        clippy::too_many_arguments,
        reason = "nine independent infrastructure handles — ports, config, \
                  lifecycle, connector, sandbox_notify, events, s3, adapters, \
                  provider_handles — each wired in from the composition root; \
                  bundling them behind a one-use struct only shuffles the same \
                  fields. provider_handles is itself a bundle, but not that \
                  shuffle: each of its four fields has a FromRef consumer that \
                  takes exactly one, which earns it a struct where a \
                  hypothetical AppStateNewArgs would not."
    )]
    pub fn new(
        ports: &Ports,
        config: Arc<Config>,
        lifecycle: Arc<Lifecycle>,
        connector: Arc<dyn SandboxConnector>,
        sandbox_notify: Arc<dyn NotifyPort>,
        events: Option<Arc<dyn EventPublisher>>,
        s3: Option<crate::file_storage::s3::S3Storage>,
        adapters: Option<Arc<Adapters>>,
        provider_handles: ProviderHandles,
    ) -> Self {
        let Ports {
            pool,
            tasks,
            task_relations,
            task_bindings,
            task_events,
            task_turns,
            messages,
            feedback,
            repositories,
            workspaces,
            tags,
            files,
            teams,
            plugins,
            loops,
            adapter_connections,
            event_receipts,
            ingress_outbox,
            users,
            provider_tokens,
            provider_token_auth,
            secret_records,
            secrets,
            github_apps,
            github_app_service,
            token_verifier,
            ..
        } = ports.clone();

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
            CredentialService::new(provider_tokens.clone())
                .with_codex_refresh(pool.clone(), codex_http)
                .with_secrets(secrets.clone()),
        );
        // Tasks-band orchestrator (9c): the in-flight guard + sandbox fan-out
        // that reach up out of the workspaces-band credential service.
        let credential_sync = Arc::new(CredentialSync::new(
            credentials_svc.clone(),
            tasks.clone(),
            connector,
        ));

        let repo_token_resolver = Arc::new(RepositoryTokenResolver::new(
            provider_tokens.clone(),
            provider_token_auth.clone(),
            github_app_service.clone(),
            github_apps.clone(),
            secrets.clone(),
        ));

        let git_sync = Arc::new(GitSyncService::new(
            pool.clone(),
            workspaces.clone(),
            repositories.clone(),
            provider_tokens.clone(),
            repo_token_resolver.clone(),
        ));

        let loop_sync = Arc::new(LoopSyncService::new(
            adapter_connections.clone(),
            loops.clone(),
            tags.clone(),
            repo_token_resolver.clone(),
        ));

        // Hoisted so the self-echo filter and the struct literal share one cache.
        let recently_sent = Arc::new(RecentlySent::new());
        let message_send = Arc::new(MessageSendService::new(
            messages.clone(),
            tasks.clone(),
            task_turns.clone(),
            files.clone(),
            Arc::new(SelfEchoFilter::new(recently_sent.clone(), adapters.clone())),
        ));

        let task_presenter = Arc::new(TaskPresenter::new(
            tags.clone(),
            repositories.clone(),
            workspaces.clone(),
            users.clone(),
            task_bindings.clone(),
            loops.clone(),
            task_relations.clone(),
            tasks.clone(),
        ));

        let task_closer = Arc::new(TaskCloser::new(
            tasks.clone(),
            task_relations.clone(),
            config.clone(),
            lifecycle.clone(),
            task_bindings.clone(),
            events.clone(),
        ));

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
        let plugin_sync = Arc::new(PluginSyncService::new(
            plugins.clone(),
            skill_fetcher.clone(),
            provider_tokens.clone(),
            provider_token_auth.clone(),
            github_app_service.clone(),
            github_apps.clone(),
            secrets.clone(),
        ));
        let skills = Arc::new(SkillService::new(
            repositories.clone(),
            plugins.clone(),
            provider_tokens.clone(),
            provider_token_auth.clone(),
            github_app_service.clone(),
            github_apps.clone(),
            secrets.clone(),
            skill_fetcher,
            plugin_sync.clone(),
            std::time::Duration::from_secs(config.skill.cache_ttl_secs),
        ));

        // Wire GitHub-token preflight once here so every task-creation path
        // (HTTP, loops, adapters) runs the same check. Held on `AppState` as the
        // `PreflightPort` so the task-start handler calls the same port
        // `task_creation` does rather than reassembling the check per request.
        let preflight: Arc<dyn PreflightPort> = Arc::new(RealPreflight::new(
            PreflightCredentials {
                secrets: secrets.clone(),
                provider_tokens: provider_tokens.clone(),
                github_app_service: github_app_service.clone(),
                provider_token_auth: provider_token_auth.clone(),
                github_apps: github_apps.clone(),
                users: users.clone(),
                credential_sync: credential_sync.clone(),
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
            preflight.clone(),
        ));

        // Config-section ports, built once so a secret-bearing section is never
        // re-cloned per request. `config` still holds the whole tree for
        // unmigrated call sites.
        let auth_config = Arc::new(config.auth.clone());
        let codex_oauth_config = Arc::new(config.codex_oauth.clone());
        let secrets_config = Arc::new(config.secrets.clone());
        let s3_config = Arc::new(config.s3.clone());

        // The tasks-band wake port, implemented in sandbox-control. Built here
        // because it needs the task DAO, the lifecycle handle, and the notify
        // port — the same three the composition root already assembled.
        let sandbox_waker: Arc<dyn crate::task::sandbox_waker::SandboxWaker> =
            Arc::new(crate::sandbox::notify::SandboxWakeService::new(
                tasks.clone(),
                lifecycle.clone(),
                sandbox_notify.clone(),
            ));

        // The inbound-routing subgraph owns the ten ports it reads, so the
        // routing logic in `bindings/runtime.rs` names none of them through
        // `AppState`. Built here from the ports the composition root already
        // assembled; `event_receipts` moves in whole because nothing else on
        // `AppState` reads it.
        let routing = Arc::new(crate::bindings::runtime::RoutingService::new(
            adapter_connections.clone(),
            loops.clone(),
            task_bindings.clone(),
            messages.clone(),
            tasks.clone(),
            task_creation.clone(),
            message_send.clone(),
            event_receipts,
            sandbox_waker.clone(),
            adapters.clone(),
        ));

        // Outbound reply delivery owns the ports its send path reads. The
        // `OutboundSender` is built here and handed to `ExternalReplyService`
        // (the SelfEchoFilter/RoutingService pattern), so neither struct's
        // constructor takes more than the ports it holds and neither names
        // `AppState`.
        let outbound_sender = Arc::new(crate::bindings::outbound::OutboundSender::new(
            task_bindings.clone(),
            adapter_connections.clone(),
            loops.clone(),
            tasks.clone(),
            recently_sent.clone(),
            messages.clone(),
            task_turns.clone(),
            s3.clone(),
            auth_config.clone(),
            s3_config.clone(),
            files.clone(),
        ));
        let external_reply = Arc::new(crate::bindings::reply::ExternalReplyService::new(
            adapters.clone(),
            task_bindings.clone(),
            adapter_connections.clone(),
            messages.clone(),
            tasks.clone(),
            loops.clone(),
            recently_sent.clone(),
            task_turns.clone(),
            outbound_sender,
        ));

        Self {
            tasks,
            task_creation,
            preflight,
            task_relations,
            task_bindings,
            adapter_connections,
            routing,
            external_reply,
            loops,
            messages,
            feedback,
            repositories,
            repo_token_resolver,
            tags,
            files,
            users,
            provider_tokens,
            provider_token_auth,
            secret_records,
            secrets,
            teams,
            task_events,
            ingress_outbox,
            task_turns,
            plugins,
            plugin_sync,
            workspaces,
            git_sync,
            loop_sync,
            message_send,
            task_presenter,
            task_closer,
            skills,
            adapter_delivery: Arc::new(AdapterDeliveryStore::new(pool.clone())),
            db: pool,
            config,
            auth_config,
            codex_oauth_config,
            secrets_config,
            s3_config,
            token_verifier,
            sandbox: lifecycle,
            sandbox_waker,
            events,
            s3,
            adapters,
            provider_handles,
            github_app_service,
            github_apps,
            credential_sync,
            recently_sent,
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

// `FromRef` lets a handler extract exactly the port it names —
// `State(port): State<Arc<dyn Port>>` — from the one router-held `AppState`,
// so handlers stop naming `AppState`. `State<AppState>` still resolves through
// axum's blanket `FromRef<S> for S`, so migrated and unmigrated handlers share
// one router. One impl is added here per port as its module migrates.

impl axum::extract::FromRef<AppState> for Arc<dyn crate::extractors::UserTokenVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.token_verifier.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<SecretService> {
    fn from_ref(state: &AppState) -> Self {
        state.secrets.clone()
    }
}

impl axum::extract::FromRef<AppState> for Option<crate::file_storage::s3::S3Storage> {
    fn from_ref(state: &AppState) -> Self {
        state.s3.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn FileDao> {
    fn from_ref(state: &AppState) -> Self {
        state.files.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn FeedbackDao> {
    fn from_ref(state: &AppState) -> Self {
        state.feedback.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TaskDao> {
    fn from_ref(state: &AppState) -> Self {
        state.tasks.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TaskEventDao> {
    fn from_ref(state: &AppState) -> Self {
        state.task_events.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TagDao> {
    fn from_ref(state: &AppState) -> Self {
        state.tags.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TeamDao> {
    fn from_ref(state: &AppState) -> Self {
        state.teams.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn WorkspaceDao> {
    fn from_ref(state: &AppState) -> Self {
        state.workspaces.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn RepositoryDao> {
    fn from_ref(state: &AppState) -> Self {
        state.repositories.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<GitSyncService> {
    fn from_ref(state: &AppState) -> Self {
        state.git_sync.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<LoopSyncService> {
    fn from_ref(state: &AppState) -> Self {
        state.loop_sync.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<RepositoryTokenResolver> {
    fn from_ref(state: &AppState) -> Self {
        state.repo_token_resolver.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<SkillService> {
    fn from_ref(state: &AppState) -> Self {
        state.skills.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn PluginDao> {
    fn from_ref(state: &AppState) -> Self {
        state.plugins.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<PluginSyncService> {
    fn from_ref(state: &AppState) -> Self {
        state.plugin_sync.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<AuthConfig> {
    fn from_ref(state: &AppState) -> Self {
        state.auth_config.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<CodexOAuthConfig> {
    fn from_ref(state: &AppState) -> Self {
        state.codex_oauth_config.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<SecretsConfig> {
    fn from_ref(state: &AppState) -> Self {
        state.secrets_config.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<S3Config> {
    fn from_ref(state: &AppState) -> Self {
        state.s3_config.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn ProviderTokenDao> {
    fn from_ref(state: &AppState) -> Self {
        state.provider_tokens.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn SecretDao> {
    fn from_ref(state: &AppState) -> Self {
        state.secret_records.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn AdapterConnectionDao> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_connections.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<GitHubAppRegistry> {
    fn from_ref(state: &AppState) -> Self {
        state.github_apps.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<CredentialSync> {
    fn from_ref(state: &AppState) -> Self {
        state.credential_sync.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<Config> {
    fn from_ref(state: &AppState) -> Self {
        state.config.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn SandboxLifecycle> {
    fn from_ref(state: &AppState) -> Self {
        state.sandbox.clone()
    }
}

impl axum::extract::FromRef<AppState> for Option<Arc<dyn EventPublisher>> {
    fn from_ref(state: &AppState) -> Self {
        state.events.clone()
    }
}

impl axum::extract::FromRef<AppState> for Option<Arc<Adapters>> {
    fn from_ref(state: &AppState) -> Self {
        state.adapters.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskCreationService> {
    fn from_ref(state: &AppState) -> Self {
        state.task_creation.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn PreflightPort> {
    fn from_ref(state: &AppState) -> Self {
        state.preflight.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TaskRelationDao> {
    fn from_ref(state: &AppState) -> Self {
        state.task_relations.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TaskBindingDao> {
    fn from_ref(state: &AppState) -> Self {
        state.task_bindings.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<LoopDao> {
    fn from_ref(state: &AppState) -> Self {
        state.loops.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<AdapterDeliveryStore> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_delivery.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn MessageDao> {
    fn from_ref(state: &AppState) -> Self {
        state.messages.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn UserDao> {
    fn from_ref(state: &AppState) -> Self {
        state.users.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TaskTurnDao> {
    fn from_ref(state: &AppState) -> Self {
        state.task_turns.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<crate::bindings::reply::ExternalReplyService> {
    fn from_ref(state: &AppState) -> Self {
        state.external_reply.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<MessageSendService> {
    fn from_ref(state: &AppState) -> Self {
        state.message_send.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn crate::task::sandbox_waker::SandboxWaker> {
    fn from_ref(state: &AppState) -> Self {
        state.sandbox_waker.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskPresenter> {
    fn from_ref(state: &AppState) -> Self {
        state.task_presenter.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskCloser> {
    fn from_ref(state: &AppState) -> Self {
        state.task_closer.clone()
    }
}

// Naming `DbPool` gives a handler the pool for a raw-pool query function that
// predates the DAO layer (the sites Step 3 left alone). It is a named residue,
// not a grant to run new SQL from a handler — new persistence goes through a DAO
// port constructed in the composition root. See `.plans/backend-ports-and-crates.md`
// Step 5, `AppState.db`.
impl axum::extract::FromRef<AppState> for DbPool {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

// Concrete per-provider handles. Each provider's own handlers name the one they
// need; `None` when there are no adapters, which those handlers answer with
// `AdapterNotConfigured`.
impl axum::extract::FromRef<AppState>
    for Option<Arc<crate::adapters::providers::github_pr::adapter::GithubPrAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.provider_handles.github_pr.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<crate::adapters::providers::linear::adapter::LinearAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.provider_handles.linear.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<crate::adapters::providers::slack::adapter::SlackAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.provider_handles.slack.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<crate::adapters::providers::telegram::adapter::TelegramAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.provider_handles.telegram.clone()
    }
}
