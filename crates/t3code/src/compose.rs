use crate::config::AgentModelDefaults;
use crate::config::UiRpcConfig;
use crate::config::UiRpcConfigError;
use crate::config::env_enabled;
use crate::config::environment_id;
use crate::dao::ConversationDao;
use crate::observability;
use crate::ports;
use crate::preview::PreviewState;
use crate::preview_tabs::PreviewTabDao;
use crate::routes::router;
use crate::sandbox::SandboxState;
use crate::scripts::ScriptsState;
use crate::servers::ServersState;
use crate::startup_scripts::StartupScripts;
use crate::state::UiRpcState;
use crate::subtasks::SubtasksState;
use crate::task_dao::T3TaskDao;
use crate::terminal;
use crate::workspace_dao::T3WorkspaceDao;
use axum::Router;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_ports::sandbox::ClusterDiagnostics;
use moatless_ports::sandbox::SandboxIntentReader;
use moatless_ports::sandbox::SandboxObserver;
use moatless_ports::sandbox::SandboxOperator;
use moatless_ports::sandbox::SandboxPortPublisher;
use moatless_ports::sandbox::SandboxStarter;
use moatless_ports::sandbox::SandboxStopper;
use moatless_ports::sandbox::WorkspaceSnapshotSource;
use sandbox_proto::client::SandboxConnector;
use std::env;
use std::sync::Arc;
use tracing::info;

/// Build the T3 compatibility router when its environment gate is enabled.
///
/// Configuration stays at this HTTP boundary rather than becoming part of
/// sandbox or pod configuration. The global Moatless auth middleware protects
/// every route except the public discovery descriptor.
///
/// The one thing here that is *not* configuration is the environment's
/// identity, which is read from the database. See `environment_id`.
pub async fn router_from_env(
    dependencies: UiRpcDependencies,
) -> Result<Option<UiRpc>, UiRpcConfigError> {
    if !env_enabled("T3_UI_RPC_ENABLED")? {
        return Ok(None);
    }

    let UiRpcDependencies {
        session_cookie_name,
        session_jwt_secret,
        pairing,
        workspaces,
        workspace_script_commands,
        conversations,
        sandboxes,
        snapshots,
        observer,
        diagnostics,
        ports,
        tasks,
        intent,
        starter,
        stopper,
        operator,
        preview_tabs,
        model_defaults,
        agents,
        branches,
        skills,
        relations,
        pull_requests,
        source_control_auth,
        commands,
        attachments,
        usage,
        otlp_traces_enabled,
        frontend_url,
    } = dependencies;
    let environment = conversations.listing_head().await?.identity;
    let usage_limit_sources =
        crate::usage_limit_sources::UsageLimitSources::from_env(reqwest::Client::new());
    let config = UiRpcConfig {
        environment_id: environment_id(&environment),
        allowed_origins: moatless_types::auth::parse_allowed_origins(
            &env::var("T3_UI_RPC_ALLOWED_ORIGINS").unwrap_or_default(),
        ),
        otlp_traces_enabled,
        attachment_uploads: session_jwt_secret.is_some(),
        usage_limit_sources: usage_limit_sources.configured(),
    };
    config.validate()?;
    crate::rpc::metrics::describe();
    let client_traces = observability::ClientTraceForwarder::from_env();
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
        tabs: preview_tabs,
        tasks: tasks.clone(),
    };
    // Both held outside the state literal because the startup runner is handed
    // the same two: a Script started at Sandbox Ready has to land in the hub
    // the RPC surface serves terminal reads from.
    let scripts = ScriptsState {
        workspaces: workspaces.clone(),
        ports,
        preview: preview.clone(),
    };
    let terminal = terminal::TerminalHub::default();
    let startup_scripts = StartupScripts::new(
        scripts.clone(),
        conversations.clone(),
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
        workspaces: workspaces.clone(),
        workspace_script_commands,
        conversations: conversations.clone(),
        sandboxes,
        snapshots,
        servers: ServersState {
            observer: observer.clone(),
            tasks: tasks.clone(),
            intent: intent.clone(),
        },
        scripts,
        sandbox: SandboxState {
            observer,
            tasks: tasks.clone(),
            intent,
            starter,
            stopper,
            operator,
            diagnostics,
        },
        subtasks: SubtasksState {
            relations,
            conversations,
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
        usage,
        terminal,
        client_traces,
        usage_limit_sources,
    });
    Ok(Some(UiRpc {
        router,
        startup_scripts,
    }))
}

/// The two halves of the T3 compatibility module: the routes it serves, and the
/// Scripts it starts on its own when a Sandbox becomes Ready.
///
/// One value because the second half is only reachable through state the first
/// half owns; the Task band subscribes it to the Ready event.
pub struct UiRpc {
    pub router: Router,
    pub startup_scripts: StartupScripts,
}

/// Everything the T3 compatibility routes need from the rest of the backend.
///
/// A struct rather than an argument list because all six are opaque handles
/// whose order carries no meaning at the call site.
pub struct UiRpcDependencies {
    /// Reported in the auth descriptor so a browser knows which cookie to send.
    pub session_cookie_name: String,
    /// The secret WebSocket tickets are signed with — the same one session
    /// cookies use. The root backend reads it from its auth configuration
    /// before mounting this router.
    pub session_jwt_secret: Option<String>,
    pub pairing: Arc<DevicePairing>,
    pub workspaces: Arc<dyn T3WorkspaceDao>,
    pub workspace_script_commands: Arc<dyn ports::WorkspaceScriptCommands>,
    pub conversations: Arc<dyn ConversationDao>,
    /// Reaches a conversation's Sandbox, which is where its checkout lives.
    pub sandboxes: Arc<dyn SandboxConnector>,
    /// Reads a conversation's files from the S3 mirror when its Sandbox is not
    /// running. `None` when the deployment stores no snapshots.
    pub snapshots: Option<Arc<dyn WorkspaceSnapshotSource>>,
    /// Reports what a Task's servers are doing, without reaching inside.
    pub observer: Arc<dyn SandboxObserver>,
    /// Fleet-wide Sandbox status and live resource usage, deployment-scoped
    /// rather than per-Task.
    pub diagnostics: Arc<dyn ClusterDiagnostics>,
    /// Publishes a Script's port on a running Sandbox. The Kubernetes provider
    /// patches the Service and creates the route; the Docker provider returns
    /// `None` (no way out of the fixed-port agent container).
    pub ports: Arc<dyn SandboxPortPublisher>,
    /// Authorizes every `servers.*`, `sandbox.*`, and `preview.*` call: a
    /// scoped read that finds nothing is the same answer as a Task that does
    /// not exist.
    pub tasks: Arc<dyn T3TaskDao>,
    /// The desired state a status read is resolved against.
    pub intent: Arc<dyn SandboxIntentReader>,
    /// Starts a Task's Sandbox through the same lifecycle path as the REST API.
    pub starter: Arc<dyn SandboxStarter>,
    /// Stops a Task's Sandbox through the same lifecycle path as the REST API.
    pub stopper: Arc<dyn SandboxStopper>,
    /// Restarts, redeploys and cleans up a Task's Sandbox, through the same
    /// lifecycle path as the REST API.
    pub operator: Arc<dyn SandboxOperator>,
    pub preview_tabs: Arc<dyn PreviewTabDao>,
    pub model_defaults: AgentModelDefaults,
    /// Which agents this viewer can run, and with which models.
    pub agents: Arc<dyn ports::AgentCatalogSource>,
    /// A repository's branches, read from its git host, for a draft thread's
    /// branch picker.
    pub branches: Arc<dyn ports::RepositoryBranchSource>,
    /// The skills a Workspace offers, for the composer's `/` menu.
    pub skills: Arc<dyn ports::SkillCatalog>,
    /// The child edges behind `subtasks.list`. Access is applied afterwards,
    /// by re-reading each child through `conversations`.
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
    /// Token and cost totals behind `server.getUsageSummary`.
    pub usage: Arc<dyn ports::UsageReporter>,
    /// Whether this deployment exports spans over OTLP (an OTLP endpoint is
    /// configured — see `config::OtelConfig`), reported to the client as
    /// `observability.otlpTracesEnabled`.
    pub otlp_traces_enabled: bool,
    /// This deployment's own web address (`config.frontend_url`), for the link a
    /// Task origin carries back into the Moatless UI.
    pub frontend_url: String,
}
