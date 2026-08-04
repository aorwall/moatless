//! Unified application state for API routes.
//!
//! All handler-facing database access goes through DAO trait objects,
//! enabling full unit-test mocking without a real database. They are built by
//! the composition root ([`crate::compose`]) and handed to [`AppState::new`].

use std::sync::Arc;

use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::delivery::AdapterDeliveryStore;
use crate::adapters::linear::active_turns::ActiveLinearTurns;
use crate::adapters::Adapters;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::ProviderTokenAuthDao;
use crate::auth::users::UserDao;
use crate::bindings::dao::TaskBindingDao;
use crate::bindings::recently_sent::RecentlySent;
use crate::compose::Ports;
use crate::config::{AuthConfig, CodexOAuthConfig, Config, S3Config, SecretsConfig};
use crate::credentials::{BroadcastContext, CredentialService};
use crate::db::DbPool;
use crate::event_ingress::outbox::IngressOutbox;
use crate::events::EventPublisher;
use crate::feedback::dao::FeedbackDao;
use crate::file_storage::dao::FileDao;
use crate::loops::dao::LoopDao;
use crate::loops::firing::receipts::EventReceiptDao;
use crate::loops::sync::LoopSyncService;
use crate::message::dao::MessageDao;
use crate::plugin::dao::PluginDao;
use crate::plugin::sync::PluginSyncService;
use crate::repository::access::RepositoryTokenResolver;
use crate::repository::dao::RepositoryDao;
use crate::sandbox::lifecycle::{Lifecycle, NotifyPort};
use crate::sandbox::preflight::{PreflightCredentials, RealGithubValidator};
use crate::secret::audit::SecretAuditDao;
use crate::secret::dao::SecretDao;
use crate::secret::service::SecretService;
use crate::skill::fetcher::{GitHubSkillFetcher, SkillFetcher};
use crate::skill::service::SkillService;
use crate::tag::dao::TagDao;
use crate::task::creation::{RealPreflight, TaskCreationService};
use crate::task::dao::TaskDao;
use crate::task::relations::TaskRelationDao;
use crate::task_event::dao::TaskEventDao;
use crate::task_turn::dao::TaskTurnDao;
use crate::team::dao::TeamDao;
use crate::workspace::dao::WorkspaceDao;
use crate::workspace::git_sync::GitSyncService;
use moatless_backend_core::sandbox::SandboxConnector;

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
    /// Message notification port. Address construction stays inside the
    /// connector-backed adapter rather than on `AppState`.
    ///
    /// Still a field, unlike `agent_status`: its only reader
    /// (`sandbox::notify::notify_sandbox_for_task`) is reached through
    /// `ensure_sandbox_and_notify(&AppState)`, whose seven handler/adapter/loop
    /// callers would each have to take the port to remove it — that is the
    /// per-handler sweep. It leaves `AppState` in the slice that migrates those
    /// callers. See `.plans/backend-ports-and-crates.md` Step 5.
    pub sandbox_notify: Arc<dyn NotifyPort>,
    /// Optional event publisher for real-time event streaming (trait object).
    pub events: Option<Arc<dyn EventPublisher>>,
    /// Optional S3 handle for file uploads/downloads.
    pub s3: Option<crate::file_storage::s3::S3Storage>,
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
    /// any admin-registered bot Apps, read from storage). Bot users' provider
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
    // DAO trait objects
    // =========================================================================
    pub tasks: Arc<dyn TaskDao>,
    /// Single owner of the create-then-provision sequence (short-id, preflight,
    /// insert, tags, relations, provision, first message, event).
    pub task_creation: Arc<TaskCreationService>,
    pub task_relations: Arc<dyn TaskRelationDao>,
    /// Task↔external-subject bindings (`task_bindings`) — the return channel a
    /// task replies on. Handlers read and write bindings through this port.
    pub task_bindings: Arc<dyn TaskBindingDao>,
    /// Adapter connections (`adapter_connections`) — the per-provider
    /// credential/webhook-secret rows inbound and outbound calls resolve
    /// against. Handlers read and write connections through this port.
    pub adapter_connections: Arc<dyn AdapterConnectionDao>,
    /// Durable adapter-event deduplication (`adapter_event_receipts`). The
    /// binding resolver records a delivery here so a redelivered webhook does
    /// not double-post to a task.
    pub event_receipts: Arc<dyn EventReceiptDao>,
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
    pub secret_audits: Arc<dyn SecretAuditDao>,
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
    // reason: independent infrastructure handles (ports, config, lifecycle,
    // connector, events, s3, adapters); bundling them behind a one-use struct
    // only shuffles the same fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ports: &Ports,
        config: Arc<Config>,
        lifecycle: Arc<Lifecycle>,
        connector: Arc<dyn SandboxConnector>,
        sandbox_notify: Arc<dyn NotifyPort>,
        events: Option<Arc<dyn EventPublisher>>,
        s3: Option<crate::file_storage::s3::S3Storage>,
        adapters: Option<Arc<Adapters>>,
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
            secret_audits,
            secrets,
            github_apps,
            github_app_service,
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
            CredentialService::new(
                provider_tokens.clone(),
                Some(provider_token_auth.clone()),
                github_app_service.clone(),
                Some(BroadcastContext {
                    connector,
                    tasks: tasks.clone(),
                }),
            )
            .with_codex_refresh(pool.clone(), codex_http)
            .with_secrets(secrets.clone()),
        );

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
        // (HTTP, loops, adapters) gets the same check for free.
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

        let token_verifier: Arc<dyn crate::extractors::UserTokenVerifier> =
            Arc::new(crate::sandbox::jwt::JwtUserTokenVerifier::new(
                config.sandbox.task_token_secret.clone(),
            ));

        // Config-section ports, built once so a secret-bearing section is never
        // re-cloned per request. `config` still holds the whole tree for
        // unmigrated call sites.
        let auth_config = Arc::new(config.auth.clone());
        let codex_oauth_config = Arc::new(config.codex_oauth.clone());
        let secrets_config = Arc::new(config.secrets.clone());
        let s3_config = Arc::new(config.s3.clone());

        Self {
            tasks,
            task_creation,
            task_relations,
            task_bindings,
            adapter_connections,
            event_receipts,
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
            secret_audits,
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

impl axum::extract::FromRef<AppState> for Arc<CredentialService> {
    fn from_ref(state: &AppState) -> Self {
        state.credentials_svc.clone()
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
