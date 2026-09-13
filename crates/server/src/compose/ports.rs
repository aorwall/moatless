//! The composition root.
//!
//! [`compose`] answers "given process configuration, what ports exist", and is
//! where the ports a process holds are constructed. A caller that receives
//! [`Ports`] takes the ports it needs rather than a pool and a `Pg*Dao`
//! constructor. Holding a [`DbPool`] is a separate question — raw-pool services
//! keep their own and are unaffected.

use std::sync::Arc;

use moatless_adapter_model::connections::{AdapterConnections, ConnectionRouting};
use moatless_adapter_model::loops::LoopReader;
use moatless_core::config::Config;
use moatless_core::db::DbPool;
use moatless_core::file_storage::Files;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_identity::credentials::CredentialService;
use moatless_identity::forgejo::{ForgejoAccess, ForgejoInstances};
use moatless_identity::git_host_tokens::GitHostTokens;
use moatless_identity::secret::audit::SecretAudit;
use moatless_identity::secret::service::LocalEnvelopeSecretCipher;
use moatless_identity::secret::service::SecretService;
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::ports::BindingDirectory;
use moatless_loops::compose::LoopGraph;
use moatless_loops::loops::operations::Loops;
use moatless_sandbox_control::compose::SandboxGraph;
use moatless_tasks::compose::TaskGraph;
use moatless_tasks::feedback::operations::FeedbackNotes;
use moatless_tasks::message::operations::Messages;
use moatless_tasks::tag::operations::Tags;
use moatless_tasks::task::fork::TaskForks;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::relations::TaskRelations;
use moatless_tasks::task_event::operations::TaskEvents;
use moatless_tasks::task_turn::operations::TaskTurns;
use moatless_workspaces::plugin::Plugins;
use moatless_workspaces::repository::Repositories;
use moatless_workspaces::workspace::Workspaces;
use moatless_workspaces::workspace::credentials::service::WorkspaceCredentialService;

/// The ports one process has, plus the pool they were built from.
///
/// Cheap to clone (every field is `Arc`-wrapped); config is the input to
/// [`compose`], not part of its answer, so holding ports doesn't widen what a
/// consumer can read.
#[derive(Clone)]
pub struct Ports {
    /// The main connection pool. Raw-pool services take it from here rather
    /// than opening their own.
    pub pool: DbPool,

    pub tasks: Arc<TaskCommands>,
    /// E-(b) session-gate port: the one `may_access_task` bool the auth
    /// `ForwardAuth` gate needs, over `TaskDao`, so identity holds no `task`
    /// import.
    pub task_access: Arc<dyn moatless_core::http_auth::TaskAccess>,
    pub task_relations: Arc<TaskRelations>,
    /// Where a fork's first turn branches from. Read by the creation service
    /// that writes it and by the delivery path that spends it.
    pub task_forks: Arc<TaskForks>,
    /// Task↔external-subject bindings (`task_bindings`).
    pub task_bindings: Arc<Bindings>,
    /// The binding store and the Loops that label its rows, behind the
    /// `TaskBindings` seam the Task band reads. Also serves `BindingProbe`.
    pub binding_directory: Arc<BindingDirectory>,
    pub task_events: Arc<TaskEvents>,
    /// Authoritative per-turn lifecycle state (`task_turns`).
    pub task_turns: Arc<TaskTurns>,
    pub messages: Arc<Messages>,
    pub feedback: Arc<FeedbackNotes>,
    pub repositories: Arc<Repositories>,
    /// The narrowed read the task and loop bands take, over the same store.
    pub repository_reader: Arc<dyn moatless_workspace_ports::RepositoryReader>,
    /// Workspace composition + run-config (`workspaces` / `workspace_repos`).
    /// The `tasks` table behind the published `TaskReader` seam.
    pub task_reader: Arc<moatless_tasks::ports::TaskProjection>,
    /// The `task_messages` table behind the published `TranscriptAppend` seam.
    pub transcript: Arc<moatless_tasks::ports::Transcript>,
    /// The `task_turns` table behind the published `TurnCounter` seam.
    pub turn_counter: Arc<moatless_tasks::ports::TurnLedger>,
    /// The `tags` table behind the published `TagResolver` seam.
    pub tag_resolver: Arc<moatless_tasks::ports::TagCatalog>,
    pub workspaces: Arc<Workspaces>,
    /// The workspace, placement, and plugin stores behind the
    /// `WorkspaceResolver` port.
    pub workspace_resolver: Arc<moatless_workspaces::ports::WorkspaceProjection>,
    /// Provider credentials pinned to a Workspace (`workspace_credentials`).
    /// The highest-precedence source of a sandbox's provider credentials.
    pub workspace_credentials: Arc<WorkspaceCredentialService>,
    pub tags: Arc<Tags>,
    /// Uploads: the object, the metadata row, and who may read one back.
    pub files: Arc<Files>,
    pub plugins: Arc<Plugins>,
    /// The four Loop tables (`loops`, `loop_configs`, `subscriptions`,
    /// `schedules`), as a host reads and writes them.
    pub loops: Arc<Loops>,
    /// The Subscription reads an Adapter's webhook route makes.
    pub loop_reader: Arc<dyn LoopReader>,
    /// Adapter connections (`adapter_connections`).
    pub adapter_connections: Arc<AdapterConnections>,
    /// The same rows an Adapter routes against, unscoped.
    pub connection_routing: Arc<ConnectionRouting>,
    /// Durable admission for inbound Adapter deliveries. Shared by Adapter
    /// ingress and the worker-facing application state.
    pub adapter_delivery: Arc<dyn moatless_adapter_model::delivery::DeliveryAdmission>,
    pub adapter_delivery_worker: moatless_loops::adapters::delivery::AdapterDeliveryWorker,

    /// The `users` table behind the three identity ports peer crates read:
    /// `UserDisplay`, `UserResolver`, `CommitIdentity`.
    pub user_directory: Arc<moatless_identity::ports::UserDirectory>,
    /// The `provider_tokens` table behind the `HarnessCredentials` port.
    pub harness_credentials: Arc<moatless_identity::ports::HarnessTokens>,
    /// The `adapter_apps` tables behind the `AdapterAppReader` port an Adapter
    /// resolves its provider credentials through.
    pub adapter_apps: Arc<dyn moatless_adapter_model::apps::AdapterAppReader>,
    /// The one way to read, persist and refresh a provider-token credential.
    /// One per process: the lock that serializes a concurrent store lives in
    /// it.
    pub credentials: Arc<CredentialService>,
    /// Shared HTTP client for the Codex refresh path, kept warm across calls.
    pub codex_http: Arc<reqwest::Client>,
    /// Verifies sandbox-issued user tokens behind a port, so the token secret
    /// and `sandbox::jwt` stay out of the auth handlers that consume it.
    pub token_verifier: Arc<dyn moatless_core::http_auth::UserTokenVerifier>,
    /// The `secret_audit_events` trail `WorkspaceCredentialService` records its
    /// own writes to.
    pub secret_audits: Arc<SecretAudit>,

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
    /// The one way to a git-host token: a named identity and a repository, with
    /// no scope walk. Built here so the five credential ports it reads are
    /// supplied in one place.
    pub git_host_tokens: Arc<GitHostTokens>,
    /// The OAuth2 application registered for each Forgejo instance. One
    /// instance per process, shared by the connect flow and by token renewal.
    pub forgejo_instances: Arc<ForgejoInstances>,
    /// One person's Forgejo credential, per instance. Built here because the
    /// auth capsule's connect flow and the settings routes write the same rows.
    pub forgejo_access: Arc<ForgejoAccess>,

    /// The T3 band's own stores, for the UI-RPC router to read through.
    pub(crate) t3_graph: moatless_t3code::T3Graph,

    /// The sandbox band's store, for `AppState` to assemble the lifecycle
    /// handle and the reconciler over once the runtime and connector exist.
    pub(crate) sandbox_graph: SandboxGraph,

    /// The tasks band's stores, for `AppState` to assemble the workflow
    /// services over once the lifecycle handle and the connector exist.
    pub(crate) task_graph: TaskGraph,
    /// The loops band's stores, for `AppState` to build the routing, sync and
    /// reply services over once the ports they read exist.
    pub(crate) loop_graph: LoopGraph,
    /// The unclaimed-user-message predicate the desired-state reconciler reads.
    pub(crate) pending_work: Arc<moatless_tasks::ports::PendingWork>,
}

/// Build every port this process has from its pool and configuration.
pub fn compose(pool: DbPool, config: &Config, s3: Option<moatless_s3::S3Storage>) -> Ports {
    let cipher = LocalEnvelopeSecretCipher::from_config(&config.secrets);

    let secret_audits = moatless_identity::compose::secret_audit(pool.clone());
    let secrets = moatless_identity::compose::secret_service(pool.clone(), cipher.clone());

    let workspaces = moatless_workspaces::compose::workspaces(pool.clone());
    let workspace_credentials = moatless_workspaces::compose::workspace_credentials(
        pool.clone(),
        secret_audits.clone(),
        cipher.clone(),
    );

    let repositories = moatless_workspaces::compose::repositories(pool.clone());
    let repository_reader = moatless_workspaces::compose::repository_reader(pool.clone());
    let plugins = moatless_workspaces::compose::plugins(pool.clone());
    let workspace_resolver = moatless_workspaces::compose::workspace_resolver(pool.clone());

    let connections = moatless_adapter_model::compose::adapter_connections(pool.clone());

    let pairing = moatless_identity::compose::device_pairing(pool.clone());

    let github_apps =
        moatless_identity::compose::github_app_registry(config, Some(pool.clone()), cipher.clone());
    let github_app_service = github_apps.default_service();

    let codex_http = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            // Branch listing shares this client, so it reaches a self-hosted
            // Forgejo. See `moatless_core::USER_AGENT`.
            .user_agent(moatless_core::USER_AGENT)
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "Failed to build reqwest client for Codex refresh, falling back to default"
                );
                reqwest::Client::new()
            }),
    );
    let credentials =
        moatless_identity::compose::credentials(pool.clone(), secrets.clone(), codex_http.clone());
    let harness_credentials = moatless_identity::compose::harness_credentials(pool.clone());
    let forgejo_instances = moatless_identity::compose::forgejo_instances(pool.clone(), cipher);
    let forgejo_access = moatless_identity::compose::forgejo_access(
        pool.clone(),
        secrets.clone(),
        forgejo_instances.clone(),
    );
    let git_host_tokens = moatless_identity::compose::git_host_tokens(
        pool.clone(),
        github_app_service.clone(),
        github_apps.clone(),
        secrets.clone(),
        Some(workspace_credentials.clone()),
        forgejo_instances.clone(),
    );

    // The tasks band's own construction root owns its stores; this process takes
    // the capabilities over them, and `AppState` consumes the graph to assemble
    // the workflow services.
    let task_graph = moatless_tasks::compose::task_graph(pool.clone());

    // The loops band's own construction root owns its stores; this process takes
    // the capabilities over them, and `AppState` builds the routing, sync and
    // reply services from the graph.
    let loop_graph = moatless_loops::compose::loop_graph(pool.clone());
    let task_bindings = loop_graph.bindings();
    let binding_directory = loop_graph.binding_directory();
    let loops = loop_graph.loops();
    let loop_reader = loop_graph.loop_reader();
    let adapter_delivery = loop_graph.delivery_admission();
    let adapter_delivery_worker = loop_graph.delivery_worker();
    let activities = task_graph.activities();
    let pending_work = task_graph.pending_work();
    let task_events = task_graph.task_events();
    let task_turns = task_graph.task_turns();
    let turn_counter = task_graph.turn_ledger();
    let feedback = task_graph.feedback();
    let tags = task_graph.tags();
    let tag_resolver = task_graph.tag_catalog();

    Ports {
        tasks: task_graph.tasks(),
        task_access: task_graph.task_access(),
        task_reader: task_graph.task_reader(),
        task_relations: task_graph.relation_recorder(),
        task_forks: task_graph.fork_origins(),
        messages: task_graph.messages(),
        transcript: task_graph.transcript(),
        task_graph,
        loop_graph,
        pending_work,
        task_bindings,
        binding_directory,
        task_events,
        task_turns,
        turn_counter,
        feedback,
        repositories,
        repository_reader,
        workspaces,
        workspace_resolver,
        workspace_credentials,
        tags,
        tag_resolver,
        files: moatless_core::file_storage::compose::files(
            pool.clone(),
            s3,
            Arc::new(config.s3.clone()),
        ),
        plugins,
        loops,
        loop_reader,
        adapter_connections: connections.administration(),
        connection_routing: connections.routing(),
        adapter_delivery,
        adapter_delivery_worker,

        user_directory: moatless_identity::compose::user_directory(pool.clone()),
        harness_credentials,
        adapter_apps: Arc::new(moatless_identity::ports::AdapterApps::new(
            pool.clone(),
            Arc::new(config.clone()),
        )),
        credentials,
        codex_http,
        token_verifier: Arc::new(moatless_core::sandbox_jwt::JwtUserTokenVerifier::new(
            config.sandbox.task_token_secret.clone(),
        )),
        secret_audits,

        secrets,
        pairing,
        github_apps,
        github_app_service,
        git_host_tokens,
        forgejo_instances,
        forgejo_access,

        // The T3 band's own construction root owns its read model; this
        // process supplies the activity rows it projects.
        t3_graph: moatless_t3code::t3_graph(pool.clone(), activities),

        // The sandbox band's own construction root owns its store; this process
        // takes the capabilities over it.
        sandbox_graph: moatless_sandbox_control::compose::sandbox_graph(pool.clone()),

        pool,
    }
}

impl Ports {
    /// The sandbox band's construction handle, for a host that assembles its
    /// own lifecycle over mock runtime adapters.
    pub fn sandbox_graph(&self) -> &SandboxGraph {
        &self.sandbox_graph
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
                github_app_service: self.github_app_service.clone(),
                github_apps: self.github_apps.clone(),
                git_host_tokens: self.git_host_tokens.clone(),
                forgejo_access: self.forgejo_access.clone(),
                secrets: self.secrets.clone(),
                task_access: self.task_access.clone(),
                token_verifier: self.token_verifier.clone(),
            },
        )
    }
}
