use crate::config::AgentModelDefaults;
use crate::config::UiRpcConfig;
use crate::config::UiRpcConfigError;
use crate::config::environment_id;
use crate::dao::PgConversationDao;
use crate::observability;
use crate::ports;
use crate::preview::PreviewState;
use crate::preview_relay::{PreviewRelay, PreviewRelayWorker};
use crate::preview_tabs::PreviewTabTtl;
use crate::reads::T3Reads;
use crate::routes::router;
use crate::sandbox::SandboxState;
use crate::scripts::ScriptsState;
use crate::startup_scripts::StartupScripts;
use crate::state::{UiRpcDbPool, UiRpcState};
use crate::subtasks::SubtasksState;
use crate::terminal;
use crate::terminal_relay::{TerminalRelay, TerminalRelayWorker};
use axum::Router;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_ports::activity::ActivitySource;
use moatless_ports::sandbox::ClusterDiagnostics;
use moatless_ports::sandbox::SandboxIntentReader;
use moatless_ports::sandbox::SandboxObserver;
use moatless_ports::sandbox::SandboxOperator;
use moatless_ports::sandbox::SandboxPortPublisher;
use moatless_ports::sandbox::SandboxStarter;
use moatless_ports::sandbox::SandboxStopper;
use moatless_ports::sandbox::WorkspaceSnapshotSource;
use moatless_tasks::task_turn::usage::TurnUsage;
use moatless_types::http::HttpTimeouts;
use moatless_types::ui_rpc::T3UiRpcConfig;
use sandbox_proto::client::SandboxConnector;
use std::sync::Arc;
use tracing::info;

/// [`mount`] the T3 compatibility router when `settings.enabled` turns it on.
pub async fn router_if_enabled(
    graph: &T3Graph,
    dependencies: UiRpcDependencies,
    settings: T3UiRpcConfig,
) -> Result<Option<UiRpc>, UiRpcConfigError> {
    if !settings.enabled {
        return Ok(None);
    }
    mount(graph, dependencies, settings).await.map(Some)
}

/// Build the T3 compatibility router for `settings`, enabled or not.
///
/// The global Moatless auth middleware protects every route except the public
/// discovery descriptor.
///
/// The one thing here that is *not* configuration is the environment's
/// identity, which is read from the database. See `environment_id`.
pub async fn mount(
    graph: &T3Graph,
    dependencies: UiRpcDependencies,
    settings: T3UiRpcConfig,
) -> Result<UiRpc, UiRpcConfigError> {
    let UiRpcDependencies {
        session_cookie_name,
        session_jwt_secret,
        pairing,
        workspace_script_commands,
        sandboxes,
        snapshots,
        observer,
        diagnostics,
        ports,
        intent,
        starter,
        stopper,
        operator,
        preview_tab_ttl,
        model_defaults,
        agents,
        branches,
        skills,
        relations,
        pull_requests,
        source_control_auth,
        commands,
        attachments,
        otlp_traces_enabled,
        frontend_url,
        client_traces_http,
        usage_limits_http,
    } = dependencies;
    let reads = graph.reads.clone();
    let environment = reads.listing_head().await?.identity;
    let usage_limit_sources =
        crate::usage_limit_sources::UsageLimitSources::start(&settings, usage_limits_http);
    let config = UiRpcConfig {
        environment_id: environment_id(&environment),
        orchestration_protocol_version: settings.orchestration_protocol_version,
        allowed_origins: settings.allowed_origins,
        otlp_traces_enabled,
        attachment_uploads: session_jwt_secret.is_some(),
        usage_limit_sources: usage_limit_sources.configured(),
    };
    config.validate()?;
    crate::rpc::metrics::describe();
    let client_traces = observability::ClientTraceForwarder::configured(
        settings.client_traces_endpoint,
        client_traces_http,
    );
    info!(
        environment_id = %config.environment_id,
        // Absent means the T3 client's own spans are dropped rather than
        // forwarded, which is the difference between half a trace and a whole
        // one — see [`observability`].
        client_traces_endpoint = client_traces.endpoint().unwrap_or("(none)"),
        "T3 UI RPC compatibility routes registered"
    );
    // Built before the state literal because two of its fields hold it: a
    // Script with a port opens its own preview tab (`scripts::run`).
    let preview = PreviewState {
        tabs: Arc::new(crate::preview_tabs::PreviewTabs::new(preview_tab_ttl)),
        tasks: reads.tasks_arc(),
        relay: PreviewRelay::new(graph.pool.clone()),
    };
    // Cloned before the state literal takes it: the worker applies the other
    // replicas' opens into the same store the routes read from.
    let preview_relay = PreviewRelayWorker::new(preview.clone());
    // Both held outside the state literal because the startup runner is handed
    // the same two: a Script started at Sandbox Ready has to land in the hub
    // the RPC surface serves terminal reads from.
    let scripts = ScriptsState {
        tasks: reads.tasks_arc(),
        ports,
        preview: preview.clone(),
    };
    let terminal = terminal::TerminalHub::new(TerminalRelay::new(graph.pool.clone()));
    // Cloned before the state literal takes it: the worker applies the other
    // replicas' starts into the same hub the routes serve terminal reads from.
    let terminal_relay = TerminalRelayWorker::new(terminal.clone());
    let startup_scripts = StartupScripts::new(
        scripts.clone(),
        reads.clone(),
        sandboxes.clone(),
        terminal.clone(),
    );
    let router = router(UiRpcState {
        rpc: Arc::new(config),
        environment: Arc::new(environment),
        session_cookie_name: Arc::from(session_cookie_name),
        frontend_url: Arc::from(frontend_url),
        session_jwt_secret: session_jwt_secret.map(Arc::from),
        pairing,
        workspace_script_commands,
        sandboxes,
        snapshots,
        scripts,
        sandbox: SandboxState {
            observer,
            tasks: reads.tasks_arc(),
            intent,
            starter,
            stopper,
            operator,
            diagnostics,
        },
        subtasks: SubtasksState {
            relations,
            conversations: reads.conversations_arc(),
        },
        preview,
        model_defaults: Arc::new(model_defaults),
        agents,
        branches,
        skills,
        pull_requests,
        source_control_auth,
        commands,
        attachments,
        usage: graph.usage.clone(),
        reads,
        terminal,
        client_traces,
        usage_limit_sources,
    });
    Ok(UiRpc {
        router,
        startup_scripts,
        preview_relay,
        terminal_relay,
    })
}

/// The T3 band's own persistence, built once at the composition root.
///
/// One [`PgConversationDao`] serves the conversation and task read views, so a
/// request spanning them shares one pool and one access filter.
#[derive(Clone)]
pub struct T3Graph {
    /// The one write this crate makes for itself: the preview relay records an
    /// open here. Every other write goes out through a port, and every read
    /// through [`T3Reads`].
    pool: UiRpcDbPool,
    reads: T3Reads,
    usage: Arc<dyn ports::UsageReporter>,
}

/// Build the T3 read projections over `db`.
///
/// `activities` is the Task band's activity rows, which this crate projects and
/// does not own; `usage` is the Task band's Turn totals.
pub fn t3_graph(
    db: UiRpcDbPool,
    activities: Arc<dyn ActivitySource>,
    usage: Arc<TurnUsage>,
) -> T3Graph {
    let conversations = Arc::new(PgConversationDao::new(db.clone(), activities));
    T3Graph {
        pool: db.clone(),
        reads: T3Reads::new(
            db.clone(),
            conversations.clone(),
            moatless_workspaces::compose::workspaces(db),
            conversations,
        ),
        usage,
    }
}

impl T3Graph {
    /// The read projections a caller outside this crate resolves a Task
    /// through.
    pub fn reads(&self) -> &T3Reads {
        &self.reads
    }

    /// What this deployment has spent, bucketed the way the usage panel reads
    /// it.
    #[must_use]
    pub fn usage(&self) -> Arc<dyn ports::UsageReporter> {
        self.usage.clone()
    }
}

/// The T3 compatibility module: the routes it serves, the Scripts it starts
/// when a Sandbox becomes Ready, and the two relays that keep preview tabs and
/// terminal sessions consistent across replicas.
///
/// One value because the rest reach only state the router owns; the Task band
/// subscribes the Scripts and the root supervisor runs the relays.
pub struct UiRpc {
    pub router: Router,
    pub startup_scripts: StartupScripts,
    pub preview_relay: PreviewRelayWorker,
    pub terminal_relay: TerminalRelayWorker,
}

/// Everything the T3 compatibility routes need from the rest of the backend.
///
/// A struct rather than an argument list because these are opaque handles whose
/// order carries no meaning; what this crate reads for itself comes from
/// [`T3Graph`] instead.
pub struct UiRpcDependencies {
    /// Reported in the auth descriptor so a browser knows which cookie to send.
    pub session_cookie_name: String,
    /// The secret WebSocket tickets are signed with — the same one session
    /// cookies use. The root backend reads it from its auth configuration
    /// before mounting this router.
    pub session_jwt_secret: Option<String>,
    pub pairing: Arc<DevicePairing>,
    pub workspace_script_commands: Arc<dyn ports::WorkspaceScriptCommands>,
    /// Reaches a conversation's Sandbox, which is where its checkout lives.
    pub sandboxes: Arc<dyn SandboxConnector>,
    /// Reads a conversation's files from the S3 mirror when its Sandbox is not
    /// running. `None` when the deployment stores no snapshots.
    pub snapshots: Option<Arc<dyn WorkspaceSnapshotSource>>,
    /// Reports what a Task's Sandbox is doing, without reaching inside.
    pub observer: Arc<dyn SandboxObserver>,
    /// Fleet-wide Sandbox status and live resource usage, deployment-scoped
    /// rather than per-Task.
    pub diagnostics: Arc<dyn ClusterDiagnostics>,
    /// Publishes a Script's port on a running Sandbox. The Kubernetes provider
    /// patches the Service and creates the route; the Docker provider returns
    /// `None` (no way out of the fixed-port agent container).
    pub ports: Arc<dyn SandboxPortPublisher>,
    /// The desired state a status read is resolved against.
    pub intent: Arc<dyn SandboxIntentReader>,
    /// Starts a Task's Sandbox through the same lifecycle path as the REST API.
    pub starter: Arc<dyn SandboxStarter>,
    /// Stops a Task's Sandbox through the same lifecycle path as the REST API.
    pub stopper: Arc<dyn SandboxStopper>,
    /// Restarts, redeploys and cleans up a Task's Sandbox, through the same
    /// lifecycle path as the REST API.
    pub operator: Arc<dyn SandboxOperator>,
    /// How long a Task's tabs outlive their last write.
    pub preview_tab_ttl: PreviewTabTtl,
    pub model_defaults: AgentModelDefaults,
    /// Which agents this viewer can run, and with which models.
    pub agents: Arc<dyn ports::AgentCatalogSource>,
    /// A repository's branches, read from its git host, for a draft thread's
    /// branch picker.
    pub branches: Arc<dyn ports::RepositoryBranchSource>,
    /// The skills a Workspace offers, for the composer's `/` menu.
    pub skills: Arc<dyn ports::SkillCatalog>,
    /// The child edges behind `subtasks.list`. Access is applied afterwards, by
    /// re-reading each child through `conversations`.
    pub relations: Arc<dyn ports::TaskRelationSource>,
    /// The pull request a Task is bound to, and reference resolution for the
    /// attach-PR dialog.
    pub pull_requests: Arc<dyn ports::BoundPullRequestSource>,
    /// The viewer's usable GitHub credential, for the Source Control panel.
    pub source_control_auth: Arc<dyn ports::SourceControlAuthSource>,
    /// Carries out the commands the T3 client dispatches.
    pub commands: Arc<dyn ports::ThreadCommandSink>,
    /// Where a chat attachment's bytes go. Uploads are stored as ordinary
    /// Moatless uploads, under the id `attachments.createUploadUrl` minted.
    pub attachments: Arc<dyn ports::AttachmentStore>,
    /// Whether this deployment exports spans over OTLP (an OTLP endpoint is
    /// configured — see `config::OtelConfig`), reported to the client as
    /// `observability.otlpTracesEnabled`.
    pub otlp_traces_enabled: bool,
    /// This deployment's own web address (`config.frontend_url`), for the link
    /// a Task origin carries back into the Moatless UI.
    pub frontend_url: String,
    /// Bounds the forward of a client's trace batch to the collector.
    pub client_traces_http: HttpTimeouts,
    /// Bounds each read of the usage-limit hub.
    pub usage_limits_http: HttpTimeouts,
}
