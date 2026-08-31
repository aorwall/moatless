//! Unified application state for API routes.
//!
//! All handler-facing database access goes through DAO trait objects,
//! enabling full unit-test mocking without a real database. They are built by
//! the composition root ([`crate::compose`]) and handed to [`AppState::new`].

use std::sync::Arc;

use crate::adapter_installations::AdapterInstallations;
use moatless_adapter_model::registry::Adapters;
use moatless_adapter_model::dao::AdapterConnectionDao;
use crate::adapters::delivery::AdapterDeliveryStore;
use crate::adapters::self_echo::SelfEchoFilter;
use crate::agent_harness_access::{AgentHarnessAccess, ProviderCodexDeviceLogin};
use crate::app_administration::{AppAdministration, ProviderGitHubAppProver};
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::ProviderTokenAuthDao;
use crate::auth::users::UserDao;
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
use crate::message::send::MessageSendService;
use crate::plugin::dao::PluginDao;
use crate::plugin::sync::PluginSyncService;
use crate::repository::dao::RepositoryDao;
use crate::sandbox::lifecycle::{Lifecycle, NotifyPort};
use crate::sandbox::preflight::{PreflightCredentials, RealGithubValidator, RealPreflight};
use crate::secret::dao::SecretDao;
use crate::secret::service::SecretService;
use crate::skill::fetcher::{GitHubSkillFetcher, SkillFetcher};
use crate::skill::service::SkillService;
use crate::tag::dao::TagDao;
use moatless_backend_core::binding::TaskBindings;
use moatless_adapter_model::binding::BindingProbe;
use moatless_loops::bindings::dao::TaskBindingDao;
use moatless_loops::bindings::ports::BindingDirectory;
use crate::task::closer::TaskCloser;
use crate::task::creation::TaskCreationService;
use moatless_backend_core::sandbox::{PreflightPort, SandboxJoiner};
use crate::task::dao::TaskDao;
use moatless_types::task::CheckoutMode;
use crate::task::presenter::TaskPresenter;
use crate::task::relations::TaskRelationDao;
use moatless_backend_core::sandbox::SandboxLifecycle;
use crate::task_event::dao::TaskEventDao;
use crate::task_turn::dao::TaskTurnDao;
use crate::team::dao::TeamDao;
use crate::workspace::credentials::service::WorkspaceCredentialService;
use crate::workspace::dao::WorkspaceDao;
use crate::workspace::git_sync::GitSyncService;
use moatless_backend_core::sandbox::SandboxError;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};
use moatless_core::config::{Config, S3Config};
use moatless_types::agent::CodexOAuthConfig;
use moatless_types::auth::AuthConfig;
use moatless_types::secret::SecretsConfig;
use moatless_core::db::DbPool;
use moatless_core::events::EventPublisher;
use moatless_identity::git_host_access::{GitHostAccess, ProviderGitHostTokenValidator};
use moatless_backend_core::identity::GitHostTokenIssuer;

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
    /// ([`moatless_backend_core::sandbox::SandboxWaker`]) and implemented in
    /// sandbox-control ([`crate::sandbox::notify::SandboxWakeService`]), so the
    /// message-delivery consumer wakes a sandbox through
    /// [`moatless_backend_core::sandbox::wake_for_message`] without naming the sandbox
    /// layer above it. Built in [`AppState::new`] from the connector-backed notify
    /// port, which `AppState` takes as a constructor parameter but does not hold as
    /// a field — nothing reads the raw notifier off state.
    pub sandbox_waker: Arc<dyn moatless_backend_core::sandbox::SandboxWaker>,
    /// Optional event publisher for real-time event streaming (trait object).
    pub events: Option<Arc<dyn EventPublisher>>,
    /// Optional S3 handle for file uploads/downloads.
    pub s3: Option<crate::file_storage::s3::S3Storage>,
    /// Optional capability registry derived from [`Self::adapter_installations`].
    pub adapters: Option<Arc<Adapters>>,
    /// The process's one coherent installation of every external Adapter.
    /// Construction, capability registration, delivery wiring, and concrete
    /// handler access remain joined behind this value.
    pub adapter_installations: Option<Arc<AdapterInstallations>>,
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
    /// GitHub and Gitness provider-token workflows.
    pub git_host_access: Arc<GitHostAccess>,
    /// Codex credential and device-login workflows.
    pub agent_harness_access: Arc<AgentHarnessAccess>,
    /// Installation-wide Adapter and GitHub App workflows.
    pub app_administration: Arc<AppAdministration>,
    /// Short-lived in-memory cache of `(task_id, external_event_id)` we
    /// just sent outbound — closes the race where the inbound webhook lands
    /// before `set_external_event_id` commits. See [`RecentlySent`].
    pub recently_sent: Arc<RecentlySent>,
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
    /// The `tasks` table behind the published `TaskReader` seam.
    pub task_reader: Arc<moatless_tasks::ports::TaskProjection>,
    /// Single owner of the create-then-provision sequence (short-id, preflight,
    /// insert, tags, relations, provision, first message, event).
    pub task_creation: Arc<TaskCreationService>,
    /// GitHub-token preflight, assembled once at the composition root. The
    /// task-start handler calls the same port `task_creation` runs rather than
    /// reassembling the credential handles and validator per request.
    pub preflight: Arc<dyn PreflightPort>,
    pub task_relations: Arc<dyn TaskRelationDao>,
    /// The one relation a provider records, over the same store.
    pub relation_recorder: Arc<dyn moatless_backend_core::task::RelationRecorder>,
    /// Task↔external-subject bindings (`task_bindings`) — the return channel a
    /// task replies on. The store itself, for the Adapters that drive the
    /// binding lifecycle (upsert plus reads plus status writes).
    pub task_bindings: Arc<dyn TaskBindingDao>,
    /// The same store behind the two published Binding seams: `BindingProbe`
    /// for an Adapter that only asks whether a subject is bound, and
    /// `TaskBindings` for the Task band below `loops`.
    pub binding_directory: Arc<BindingDirectory>,
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
    /// The `task_messages` table behind the published `TranscriptAppend` seam.
    pub transcript: Arc<moatless_tasks::ports::Transcript>,
    pub feedback: Arc<dyn FeedbackDao>,
    pub repositories: Arc<dyn RepositoryDao>,
    pub repository_reader: Arc<dyn moatless_backend_core::workspace::RepositoryReader>,
    /// The workspace, placement, and plugin stores behind the
    /// `WorkspaceResolver` port.
    pub workspace_resolver: Arc<moatless_workspaces::ports::WorkspaceProjection>,
    /// The one way to obtain a git-host token: a named identity and a
    /// repository, with no scope walk.
    pub git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    pub tags: Arc<dyn TagDao>,
    /// The `tags` table behind the published `TagResolver` seam.
    pub tag_resolver: Arc<moatless_tasks::ports::TagCatalog>,
    pub files: Arc<dyn FileDao>,
    pub users: Arc<dyn UserDao>,
    /// The `users` table behind the three identity ports peer crates read:
    /// `UserDisplay`, `UserResolver`, `CommitIdentity`, `ProviderIdentityLink`.
    pub user_directory: Arc<moatless_identity::ports::UserDirectory>,
    pub provider_tokens: Arc<dyn ProviderTokenDao>,
    /// The `provider_tokens` table behind the `HarnessCredentials` port.
    pub harness_credentials: Arc<moatless_identity::ports::HarnessTokens>,
    /// The `adapter_apps` tables behind the `AdapterAppReader` port an Adapter
    /// resolves its provider credentials through.
    pub adapter_apps: Arc<dyn moatless_adapter_model::apps::AdapterAppReader>,
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
    /// The `task_turns` table behind the published `TurnCounter` seam.
    pub turn_counter: Arc<moatless_tasks::ports::TurnLedger>,
    pub plugins: Arc<dyn PluginDao>,
    /// The one path from a plugin's git content to its stored skills. Every
    /// reader of plugin skills goes through this.
    pub plugin_sync: Arc<PluginSyncService>,
    /// Workspace composition + run-config (`workspaces` / `workspace_repos`).
    /// Provisioning resolves a task's repository composition through this.
    pub workspaces: Arc<dyn WorkspaceDao>,
    /// Provider credentials pinned to a Workspace (`workspace_credentials`) —
    /// the highest-precedence source of a sandbox's provider credentials, and
    /// the port the CLI-only credential routes write through.
    pub workspace_credentials: Arc<WorkspaceCredentialService>,
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
        reason = "eight independent infrastructure handles — ports, config, \
                  lifecycle, connector, sandbox_notify, events, s3, and \
                  adapter_installations — each wired in from the composition \
                  root; bundling them behind a one-use struct only shuffles the \
                  same fields."
    )]
    pub fn new(
        ports: &Ports,
        config: Arc<Config>,
        lifecycle: Arc<Lifecycle>,
        connector: Arc<dyn SandboxConnector>,
        sandbox_notify: Arc<dyn NotifyPort>,
        events: Option<Arc<dyn EventPublisher>>,
        s3: Option<crate::file_storage::s3::S3Storage>,
        adapter_installations: Option<Arc<AdapterInstallations>>,
    ) -> Self {
        let Ports {
            pool,
            tasks,
            task_reader,
            task_relations,
            task_bindings,
            binding_directory,
            task_events,
            task_turns,
            turn_counter,
            messages,
            transcript,
            feedback,
            repositories,
            repository_reader,
            workspaces,
            workspace_resolver,
            workspace_credentials,
            tags,
            tag_resolver,
            files,
            teams,
            plugins,
            loops,
            adapter_connections,
            event_receipts,
            ingress_outbox,
            adapter_delivery,
            users,
            user_directory,
            provider_tokens,
            harness_credentials,
            adapter_apps,
            provider_token_auth,
            secret_records,
            secrets,
            github_apps,
            github_app_service,
            git_host_tokens,
            token_verifier,
            ..
        } = ports.clone();

        // The concrete issuer is built once in `compose`; every consumer below
        // takes the port.
        let git_host_tokens: Arc<dyn GitHostTokenIssuer> = git_host_tokens;

        let adapters = adapter_installations
            .as_ref()
            .map(|installations| installations.adapters());

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
                .with_codex_refresh(codex_http)
                .with_secrets(secrets.clone()),
        );
        // Tasks-band orchestrator (9c): the in-flight guard + sandbox fan-out
        // that reach up out of the workspaces-band credential service.
        let credential_sync = Arc::new(CredentialSync::new(
            credentials_svc.clone(),
            tasks.clone(),
            Arc::new(crate::sandbox::notify::CredentialBroadcastService::new(
                connector.clone(),
            )),
        ));

        let git_sync = Arc::new(GitSyncService::new(
            pool.clone(),
            workspaces.clone(),
            repositories.clone(),
            git_host_tokens.clone(),
        ));

        let loop_sync = Arc::new(LoopSyncService::new(
            adapter_connections.clone(),
            loops.clone(),
            tag_resolver.clone(),
            git_host_tokens.clone(),
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
            repository_reader.clone(),
            workspace_resolver.clone(),
            user_directory.clone(),
            binding_directory.clone(),
            task_relations.clone(),
            tasks.clone(),
        ));

        let task_closer = Arc::new(TaskCloser::new(
            tasks.clone(),
            task_relations.clone(),
            config.clone(),
            lifecycle.clone(),
            binding_directory.clone(),
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
            git_host_tokens.clone(),
        ));
        let skills = Arc::new(SkillService::new(
            repositories.clone(),
            workspaces.clone(),
            plugins.clone(),
            git_host_tokens.clone(),
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
                git_host_tokens: git_host_tokens.clone(),
                users: user_directory.clone(),
                workspaces: workspace_resolver.clone(),
                credential_sync: credential_sync.clone(),
                workspace_credentials: workspace_credentials.clone(),
            },
            Arc::new(RealGithubValidator),
        ));

        let sandbox_joiner: Arc<dyn SandboxJoiner> = Arc::new(ConnectorSandboxJoiner {
            connector: connector.clone(),
        });
        let task_creation = Arc::new(TaskCreationService::new(
            tasks.clone(),
            tags.clone(),
            task_relations.clone(),
            repository_reader.clone(),
            events.clone(),
            lifecycle.clone(),
            sandbox_joiner,
            preflight.clone(),
        ));

        // Config-section ports, built once so a secret-bearing section is never
        // re-cloned per request. `config` still holds the whole tree for
        // unmigrated call sites.
        let auth_config = Arc::new(config.auth.clone());
        let codex_oauth_config = Arc::new(config.codex_oauth.clone());
        let secrets_config = Arc::new(config.secrets.clone());
        let s3_config = Arc::new(config.s3.clone());

        let git_host_access = Arc::new(GitHostAccess::new(
            provider_tokens.clone(),
            secrets.clone(),
            Arc::new(ProviderGitHostTokenValidator::default()),
        ));
        let agent_harness_access = Arc::new(AgentHarnessAccess::new(
            provider_tokens.clone(),
            credential_sync.clone(),
            codex_oauth_config.clone(),
            Arc::new(ProviderCodexDeviceLogin::default()),
            users.clone(),
        ));
        let app_administration = Arc::new(AppAdministration::new(
            github_apps.clone(),
            users.clone(),
            Arc::new(ProviderGitHubAppProver),
        ));

        // The wake port, implemented in sandbox-control. Built here because it
        // needs the lifecycle handle and the notify port — both of which the
        // composition root already assembled.
        let sandbox_waker: Arc<dyn moatless_backend_core::sandbox::SandboxWaker> =
            Arc::new(crate::sandbox::notify::SandboxWakeService::new(
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
            transcript.clone(),
            task_reader.clone(),
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
            task_reader.clone(),
            recently_sent.clone(),
            transcript.clone(),
            turn_counter.clone(),
            s3.clone(),
            auth_config.clone(),
            s3_config.clone(),
            files.clone(),
            workspace_resolver.clone(),
            user_directory.clone(),
        ));
        let external_reply = Arc::new(crate::bindings::reply::ExternalReplyService::new(
            adapters.clone(),
            task_bindings.clone(),
            adapter_connections.clone(),
            transcript.clone(),
            task_reader.clone(),
            loops.clone(),
            recently_sent.clone(),
            turn_counter.clone(),
            outbound_sender,
        ));

        Self {
            tasks,
            task_reader,
            task_creation,
            preflight,
            relation_recorder: Arc::new(moatless_tasks::ports::TaskRelations::new(
                task_relations.clone(),
            )),
            task_relations,
            task_bindings,
            binding_directory,
            adapter_connections,
            routing,
            external_reply,
            loops,
            messages,
            transcript,
            feedback,
            repositories,
            repository_reader,
            git_host_tokens,
            tags,
            tag_resolver,
            files,
            users,
            user_directory,
            provider_tokens,
            harness_credentials,
            adapter_apps,
            provider_token_auth,
            secret_records,
            secrets,
            teams,
            task_events,
            ingress_outbox,
            task_turns,
            turn_counter,
            plugins,
            plugin_sync,
            workspaces,
            workspace_resolver,
            workspace_credentials,
            git_sync,
            loop_sync,
            message_send,
            task_presenter,
            task_closer,
            skills,
            adapter_delivery,
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
            adapter_installations,
            github_app_service,
            github_apps,
            credential_sync,
            git_host_access,
            agent_harness_access,
            app_administration,
            recently_sent,
            tool_call_names: Arc::new(
                moka::future::Cache::builder()
                    .max_capacity(50_000)
                    .time_to_live(std::time::Duration::from_mins(30))
                    .build(),
            ),
        }
    }
}

struct ConnectorSandboxJoiner {
    connector: Arc<dyn SandboxConnector>,
}

#[async_trait::async_trait]
impl SandboxJoiner for ConnectorSandboxJoiner {
    async fn join_task(
        &self,
        task_id: &str,
        checkout_mode: CheckoutMode,
    ) -> Result<(), SandboxError> {
        let client = self.connector.sandbox_client(task_id).await?;
        client
            .join_task(task_id, Some(checkout_mode.as_str()))
            .await
            .map_err(|error| classify_grpc_error(error, "JoinTask"))
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

impl axum::extract::FromRef<AppState> for Arc<WorkspaceCredentialService> {
    fn from_ref(state: &AppState) -> Self {
        state.workspace_credentials.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn RepositoryDao> {
    fn from_ref(state: &AppState) -> Self {
        state.repositories.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Arc<dyn moatless_backend_core::workspace::RepositoryReader>
{
    fn from_ref(state: &AppState) -> Self {
        state.repository_reader.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Arc<dyn moatless_backend_core::workspace::WorkspaceResolver>
{
    fn from_ref(state: &AppState) -> Self {
        state.workspace_resolver.clone()
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

impl axum::extract::FromRef<AppState> for Arc<dyn GitHostTokenIssuer> {
    fn from_ref(state: &AppState) -> Self {
        state.git_host_tokens.clone()
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

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_backend_core::identity::SecretCatalog> {
    fn from_ref(state: &AppState) -> Self {
        state.secrets.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Arc<dyn moatless_backend_core::identity::SecretResolver>
{
    fn from_ref(state: &AppState) -> Self {
        state.secrets.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Arc<dyn moatless_backend_core::identity::HarnessCredentials>
{
    fn from_ref(state: &AppState) -> Self {
        state.harness_credentials.clone()
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

impl axum::extract::FromRef<AppState> for Arc<GitHostAccess> {
    fn from_ref(state: &AppState) -> Self {
        state.git_host_access.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<AgentHarnessAccess> {
    fn from_ref(state: &AppState) -> Self {
        state.agent_harness_access.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<AppAdministration> {
    fn from_ref(state: &AppState) -> Self {
        state.app_administration.clone()
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

impl axum::extract::FromRef<AppState> for Arc<dyn BindingProbe> {
    fn from_ref(state: &AppState) -> Self {
        state.binding_directory.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn TaskBindings> {
    fn from_ref(state: &AppState) -> Self {
        state.binding_directory.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<LoopDao> {
    fn from_ref(state: &AppState) -> Self {
        state.loops.clone()
    }
}

/// The Subscription reads an Adapter's webhook route makes. Coerced from the
/// one `LoopDao` rather than held as a second field.
impl axum::extract::FromRef<AppState> for Arc<dyn moatless_adapter_model::loops::LoopReader> {
    fn from_ref(state: &AppState) -> Self {
        state.loops.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_adapter_model::delivery::DeliveryAdmission> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_delivery.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<RecentlySent> {
    fn from_ref(state: &AppState) -> Self {
        state.recently_sent.clone()
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

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_backend_core::task::TaskReader> {
    fn from_ref(state: &AppState) -> Self {
        state.task_reader.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_backend_core::task::TranscriptAppend> {
    fn from_ref(state: &AppState) -> Self {
        state.transcript.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_backend_core::identity::UserDisplay> {
    fn from_ref(state: &AppState) -> Self {
        state.user_directory.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_backend_core::identity::UserResolver> {
    fn from_ref(state: &AppState) -> Self {
        state.user_directory.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Arc<dyn moatless_backend_core::identity::ProviderIdentityLink>
{
    fn from_ref(state: &AppState) -> Self {
        state.user_directory.clone()
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

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_backend_core::sandbox::SandboxWaker> {
    fn from_ref(state: &AppState) -> Self {
        state.sandbox_waker.clone()
    }
}

/// Project the ports an errored run is closed over. The event consumer and the
/// task reaper both close a turn through the same handler; the reaper holds the
/// projection so it never names `AppState`.
impl From<&AppState> for crate::event_handler::agent_handlers::AgentErrorPorts {
    fn from(state: &AppState) -> Self {
        Self {
            tasks: state.tasks.clone(),
            task_relations: state.task_relations.clone(),
            message_send: state.message_send.clone(),
            sandbox_waker: state.sandbox_waker.clone(),
            events: state.events.clone(),
        }
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
