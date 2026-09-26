//! Unified application state for API routes.
//!
//! All handler-facing database access goes through DAO trait objects, enabling
//! full unit-test mocking without a real database. They are built by the
//! composition root ([`crate::compose`]) and handed to [`AppState::new`].

use crate::access_settings::provider_apps::ProviderApps;
use crate::adapter_installations::AdapterInstallations;
use crate::agent_harness_access::AgentHarnessAccess;
use crate::git_proxy::GitProxy;
use crate::sandbox::lifecycle::Lifecycle;
use moatless_adapter_model::binding::BindingProbe;
use moatless_adapter_model::connections::{
    AdapterConnections, ConnectionLookup, ConnectionRouting,
};
use moatless_adapter_model::registry::Adapters;
use moatless_core::config::{Config, S3Config};
use moatless_core::db::DbPool;
use moatless_core::events::EventPublisher;
use moatless_core::file_storage::Files;
use moatless_identity::adapter_apps::administration::AppAdministration;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_identity::forgejo::{ForgejoAccess, ForgejoInstances};
use moatless_identity::git_host_access::GitHostAccess;
use moatless_identity::secret::service::SecretService;
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::ports::BindingDirectory;
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_loops::loops::operations::Loops;
use moatless_loops::loops::sync::LoopSyncService;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_ports::sandbox::PreflightPort;
use moatless_ports::sandbox::SandboxLifecycle;
use moatless_tasks::credential_sync::CredentialSync;
use moatless_tasks::event_ingress::EventAdmission;
use moatless_tasks::feedback::operations::FeedbackNotes;
use moatless_tasks::message::operations::Messages;
use moatless_tasks::message::send::MessageSendService;
use moatless_tasks::tag::operations::Tags;
use moatless_tasks::task::bindings::TaskBindings;
use moatless_tasks::task::closer::TaskCloser;
use moatless_tasks::task::creation::TaskCreationService;
use moatless_tasks::task::fork::TaskForkService;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::presenter::TaskPresenter;
use moatless_tasks::task::relations::TaskRelations;
use moatless_tasks::task::rewind::TaskRewindService;
use moatless_tasks::task::switch::TaskSwitchService;
use moatless_tasks::task_event::operations::TaskEvents;
use moatless_tasks::task_turn::operations::TaskTurns;
use moatless_types::agent::CodexOAuthConfig;
use moatless_types::auth::AuthConfig;
use moatless_types::secret::SecretsConfig;
use moatless_workspaces::plugin::Plugins;
use moatless_workspaces::plugin::sync::PluginSyncService;
use moatless_workspaces::repository::Repositories;
use moatless_workspaces::skill::service::SkillService;
use moatless_workspaces::workspace::Workspaces;
use moatless_workspaces::workspace::credentials::service::WorkspaceCredentialService;
use moatless_workspaces::workspace::git_sync::GitSyncService;
use std::sync::Arc;

/// Shared application state for task/message/agent/file routes.
///
/// All fields are cheap to clone (Arc-wrapped or internally Arc'd); DAO trait
/// objects enable handler unit tests without Postgres.
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool — kept for auth routes and infrastructure
    /// code that constructs its own state (gRPC server, idle reaper, etc.).
    pub db: DbPool,
    /// Immutable configuration shared across handlers.
    pub config: Arc<Config>,
    /// Config sections handed to migrated handlers as `Arc<Section>` ports,
    /// each built once here.
    ///
    /// `Arc<Section>`, not the section by value, keeps a secret-bearing section
    /// (`AuthConfig`, `SecretsConfig`) from being re-cloned on every request.
    pub auth_config: Arc<AuthConfig>,
    pub codex_oauth_config: Arc<CodexOAuthConfig>,
    pub secrets_config: Arc<SecretsConfig>,
    pub s3_config: Arc<S3Config>,
    /// Verifies sandbox-issued `X-User-Token` JWTs. The auth extractors reach
    /// this through [`moatless_core::http_auth::UserTokenVerifier`] rather than
    /// reading the token secret off `config` directly.
    pub token_verifier: Arc<dyn moatless_core::http_auth::UserTokenVerifier>,
    /// The one Sandbox value on `AppState`: the durable lifecycle handle.
    /// Callers request transitions
    /// (`start`/`stop`/`restart`/`redeploy`/`remove`) and read desired state
    /// through it rather than driving a provider directly.
    pub sandbox: Arc<Lifecycle>,
    /// Consumer-owned [`moatless_ports::sandbox::SandboxWaker`].
    ///
    /// Lets the message-delivery consumer wake a sandbox without naming the
    /// sandbox layer above it; built in [`AppState::new`] from a
    /// connector-backed notify port that isn't itself held as a field.
    pub sandbox_waker: Arc<dyn moatless_ports::sandbox::SandboxWaker>,
    /// Optional event publisher for real-time event streaming (trait object).
    pub events: Option<Arc<dyn EventPublisher>>,
    /// Optional S3 handle for file uploads/downloads.
    pub s3: Option<moatless_s3::S3Storage>,
    /// Optional capability registry derived from
    /// [`Self::adapter_installations`].
    pub adapters: Option<Arc<Adapters>>,
    /// The process's one coherent installation of every external Adapter.
    /// Construction, capability registration, delivery wiring, and concrete
    /// handler access remain joined behind this value.
    pub adapter_installations: Option<Arc<AdapterInstallations>>,
    /// Optional GitHub App service for installation tokens and user token
    /// refresh.
    ///
    /// Convenience handle to the default App, equivalent to
    /// `github_apps.default_service()`; kept for back-compat, new code should
    /// use `github_apps` to pick an App by key.
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
    /// Forgejo provider-token workflows, addressed per instance.
    pub forgejo_access: Arc<ForgejoAccess>,
    /// The OAuth2 application registered for each Forgejo instance.
    pub forgejo_instances: Arc<ForgejoInstances>,
    /// Resolves a proxied git request to an upstream and a credential.
    pub git_proxy: Arc<GitProxy>,
    /// Codex credential and device-login workflows.
    pub agent_harness_access: Arc<AgentHarnessAccess>,
    /// Installation-wide Adapter and GitHub App workflows.
    pub app_administration: Arc<AppAdministration>,
    /// Short-lived in-memory cache of `(task_id, external_event_id)` we just
    /// sent outbound — closes the race where the inbound webhook lands before
    /// `set_external_event_id` commits. See [`RecentlySent`].
    pub recently_sent: Arc<RecentlySent>,
    /// Maps `tool_use_id` → bucketed tool-type label, populated at the
    /// assistant `tool_use` block so the later `tool_result` event can be
    /// labelled without a DB round-trip.
    ///
    /// Bounded LRU; a `messages.find_tool_call` lookup covers
    /// cross-process/post-restart misses, falling back to `unknown`.
    pub tool_call_names: Arc<moka::future::Cache<String, &'static str>>,

    // =========================================================================
    // DAO trait objects
    // =========================================================================
    pub tasks: Arc<TaskCommands>,
    /// The `tasks` table behind the published `TaskReader` seam.
    pub task_reader: Arc<moatless_tasks::ports::TaskProjection>,
    /// Single owner of the create-then-provision sequence (short-id, preflight,
    /// insert, tags, relations, provision, first message, event).
    pub task_creation: Arc<TaskCreationService>,
    /// Resolves where a fork cuts and copies the parent's conversation onto it.
    /// The create itself still goes through `task_creation`.
    pub task_fork: Arc<TaskForkService>,
    /// Cuts a Task's own conversation back to a completed turn, in place. The
    /// same cut `task_fork` resolves, applied to the Task instead of copied off
    /// it.
    pub task_rewind: Arc<TaskRewindService>,
    /// Sets the Agent profile a Task's next turn runs.
    pub task_switch: Arc<TaskSwitchService>,
    /// GitHub-token preflight, assembled once at the composition root. The
    /// task-start handler calls the same port `task_creation` runs rather than
    /// reassembling the credential handles and validator per request.
    pub preflight: Arc<dyn PreflightPort>,
    pub task_relations: Arc<TaskRelations>,
    /// The one relation a provider records, over the same store.
    pub relation_recorder: Arc<dyn moatless_ports::task::RelationRecorder>,
    /// Task↔external-subject bindings (`task_bindings`) — the return channel a
    /// task replies on. The store itself, for the Adapters that drive the
    /// binding lifecycle (upsert plus reads plus status writes).
    pub task_bindings: Arc<Bindings>,
    /// The same store behind the two published Binding seams: `BindingProbe`
    /// for an Adapter that only asks whether a subject is bound, and
    /// `TaskBindings` for the Task band below `loops`.
    pub binding_directory: Arc<BindingDirectory>,
    /// Adapter connections (`adapter_connections`) — the per-provider
    /// credential/webhook-secret rows inbound and outbound calls resolve
    /// against. Handlers read and write connections through this port.
    pub adapter_connections: Arc<AdapterConnections>,
    /// The same rows as an Adapter routes against them: no viewer, because an
    /// inbound event and not a session says which tenant is meant.
    pub connection_routing: Arc<ConnectionRouting>,
    /// The inbound-routing subgraph and the ten ports it reads, including the
    /// durable adapter-event dedup, which isn't its own field since nothing
    /// outside routing reads it.
    ///
    /// The inbound-delivery handler and GitHub PR subject router route through
    /// this instead of naming each port.
    pub routing: Arc<moatless_loops::bindings::runtime::RoutingService>,
    pub external_reply: Arc<moatless_loops::bindings::reply::ExternalReplyService>,
    pub adapter_commands: Arc<crate::adapters::operations::AdapterCommands>,
    /// The four Loop tables (`loops`, `loop_configs`, `subscriptions`,
    /// `schedules`), as a host reads and writes them.
    pub loops: Arc<Loops>,
    /// The Subscription reads an Adapter's webhook route makes, over the same
    /// tables.
    pub loop_reader: Arc<dyn moatless_adapter_model::loops::LoopReader>,
    pub messages: Arc<Messages>,
    /// The `task_messages` table behind the published `TranscriptAppend` seam.
    pub transcript: Arc<moatless_tasks::ports::Transcript>,
    pub feedback: Arc<FeedbackNotes>,
    pub repositories: Arc<Repositories>,
    pub repository_branches:
        Arc<moatless_workspaces::repository::branches::RepositoryBranchService>,
    pub repository_reader: Arc<dyn moatless_workspace_ports::RepositoryReader>,
    /// The workspace, placement, and plugin stores behind the
    /// `WorkspaceResolver` port.
    pub workspace_resolver: Arc<moatless_workspaces::ports::WorkspaceProjection>,
    /// The one way to obtain a git-host token: a named identity and a
    /// repository, with no scope walk.
    pub git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    pub tags: Arc<Tags>,
    /// The `tags` table behind the published `TagResolver` seam.
    pub tag_resolver: Arc<moatless_tasks::ports::TagCatalog>,
    /// Uploads: the object, the metadata row, and who may read one back.
    pub files: Arc<Files>,
    /// The `users` table behind the three identity ports peer crates read:
    /// `UserDisplay`, `UserResolver`, `CommitIdentity`, `ProviderIdentityLink`.
    pub user_directory: Arc<moatless_identity::ports::UserDirectory>,
    /// The one way to read, persist and refresh a provider-token credential.
    pub credentials: Arc<moatless_identity::credentials::CredentialService>,
    /// The `provider_tokens` table behind the `HarnessCredentials` port.
    pub harness_credentials: Arc<moatless_identity::ports::HarnessTokens>,
    /// The `adapter_apps` tables behind the `AdapterAppReader` port an Adapter
    /// resolves its provider credentials through.
    pub adapter_apps: Arc<dyn moatless_adapter_model::apps::AdapterAppReader>,
    pub secrets: Arc<SecretService>,
    pub task_events: Arc<TaskEvents>,
    /// Durable admission for sandbox-published events.
    ///
    /// The event-handler consumer reports delivery here once it commits the
    /// message an admitted record carries — the only delivery confirmation in
    /// the system — and the relay is spawned from it.
    pub event_ingress: Arc<EventAdmission>,
    /// Authoritative per-turn lifecycle state (`task_turns`). Written
    /// transactionally at the gRPC ingress; the reaper reads it as the
    /// backstop.
    pub task_turns: Arc<TaskTurns>,
    /// The `task_turns` table behind the published `TurnCounter` seam.
    pub turn_counter: Arc<moatless_tasks::ports::TurnLedger>,
    pub plugins: Arc<Plugins>,
    /// The one path from a plugin's git content to its stored skills. Every
    /// reader of plugin skills goes through this.
    pub plugin_sync: Arc<PluginSyncService>,
    /// Workspace composition + run-config (`workspaces` / `workspace_repos`).
    /// Provisioning resolves a task's repository composition through this.
    pub workspaces: Arc<Workspaces>,
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
    /// dedup filter, so the message domain does not name the adapter registry.
    pub message_send: Arc<MessageSendService>,
    /// Presents a task row as a `TaskResponse`; holds the eight ports the
    /// response is assembled from so presenting handlers name one service
    /// instead of eight DAOs.
    pub task_presenter: Arc<TaskPresenter>,
    pub task_closer: Arc<TaskCloser>,
    /// Skill discovery service — reads `.claude/skills/` from workspace and
    /// plugins, parses frontmatter, caches results.
    pub skills: Arc<SkillService>,
    /// Durable admission for every inbound Adapter delivery, whose work can't
    /// finish on the HTTP response path.
    ///
    /// A webhook handler admits through this before it acknowledges its sender;
    /// the supervised `adapter-delivery-worker` owns what was admitted.
    pub adapter_delivery: Arc<dyn moatless_adapter_model::delivery::DeliveryAdmission>,
    /// The worker that owns what admission wrote. The host starts it last, once
    /// every delivery handler this build knows is registered.
    pub adapter_delivery_worker: moatless_loops::adapters::delivery::AdapterDeliveryWorker,
    /// The flusher that turns each Binding's closed buffer window into one
    /// combined Task message. Started beside the delivery worker.
    pub message_buffer_worker: moatless_loops::bindings::buffer::MessageBufferWorker,
}

// `FromRef` lets a handler extract exactly the port it names —
// `State(port): State<Arc<dyn Port>>` — from the one router-held `AppState`,
// so handlers stop naming `AppState`. `State<AppState>` still resolves through
// axum's blanket `FromRef<S> for S`, so migrated and unmigrated handlers share
// one router. One impl is added here per port as its module migrates.

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_core::http_auth::UserTokenVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.token_verifier.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<SecretService> {
    fn from_ref(state: &AppState) -> Self {
        state.secrets.clone()
    }
}

impl axum::extract::FromRef<AppState> for Option<moatless_s3::S3Storage> {
    fn from_ref(state: &AppState) -> Self {
        state.s3.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<Files> {
    fn from_ref(state: &AppState) -> Self {
        state.files.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<FeedbackNotes> {
    fn from_ref(state: &AppState) -> Self {
        state.feedback.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskCommands> {
    fn from_ref(state: &AppState) -> Self {
        state.tasks.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskEvents> {
    fn from_ref(state: &AppState) -> Self {
        state.task_events.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<Tags> {
    fn from_ref(state: &AppState) -> Self {
        state.tags.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<Workspaces> {
    fn from_ref(state: &AppState) -> Self {
        state.workspaces.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<WorkspaceCredentialService> {
    fn from_ref(state: &AppState) -> Self {
        state.workspace_credentials.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<Repositories> {
    fn from_ref(state: &AppState) -> Self {
        state.repositories.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_workspace_ports::RepositoryReader> {
    fn from_ref(state: &AppState) -> Self {
        state.repository_reader.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_workspace_ports::WorkspaceResolver> {
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

impl axum::extract::FromRef<AppState> for Arc<Plugins> {
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

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::identity::SecretCatalog> {
    fn from_ref(state: &AppState) -> Self {
        state.secrets.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::identity::SecretResolver> {
    fn from_ref(state: &AppState) -> Self {
        state.secrets.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::identity::HarnessCredentials> {
    fn from_ref(state: &AppState) -> Self {
        state.harness_credentials.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<AdapterConnections> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_connections.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<ConnectionRouting> {
    fn from_ref(state: &AppState) -> Self {
        state.connection_routing.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn ConnectionLookup> {
    fn from_ref(state: &AppState) -> Self {
        state.connection_routing.clone()
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

impl axum::extract::FromRef<AppState> for Arc<GitProxy> {
    fn from_ref(state: &AppState) -> Self {
        state.git_proxy.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<ForgejoAccess> {
    fn from_ref(state: &AppState) -> Self {
        state.forgejo_access.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<ForgejoInstances> {
    fn from_ref(state: &AppState) -> Self {
        state.forgejo_instances.clone()
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

impl axum::extract::FromRef<AppState> for ProviderApps {
    fn from_ref(state: &AppState) -> Self {
        ProviderApps {
            forgejo_instances: state.forgejo_instances.clone(),
            app_administration: state.app_administration.clone(),
            frontend_url: state.config.auth.frontend_url.clone(),
        }
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

impl axum::extract::FromRef<AppState> for Arc<TaskRelations> {
    fn from_ref(state: &AppState) -> Self {
        state.task_relations.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<Bindings> {
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

impl axum::extract::FromRef<AppState> for Arc<Loops> {
    fn from_ref(state: &AppState) -> Self {
        state.loops.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_adapter_model::loops::LoopReader> {
    fn from_ref(state: &AppState) -> Self {
        state.loop_reader.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Arc<dyn moatless_adapter_model::delivery::DeliveryAdmission>
{
    fn from_ref(state: &AppState) -> Self {
        state.adapter_delivery.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<RecentlySent> {
    fn from_ref(state: &AppState) -> Self {
        state.recently_sent.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<Messages> {
    fn from_ref(state: &AppState) -> Self {
        state.messages.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::task::TaskReader> {
    fn from_ref(state: &AppState) -> Self {
        state.task_reader.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::task::TranscriptAppend> {
    fn from_ref(state: &AppState) -> Self {
        state.transcript.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::identity::UserDisplay> {
    fn from_ref(state: &AppState) -> Self {
        state.user_directory.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::identity::UserResolver> {
    fn from_ref(state: &AppState) -> Self {
        state.user_directory.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::identity::ProviderIdentityLink> {
    fn from_ref(state: &AppState) -> Self {
        state.user_directory.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskTurns> {
    fn from_ref(state: &AppState) -> Self {
        state.task_turns.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Arc<moatless_loops::bindings::reply::ExternalReplyService>
{
    fn from_ref(state: &AppState) -> Self {
        state.external_reply.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<MessageSendService> {
    fn from_ref(state: &AppState) -> Self {
        state.message_send.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<dyn moatless_ports::sandbox::SandboxWaker> {
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
            task_turns: state.task_turns.clone(),
        }
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskPresenter> {
    fn from_ref(state: &AppState) -> Self {
        state.task_presenter.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskForkService> {
    fn from_ref(state: &AppState) -> Self {
        state.task_fork.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskRewindService> {
    fn from_ref(state: &AppState) -> Self {
        state.task_rewind.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<TaskSwitchService> {
    fn from_ref(state: &AppState) -> Self {
        state.task_switch.clone()
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

impl axum::extract::FromRef<AppState>
    for Arc<moatless_workspaces::repository::branches::RepositoryBranchService>
{
    fn from_ref(state: &AppState) -> Self {
        state.repository_branches.clone()
    }
}

impl axum::extract::FromRef<AppState> for Arc<crate::adapters::operations::AdapterCommands> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_commands.clone()
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<moatless_adapter_slack::adapter::SlackAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.slack())
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<moatless_adapter_telegram::adapter::TelegramAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.telegram())
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<moatless_adapter_github_pr::adapter::GithubPrAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.github_pr())
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<moatless_loops::loops::firing::listen::Connector>>
{
    fn from_ref(state: &AppState) -> Self {
        state
            .adapter_installations
            .as_ref()
            .map(|installation| installation.github_connector())
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<moatless_adapter_linear::adapter::LinearAdapter>>
{
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.linear())
    }
}

impl axum::extract::FromRef<AppState>
    for Option<Arc<moatless_adapter_linear::installation::Installation>>
{
    fn from_ref(state: &AppState) -> Self {
        state
            .adapter_installations
            .as_ref()
            .map(|installation| installation.linear_installation())
    }
}
