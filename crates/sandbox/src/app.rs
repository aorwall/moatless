use crate::code_intel;
use crate::config::Config;
use crate::credentials::broker::{self, BrokerState};
use crate::credentials::{gh_config, github_proxy, service_relay};
use crate::grpc_server::SandboxServiceImpl;
use crate::state::SandboxState;
use crate::supervisor::{Criticality, SHUTDOWN_JOIN_DEADLINE};
use crate::{preview_proxy, shutdown};
use moatless_types::credential_key::SERVICE_RELAY_ADDR;
use moatless_types::sandbox::SANDBOX_PREVIEW_PROXY_PORT;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

const CREDENTIAL_BROKER_SOCKET_FILE: &str = "credentials.sock";

/// `gh`'s socket in `RUNTIME_DIR`. `moat gh` looks for it under the same name.
const GITHUB_PROXY_SOCKET_FILE: &str = "github.sock";

/// Configure structured logging from the process environment.
pub fn initialize_logging(log_level: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            // `rustic_core` WARNs once per restored entry whose extended
            // attributes it cannot apply, and the workspace volume accepts
            // none of them: 2.7M expected lines a day across staging, with no
            // knob on `RestoreOptions` to turn them off. `RUST_LOG` overrides
            // this whole filter and brings them back.
            EnvFilter::new(format!("{log_level},rustic_core::commands::restore=error"))
        }))
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
}

/// Assemble and supervise the sandbox process.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env().map_err(|error| {
        tracing::error!(%error, "Invalid checkout configuration");
        crate::init::InitError::Permanent(error)
    })?;

    info!(
        task_id = %config.task_id,
        port = config.port,
        grpc_port = config.grpc_port,
        "Starting Moatless Sandbox"
    );

    let (state, supervisor) = crate::compose::compose_runtime(&config).await;
    let registrar = supervisor.registrar();

    // Start gRPC server for sandbox service (receives commands from backend)
    // Available even during init so backend can reach the pod. Drains via
    // the lifecycle token so SIGTERM stops it cleanly. Fatal: this is the
    // only channel the backend has to reach the pod — before supervision,
    // an early `serve` return left a process nothing could talk to.
    let svc = SandboxServiceImpl::new(state.clone());
    let code_intel = svc.code_intel().clone();
    let grpc_port = config.grpc_port;
    let grpc_shutdown = state.lifecycle_cancel.clone();
    let verify_calls =
        sandbox_proto::auth::VerifyCalls::new(config.grpc_key.clone(), config.grpc_auth_enforce);
    let grpc_server = tokio::spawn(async move {
        let addr = SocketAddr::from(([0, 0, 0, 0], grpc_port));
        info!(port = grpc_port, "Starting sandbox gRPC server");
        let limit = sandbox_proto::client::MAX_GRPC_MESSAGE_SIZE;
        let service = sandbox_proto::sandbox_service_server::SandboxServiceServer::new(svc)
            .max_decoding_message_size(limit)
            .max_encoding_message_size(limit);
        let service = tonic::service::interceptor::InterceptedService::new(service, verify_calls);
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
    // a helper script that hits this socket on every push/fetch, so it is
    // handed the task token current at that moment.
    //
    // Spawned BEFORE the init task so the socket exists by the time
    // `init/git.rs` wires the helper into git config.
    let broker_socket =
        std::path::Path::new(&state.config.runtime_dir).join(CREDENTIAL_BROKER_SOCKET_FILE);
    let broker_state = BrokerState {
        backend_client: Arc::clone(&state.backend_client),
        backend_host: state.config.backend_api_host(),
    };
    // Optional: losing the broker leaves git with no credential, which fails
    // a push loudly rather than the pod.
    match broker::spawn(broker_state, &broker_socket, state.lifecycle_cancel.clone()) {
        Ok(broker_task) => {
            registrar.supervise("credential-broker", Criticality::Optional, broker_task);
        }
        Err(e) => tracing::warn!(
            error = %e,
            socket = %broker_socket.display(),
            "Failed to spawn credential broker — git has no credential source"
        ),
    }

    spawn_github_proxy(&state, &registrar).await;
    spawn_service_relay(&state, &registrar).await;

    // Before init, so the sockets exist by the time the agent's first `LSP`
    // call starts `moat lsp-proxy`. Optional: without them the agent loses
    // its `LSP` tool, and the UI's lookups are unaffected.
    let runtime_dir = std::path::Path::new(&state.config.runtime_dir);
    match code_intel::spawn_shared(&code_intel, runtime_dir, state.lifecycle_cancel.clone()) {
        Ok(shared) => registrar.supervise("language-server-sockets", Criticality::Optional, shared),
        Err(e) => tracing::warn!(error = %e, "Could not share language servers with the agent"),
    }

    // Optional: without it an OOM kill still happens, it just names no process.
    if let Some(watch) = crate::memory_watch::spawn(state.lifecycle_cancel.clone()) {
        registrar.supervise("memory-watch", Criticality::Optional, watch);
    }

    // Spawns init: credential fetch, git clone, workspace setup, then
    // `process_pending_messages()` for anything that arrived meanwhile.
    //
    // Init errors keep their permanent/transient classification through
    // the exit decision; panics remain supervisor failures.
    let init_state = state.clone();
    let (init_failure, init_errors) = tokio::sync::oneshot::channel();
    let init_registrar = registrar.clone();
    let init_task = tokio::spawn(async move {
        let workspace_dir = init_state.config.workspace_dir.clone();
        match run_init(init_state, init_registrar).await {
            Ok(()) => code_intel.warm(&workspace_dir),
            Err(error) => {
                let _ = init_failure.send(error);
            }
        }
    });
    registrar.supervise_finite("init", Criticality::Fatal, init_task);
    info!("Init task spawned");

    let router = crate::health::router(state.clone());

    // Drive graceful shutdown on SIGTERM/SIGINT: without it, every teardown
    // rides the grace period to a SIGKILL (exit 137). On signal it stops the
    // agent and cancels the lifecycle token, which servers and background
    // tasks observe. Liveness, not Fatal: dying before the signal just costs
    // the next teardown's final flushes, and isn't worth a restart.
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

    // The preview proxy answers for every published port (see
    // `preview_proxy`). Optional, and a bind failure is a warning rather than a
    // `main` error: a task that publishes no port never reaches this listener,
    // and taking the whole sandbox down would cost the agent its turn.
    let proxy_addr = SocketAddr::from(([0, 0, 0, 0], SANDBOX_PREVIEW_PROXY_PORT));
    match TcpListener::bind(proxy_addr).await {
        Ok(proxy_listener) => {
            let proxy = tokio::spawn(preview_proxy::serve(
                proxy_listener,
                config.grpc_port,
                state.lifecycle_cancel.clone(),
            ));
            registrar.supervise("preview-proxy", Criticality::Optional, proxy);
        }
        Err(e) => tracing::warn!(
            error = %e,
            port = SANDBOX_PREVIEW_PROXY_PORT,
            "Preview proxy could not bind; published ports will not be reachable"
        ),
    }

    // The supervisor watches until a fatal exit or the lifecycle token
    // fires, then cancels and joins within one deadline. The second arg is
    // the fatal-path teardown (no SIGTERM, shutdown driver never ran) —
    // the same `perform_shutdown`, run before cancellation so flushes land
    // on both exit paths; never polled unless a fatal exit happens.
    let teardown = shutdown::perform_shutdown_for_state(&state);
    let supervision = supervisor.run(SHUTDOWN_JOIN_DEADLINE, teardown);
    tokio::pin!(supervision);
    let outcome = tokio::select! {
        outcome = &mut supervision => outcome,
        failure = init_errors => match failure {
            Ok(error) => return Err(Box::new(error)),
            Err(_) => supervision.await,
        },
    };
    match &outcome {
        Ok(()) => info!("Sandbox exiting cleanly"),
        Err(exit) => {
            tracing::error!(error = %exit, "Sandbox exiting after a critical task failure");
        }
    }
    outcome?;
    Ok(())
}

/// Serve the service relay a pod's tools reach the backend's service proxies
/// through. Optional: without it only those tools fail, each with a refused
/// connection.
async fn spawn_service_relay(state: &SandboxState, registrar: &crate::supervisor::Registrar) {
    let relay = service_relay::ServiceRelayState::new(
        Arc::clone(&state.backend_client),
        state.config.backend_api_url.as_deref(),
        state.config.http.github,
    );
    match service_relay::serve(relay, SERVICE_RELAY_ADDR, state.lifecycle_cancel.clone()).await {
        Ok(task) => registrar.supervise("service-relay", Criticality::Optional, task),
        Err(e) => {
            tracing::warn!(error = %e, "Failed to spawn the service relay — service proxies are unreachable");
        }
    }
}

/// Serve `gh`'s socket and point `gh` at it, before init runs `gh api user`.
///
/// Optional: without it `gh` finds no token and asks for `gh auth login`,
/// while git and the agent carry on.
async fn spawn_github_proxy(state: &SandboxState, registrar: &crate::supervisor::Registrar) {
    let socket = std::path::Path::new(&state.config.runtime_dir).join(GITHUB_PROXY_SOCKET_FILE);
    let proxy = github_proxy::GitHubProxyState::new(
        Arc::clone(&state.backend_client),
        state.config.backend_api_url.as_deref(),
        state.config.http.github,
    );
    match broker::serve_on_socket(
        github_proxy::router(proxy),
        &socket,
        state.lifecycle_cancel.clone(),
        "GitHub proxy",
    ) {
        Ok(task) => registrar.supervise("github-proxy", Criticality::Optional, task),
        Err(e) => {
            tracing::warn!(error = %e, socket = %socket.display(), "Failed to spawn the GitHub proxy — gh is unauthenticated");
            return;
        }
    }
    if let Err(e) = gh_config::route_gh_through(&state.config.home_dir, &socket).await {
        tracing::warn!(error = %e, "Failed to point gh at the GitHub proxy — gh is unauthenticated");
    }
}

/// Initialize the workspace, then register the runtime loops it enables.
pub async fn run_init(
    state: SandboxState,
    registrar: crate::supervisor::Registrar,
) -> Result<(), crate::init::InitError> {
    if let Err(error) = crate::init::initialize(state.clone()).await {
        tracing::error!(%error, "Core initialization failed");
        let _ = std::fs::write("/dev/termination-log", error.to_string());
        return Err(error);
    }
    start_background_tasks(&state, &registrar).await;
    Ok(())
}

async fn start_background_tasks(state: &SandboxState, registrar: &crate::supervisor::Registrar) {
    // Credential refresher: GitHub tokens expire (1h app, 8h user), and a
    // long-running task would fail mid-turn on `git push`/`gh pr comment`
    // without it (see `refresher.rs`). Liveness: only a panic ends the loop
    // (it handles its own transient errors), after which credentials go
    // stale silently; a restart re-fetches rather than killing the turn.
    let refresher_state = state.clone();
    let refresher_cancel = state.lifecycle_cancel.clone();
    let refresher_interval = state.config.credential_refresh_interval;
    registrar.supervise(
        "credential-refresher",
        Criticality::Liveness,
        tokio::spawn(crate::credentials::refresher::run_credential_refresher(
            refresher_state,
            refresher_cancel,
            refresher_interval,
        )),
    );

    // Session-state uploader: tars Claude/Codex session state to S3 every
    // interval, the only CLI-session durability on the ephemeral tier
    // (`TRANSCRIPT_SYNC_INTERVAL_SECS=0` disables it; shutdown still flushes
    // the final delta). Liveness: a dead loop silently stops persisting
    // resume state; the shutdown-driven final flush is unaffected.
    if let Some(interval) = state.config.transcript_sync_interval {
        let transcript_state = state.clone();
        let transcript_cancel = state.lifecycle_cancel.clone();
        registrar.supervise(
            "transcript-sync",
            Criticality::Liveness,
            tokio::spawn(crate::transcript_sync::run_transcript_sync(
                transcript_state,
                transcript_cancel,
                interval,
            )),
        );
    }

    // Workspace uploader (`s3` backend): bounds the gap a turn killed
    // involuntarily (eviction, OOM, spot reclaim) leaves beyond turn-end
    // and shutdown flushes, uploading only when the tracked file set
    // changed (`WORKSPACE_SYNC_INTERVAL_SECS=0`, or a durable volume,
    // disables it). Liveness: a dead loop just drops between-turn checkpoints.
    if crate::workspace_sync::is_periodic_enabled(&state.config) {
        if let Some(interval) = state.config.workspace_sync_interval {
            let workspace_state = state.clone();
            let workspace_cancel = state.lifecycle_cancel.clone();
            registrar.supervise(
                "workspace-sync",
                Criticality::Liveness,
                tokio::spawn(crate::workspace_sync::run_workspace_sync(
                    workspace_state,
                    workspace_cancel,
                    interval,
                )),
            );
        }
    }
}
