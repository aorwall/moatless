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
use moatless_sandbox::supervisor::{Criticality, Supervisor, SHUTDOWN_JOIN_DEADLINE};

/// Unix socket file for the in-pod credential broker, inside
/// [`config::RUNTIME_DIR`].
///
/// The joined path must match the one baked into `docker/moatless-git-cred.sh`.
const CREDENTIAL_BROKER_SOCKET_FILE: &str = "credentials.sock";

/// Where the Claude CLI writes its debug logs — pointed at `/tmp` to work around
/// an SDK lazy-init race. Child CLI processes inherit it from this process.
///
/// `docker/docker-entrypoint-sandbox.sh` exports the same value and remains the
/// source for image-entrypoint launches, but the K8s pod spec overrides the
/// entrypoint with `command: ["/usr/local/bin/sandbox"]` (see
/// `backend/src/sandbox/k8s/pod_builder.rs`), so on that path the script never
/// runs and the binary has to guarantee the variable itself.
const CLAUDE_CODE_DEBUG_LOGS_DIR: &str = "/tmp/claude-debug.log";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set before the Tokio runtime exists: `set_var` is only sound while the
    // process is single-threaded, and every consumer is a CLI child process
    // spawned much later, so the pre-runtime prologue is the one safe place.
    #[expect(
        unsafe_code,
        reason = "std::env::set_var is unsafe; sound here because the process is still single-threaded"
    )]
    // SAFETY: no runtime, no worker threads, and no other thread of any kind has
    // been created yet, so nothing can concurrently read the environment.
    unsafe {
        std::env::set_var("CLAUDE_CODE_DEBUG_LOGS_DIR", CLAUDE_CODE_DEBUG_LOGS_DIR);
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = Config::from_env();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .json()
        // Lift event fields (incl. `message`) to the JSON root so Loki's
        // `| json | message=…` works directly instead of `fields_message`.
        .flatten_event(true)
        // Render the current span's fields under a compact `span` object
        // (queryable in Loki as `span_<field>`); drop the `spans` array since
        // JSON arrays aren't field-queryable there.
        .with_current_span(true)
        .with_span_list(false)
        .init();

    // The one-shot modes run to completion and exit; nothing below is built.
    // Checked here, after logging and before the backend client, because a
    // one-shot has no backend to reach: it is scheduled onto a volume whose
    // sandbox is already gone, holding a presigned URL and nothing else.
    if let Some(mode) = std::env::var("SANDBOX_ONESHOT").ok().filter(|m| !m.is_empty()) {
        return run_oneshot(&mode, &config).await;
    }

    info!(
        task_id = %config.task_id,
        port = config.port,
        grpc_port = config.grpc_port,
        "Starting Moatless Sandbox"
    );

    // Create gRPC client for backend communication (required). The sandbox
    // starts with the best available initial token and swaps to the long-lived
    // user token delivered over `GetCredentials`.
    #[allow(clippy::expect_used)] // fail-fast: sandbox cannot operate without backend connectivity
    let backend_client = BackendGrpcClient::connect(
        &config.backend_grpc_url,
        config.resolve_initial_auth_token(),
    )
    .expect("Failed to create backend gRPC client");
    info!(url = %config.backend_grpc_url, "Backend gRPC client created (lazy connection)");

    // Wrap backend client in Arc for shared ownership
    let backend_client: Arc<dyn moatless_sandbox::backend_client::BackendClient> =
        Arc::new(backend_client);

    // Lifecycle token, created up front so the event outbox drainer shares it
    // with the rest of the runtime and winds down on SIGTERM.
    let lifecycle_cancel = CancellationToken::new();

    // Root supervisor: owns the join handle of every long-lived task below under
    // a policy declared at registration. `registrar` is cloned into anything that
    // spawns supervised work later (the init task's durability loops).
    let supervisor = Supervisor::new(lifecycle_cancel.clone());
    let registrar = supervisor.registrar();

    // Spawn the at-least-once event send buffer and back the publisher with it.
    // The drainer sends events in order with retry/backoff across backend
    // rollout and network interruptions.
    //
    // Fatal: once the drainer stops, `EventOutbox::enqueue` refuses admission in
    // the same critical section as the buffer push and the outbox stays stopped.
    // A pod that cannot publish events makes its task appear frozen, so the
    // right remedy is to replace it. `into_join_handle` is the escape hatch the
    // outbox provides for exactly this: the supervisor owns termination policy,
    // so the outbox's own `shutdown(deadline)` is not used.
    //
    // The supervisor owns this task's JOIN, never the timing of its
    // cancellation. `shutdown::perform_shutdown` flushes the outbox and only
    // then cancels the lifecycle token; the supervisor is woken by that
    // cancellation and joins afterwards. Do not "optimise" this into cancelling
    // the drainer with the rest of the supervised set at the top of teardown:
    // `flush` early-returns on a stopped drainer, so that inversion makes the
    // flush an instant no-op and drops the pod's terminal lifecycle events with
    // no error anywhere. See the comment on that flush in `shutdown.rs`.
    let (outbox, outbox_drainer) = moatless_sandbox::outbox::EventOutbox::spawn(
        Arc::clone(&backend_client),
        config.task_id.clone(),
        lifecycle_cancel.clone(),
    );
    registrar.supervise(
        "event-outbox-drainer",
        Criticality::Fatal,
        outbox_drainer.into_join_handle(),
    );
    let event_publisher = GrpcEventPublisher::new(outbox);

    // Build shared state with init phase tracking
    let agent_state = Arc::new(agent::agent_state::AgentState::new());
    let config_arc = Arc::new(config.clone());
    let credentials = Arc::new(RwLock::new(std::collections::HashMap::new()));

    // Build the scheduled-job registry that owns Monitor crons and
    // ScheduleWakeup timers. The injector is late-bound: we need an
    // `AgentHandle` to inject messages, but the lifecycle task (which produces the
    // handle) needs the registry up front. The `LateInjector` breaks the
    // cycle — we construct the registry around it, spawn the lifecycle task with the
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

    // Liveness heartbeat: the coordinator service bumps it, the `/livez` probe reads it.
    let heartbeat = moatless_sandbox::agent::heartbeat::Heartbeat::new();

    // One workspace-upload baseline for the pod, shared by the coordinator's
    // turn-boundary uploads and every path that reaches it through SandboxState.
    let workspace_sync = moatless_sandbox::init::workspace_sync::WorkspaceSyncTracker::new();

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
        heartbeat.clone(),
        workspace_sync.clone(),
    );
    late_injector.set(Box::new(agent_handle.clone()));

    let state = SandboxState {
        config: config_arc,
        events: event_publisher,
        agent: agent_state,
        agent_handle,
        heartbeat,
        scheduled_jobs,
        backend_client,
        init_phase: Arc::new(RwLock::new(InitPhase::Starting)),
        credentials,
        lifecycle_cancel,
        credential_dispatcher,
        credential_failures,
        codex_watcher_handle,
        system_prompt_cache,
        dispatch_lock: Default::default(),
        supervisor_health: supervisor.health(),
        terminals: moatless_sandbox::terminal::TerminalManager::new(config.workspace_dir.clone()),
        workspace_sync,
    };

    // The agent lifecycle task is deliberately NOT registered with the root supervisor.
    // It is the one long-lived task whose termination already reaches a probe:
    // when the lifecycle task exits or panics, `command_rx` drops and
    // `agent_handle.is_closed()` makes `/livez` report 503 — the same outcome a
    // `Criticality::Liveness` registration would produce. It also does not
    // observe the lifecycle token (it exits when the last `AgentHandle` drops,
    // i.e. at process teardown), so registering it would put a known-permanent
    // entry in every shutdown's straggler report and make that report useless.

    // Start gRPC server for sandbox service (receives commands from backend)
    // Available even during init so backend can reach the pod. Drains via the
    // lifecycle token so a SIGTERM (see shutdown driver below) stops it cleanly.
    //
    // Fatal: this is the only channel the backend has to reach the pod. Before
    // supervision an early `serve` return logged an error and left a process
    // nothing could talk to, still passing its startup probe forever.
    let grpc_state = state.clone();
    let grpc_port = config.grpc_port;
    let grpc_server = tokio::spawn(async move {
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
    registrar.supervise("grpc-server", Criticality::Fatal, grpc_server);

    // Spawn the credential broker on a unix socket. Git is configured to use
    // a helper script that hits this socket on every push/fetch, so tokens
    // are pulled fresh from the backend instead of relying on hosts.yml staying
    // current between 15-min refresh cycles.
    //
    // Spawned BEFORE the init task so the socket exists by the time
    // `init/git.rs` wires the helper into git config.
    let broker_socket =
        std::path::Path::new(&state.config.runtime_dir).join(CREDENTIAL_BROKER_SOCKET_FILE);
    let broker_state = BrokerState {
        task_id: state.config.task_id.clone(),
        backend_client: Arc::clone(&state.backend_client),
    };
    // Optional: losing the broker degrades git auth to whatever
    // `~/.config/gh/hosts.yml` last held — the same fallback a failed spawn
    // already accepts below.
    match broker::spawn(broker_state, &broker_socket, state.lifecycle_cancel.clone()) {
        Ok(broker_task) => {
            registrar.supervise("credential-broker", Criticality::Optional, broker_task);
        }
        Err(e) => tracing::warn!(
            error = %e,
            socket = %broker_socket.display(),
            "Failed to spawn credential broker — git operations will fall back to hosts.yml"
        ),
    }

    // Spawn init task — handles credential fetching, git clone, workspace setup.
    // After init completes, it triggers process_pending_messages() to pick up
    // any messages that arrived during initialization.
    //
    // Finite + Fatal: `run_init` returning is the normal end of startup, but a
    // panic inside it leaves a pod that never reaches Ready and never says why —
    // before supervision the `JoinError` was discarded with the handle. Core-init
    // *errors* still fail fast through `run_init`'s own `process::exit`.
    let init_state = state.clone();
    let init_task = tokio::spawn(moatless_sandbox::init::run_init(
        init_state,
        registrar.clone(),
    ));
    registrar.supervise_finite("init", Criticality::Fatal, init_task);
    info!("Init task spawned");

    // Build HTTP router with health probe that reflects init state
    let health_state = state.clone();
    let router = Router::new()
        .route("/health", get(health))
        .route("/livez", get(livez))
        .with_state(health_state);

    // Drive graceful shutdown on SIGTERM/SIGINT. Without this the process rides
    // the full pod termination grace period and is SIGKILLed (exit 137) on every
    // teardown (idle-down, node drain, eviction, PR-close). On signal we stop the
    // agent and cancel the lifecycle token, which both servers and background
    // tasks observe to wind down.
    //
    // Liveness, not Fatal: a driver that died before the signal means the *next*
    // teardown rides the grace period to exit-137, which costs the final flushes.
    // Exiting the process now to fix that would destroy the turn currently
    // running, which is strictly worse — a restart under SIGTERM is the remedy.
    // Its own normal return happens after it cancels the lifecycle token, so it
    // classifies as shutdown rather than early termination.
    registrar.supervise(
        "shutdown-driver",
        Criticality::Liveness,
        shutdown::spawn_shutdown_driver(state.clone()),
    );

    // Start HTTP server (K8s health probes only). The bind still fails `main`
    // directly — there is nothing to supervise if the socket never opened.
    //
    // Fatal: an early `serve` return leaves the probes unanswerable. The HTTP
    // graceful-shutdown wait sits inside the supervisor's bounded join.
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    info!(
        port = config.port,
        "Sandbox HTTP server started (health only)"
    );

    let http_shutdown = state.lifecycle_cancel.clone();
    let http_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async move { http_shutdown.cancelled().await })
            .await
        {
            tracing::error!(error = %e, "Sandbox HTTP server failed");
        }
        info!("Sandbox HTTP server stopped");
    });
    registrar.supervise("http-server", Criticality::Fatal, http_server);

    // Hand control to the supervisor: it watches until a fatal exit or the
    // lifecycle token fires, then cancels and joins everything within one
    // bounded deadline (see `SHUTDOWN_JOIN_DEADLINE`).
    //
    // The second argument is the graceful teardown for the *fatal* path, where
    // no SIGTERM arrived and so the shutdown driver never ran. It is the same
    // `perform_shutdown` the signal path uses, run in the same position — before
    // cancellation — so the outbox flush, the transcript flush and the workspace
    // flush all happen on both exit paths. `Fatal` is a per-task policy, not a
    // synonym for "the backend is unreachable": for every Fatal task except the
    // gRPC server the connection is fine, and a fatal exit is exactly when a
    // terminal lifecycle event most needs to land. The future is not polled
    // unless a fatal exit actually happens.
    let teardown = shutdown::perform_shutdown(&state.agent, &state.lifecycle_cancel, Some(&state));
    let outcome = supervisor.run(SHUTDOWN_JOIN_DEADLINE, teardown).await;
    match &outcome {
        Ok(()) => info!("Sandbox exiting cleanly"),
        Err(exit) => {
            tracing::error!(error = %exit, "Sandbox exiting after a critical task failure");
        }
    }
    outcome?;
    Ok(())
}

/// Run a one-shot job instead of the sandbox runtime.
///
/// `SANDBOX_ONESHOT=sync` is the workspace rescue: the backend could not prove
/// the S3 archive current before reclaiming a stopped task's volume, so it runs
/// this against the volume with a presigned `PUT` in `WORKSPACE_SYNC_PUT_URL`.
/// Nothing else about a sandbox is started — no gRPC server, no agent, no
/// credentials, no backend client — because none of it exists to talk to.
///
/// The exit status is the whole protocol: the caller deletes the volume when
/// this process succeeds and keeps it when it does not, so every failure must
/// be an error return and never a warning.
async fn run_oneshot(mode: &str, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    match mode {
        "sync" => {
            let put_url = std::env::var("WORKSPACE_SYNC_PUT_URL")
                .map_err(|_| "SANDBOX_ONESHOT=sync requires WORKSPACE_SYNC_PUT_URL")?;
            info!(
                task_id = %config.task_id,
                workspace_dir = %config.workspace_dir,
                "Rescuing workspace to S3"
            );
            let bytes = moatless_sandbox::init::workspace_sync::rescue_upload(
                &config.workspace_dir,
                &put_url,
            )
            .await?;
            info!(task_id = %config.task_id, bytes, "Workspace rescued");
            Ok(())
        }
        other => Err(format!("unknown SANDBOX_ONESHOT mode: {other}").into()),
    }
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

/// GET /livez — honest liveness signal for the pod's own machinery.
///
/// Returns 200 while the pod can still make progress and 503 once it cannot, so
/// K8s can restart a hung-but-running pod that `/health` (a startup probe) would
/// keep reporting healthy. Two ways to lose it: a supervised task terminated
/// early, or the agent lifecycle task wedged or exited. Still depends on neither backend
/// reachability, credentials, nor disk, so a transient dependency outage can't
/// trigger a restart that kills an in-flight turn.
///
/// The decision itself is [`SandboxState::is_live`] — in the library, where it
/// is testable.
async fn livez(State(state): State<SandboxState>) -> impl IntoResponse {
    if state.is_live().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
