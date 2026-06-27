//! Moatless Sandbox — runtime service for sandbox pods.
//!
//! Provides gRPC API for agent execution, status, and file search.
//! HTTP is kept only for K8s health probes.
//! Runs inside each sandbox container/pod.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

use moatless_sandbox::agent;
use moatless_sandbox::agent::handle::spawn_agent;
use moatless_sandbox::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
use moatless_sandbox::backend_client::BackendGrpcClient;
use moatless_sandbox::config::Config;
use moatless_sandbox::credentials::broker::{self, BrokerState};
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use moatless_sandbox::credentials::sinks::{ClaudeOauthSink, CodexAuthJsonSink, GitTokenSink};
use moatless_sandbox::credentials::Dispatcher;
use moatless_sandbox::events::GrpcEventPublisher;
use moatless_sandbox::grpc_server::SandboxServiceImpl;
use moatless_sandbox::init::InitPhase;
use moatless_sandbox::shutdown;
use moatless_sandbox::state::SandboxState;

/// Unix socket path for the in-pod credential broker.
///
/// Must match the path baked into `docker/moatless-git-cred.sh`.
const CREDENTIAL_BROKER_SOCKET: &str = "/run/moatless/credentials.sock";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = Config::from_env();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .json()
        .init();

    info!(
        task_id = %config.task_id,
        port = config.port,
        grpc_port = config.grpc_port,
        "Starting Moatless Sandbox"
    );

    // Create gRPC client for backend communication (required)
    #[allow(clippy::expect_used)] // fail-fast: sandbox cannot operate without backend connectivity
    let backend_client =
        BackendGrpcClient::connect(&config.backend_grpc_url, config.user_auth_token.clone())
            .expect("Failed to create backend gRPC client");
    info!(url = %config.backend_grpc_url, "Backend gRPC client created (lazy connection)");

    // Wrap backend client in Arc for shared ownership
    let backend_client: Arc<dyn moatless_sandbox::backend_client::BackendClient> =
        Arc::new(backend_client);

    // Create event publisher backed by gRPC
    let event_publisher = GrpcEventPublisher::new(Arc::clone(&backend_client));

    // Build shared state with init phase tracking
    let agent_state = Arc::new(agent::agent_state::AgentState::new());
    let config_arc = Arc::new(config.clone());
    let credentials = Arc::new(RwLock::new(std::collections::HashMap::new()));

    // Build the scheduled-job registry that owns Monitor crons and
    // ScheduleWakeup timers. The injector is late-bound: we need an
    // `AgentHandle` to inject messages, but the actor (which produces the
    // handle) needs the registry up front. The `LateInjector` breaks the
    // cycle — we construct the registry around it, spawn the actor with the
    // registry, then point the injector at the returned handle.
    let late_injector = Arc::new(LateInjector::new());
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));

    // Build the credential dispatcher and register sinks for every credential
    // type the sandbox knows how to refresh. The watcher handle is set later
    // by the init flow once the codex_watcher is spawned.
    let codex_watcher_handle = Arc::new(RwLock::new(None));
    let mut dispatcher = Dispatcher::new();
    dispatcher.register(
        "CODEX_AUTH_JSON",
        Arc::new(CodexAuthJsonSink {
            codex_home_dir: config.codex_home_dir.clone(),
            watcher_handle: Arc::clone(&codex_watcher_handle),
        }),
    );
    dispatcher.register(
        "GITHUB_TOKEN",
        Arc::new(GitTokenSink {
            home_dir: config.home_dir.clone(),
        }),
    );
    dispatcher.register("CLAUDE_CODE_OAUTH_TOKEN", Arc::new(ClaudeOauthSink));
    let credential_dispatcher = Arc::new(dispatcher);
    let credential_failures = CredentialFailureDetector::default();
    let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

    let agent_handle = spawn_agent(
        Arc::clone(&config_arc),
        Arc::clone(&agent_state),
        event_publisher.clone(),
        Arc::clone(&credentials),
        Arc::clone(&backend_client),
        Arc::clone(&scheduled_jobs),
        credential_failures.clone(),
        Arc::clone(&credential_dispatcher),
        Arc::clone(&system_prompt_cache),
    );
    late_injector.set(Box::new(agent_handle.clone()));

    let state = SandboxState {
        config: config_arc,
        events: event_publisher,
        agent: agent_state,
        agent_handle,
        scheduled_jobs,
        backend_client,
        init_phase: Arc::new(RwLock::new(InitPhase::Starting)),
        credentials,
        lifecycle_cancel: CancellationToken::new(),
        credential_dispatcher,
        credential_failures,
        codex_watcher_handle,
        system_prompt_cache,
    };

    // Start gRPC server for sandbox service (receives commands from backend)
    // Available even during init so backend can reach the pod. Drains via the
    // lifecycle token so a SIGTERM (see shutdown driver below) stops it cleanly.
    let grpc_state = state.clone();
    let grpc_port = config.grpc_port;
    tokio::spawn(async move {
        let grpc_shutdown = grpc_state.lifecycle_cancel.clone();
        let svc = SandboxServiceImpl { state: grpc_state };
        let addr = SocketAddr::from(([0, 0, 0, 0], grpc_port));
        info!(port = grpc_port, "Starting sandbox gRPC server");
        // Raise from tonic's 4 MB default so binary files (audio, images) can round-trip.
        const MAX_MESSAGE_SIZE: usize = 32 * 1024 * 1024;
        let service = sandbox_proto::sandbox_service_server::SandboxServiceServer::new(svc)
            .max_decoding_message_size(MAX_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_MESSAGE_SIZE);
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async move { grpc_shutdown.cancelled().await })
            .await
        {
            tracing::error!(error = %e, "Sandbox gRPC server failed");
        }
        info!("Sandbox gRPC server stopped");
    });

    // Spawn the credential broker on a unix socket. Git is configured to use
    // a helper script that hits this socket on every push/fetch, so tokens
    // are pulled fresh from the backend instead of relying on hosts.yml staying
    // current between 15-min refresh cycles.
    //
    // Spawned BEFORE the init task so the socket exists by the time
    // `init/git.rs` wires the helper into git config.
    let broker_socket = std::path::PathBuf::from(CREDENTIAL_BROKER_SOCKET);
    let broker_state = BrokerState {
        task_id: state.config.task_id.clone(),
        backend_client: Arc::clone(&state.backend_client),
    };
    if let Err(e) = broker::spawn(broker_state, &broker_socket, state.lifecycle_cancel.clone()) {
        tracing::warn!(
            error = %e,
            socket = %broker_socket.display(),
            "Failed to spawn credential broker — git operations will fall back to hosts.yml"
        );
    }

    // Spawn init task — handles credential fetching, git clone, workspace setup.
    // After init completes, it triggers process_pending_messages() to pick up
    // any messages that arrived during initialization.
    let init_state = state.clone();
    tokio::spawn(moatless_sandbox::init::run_init(init_state));
    info!("Init task spawned");

    // Build HTTP router with health probe that reflects init state
    let health_state = state.clone();
    let router = Router::new()
        .route("/health", get(health))
        .with_state(health_state);

    // Drive graceful shutdown on SIGTERM/SIGINT. Without this the process rides
    // the full pod termination grace period and is SIGKILLed (exit 137) on every
    // teardown (idle-down, node drain, eviction, PR-close). On signal we stop the
    // agent and cancel the lifecycle token, which both servers and background
    // tasks observe to wind down.
    shutdown::spawn_shutdown_driver(state.clone());

    // Start HTTP server (K8s health probes only)
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    info!(
        port = config.port,
        "Sandbox HTTP server started (health only)"
    );

    let http_shutdown = state.lifecycle_cancel.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { http_shutdown.cancelled().await })
        .await?;

    info!("Sandbox HTTP server stopped — exiting cleanly");
    Ok(())
}

/// GET /health — returns 200 when init is Ready, 503 otherwise.
///
/// K8s startup probe will retry until Ready, at which point the pod
/// becomes eligible for traffic.
async fn health(State(state): State<SandboxState>) -> impl IntoResponse {
    let phase = state.init_phase.read().await;
    match *phase {
        InitPhase::Ready => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}
