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
use tracing::info;
use tracing_subscriber::EnvFilter;

use moatless_sandbox::agent;
use moatless_sandbox::backend_client::BackendGrpcClient;
use moatless_sandbox::config::Config;
use moatless_sandbox::events::GrpcEventPublisher;
use moatless_sandbox::grpc_server::SandboxServiceImpl;
use moatless_sandbox::init::InitPhase;
use moatless_sandbox::state::SandboxState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = Config::from_env();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
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
    let backend_client = BackendGrpcClient::connect(&config.backend_grpc_url, config.user_auth_token.clone())
        .expect("Failed to create backend gRPC client");
    info!(url = %config.backend_grpc_url, "Backend gRPC client created (lazy connection)");

    // Create event publisher backed by gRPC
    let event_publisher = GrpcEventPublisher::new(backend_client.clone());

    // Build shared state with init phase tracking
    let state = SandboxState {
        config: Arc::new(config.clone()),
        events: event_publisher,
        agent: Arc::new(agent::agent_state::AgentState::new()),
        backend_client,
        init_phase: Arc::new(RwLock::new(InitPhase::Starting)),
        credentials: Arc::new(RwLock::new(std::collections::HashMap::new())),
    };

    // Start gRPC server for sandbox service (receives commands from backend)
    // Available even during init so backend can reach the pod
    let grpc_state = state.clone();
    let grpc_port = config.grpc_port;
    tokio::spawn(async move {
        let svc = SandboxServiceImpl { state: grpc_state };
        let addr = SocketAddr::from(([0, 0, 0, 0], grpc_port));
        info!(port = grpc_port, "Starting sandbox gRPC server");
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(
                sandbox_proto::sandbox_service_server::SandboxServiceServer::new(svc),
            )
            .serve(addr)
            .await
        {
            tracing::error!(error = %e, "Sandbox gRPC server failed");
        }
    });

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

    // Start HTTP server (K8s health probes only)
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    info!(port = config.port, "Sandbox HTTP server started (health only)");

    axum::serve(listener, router).await?;

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
