use crate::config::Config;
use crate::credentials::broker::{self, BrokerState};
use crate::grpc_server::SandboxServiceImpl;
use crate::state::SandboxState;
use crate::supervisor::{Criticality, SHUTDOWN_JOIN_DEADLINE};
use crate::{preview_proxy, shutdown};
use moatless_types::sandbox::SANDBOX_PREVIEW_PROXY_PORT;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

const CREDENTIAL_BROKER_SOCKET_FILE: &str = "credentials.sock";

/// Configure structured logging from the process environment.
pub fn initialize_logging() {
    // Read the log level directly rather than through `Config::from_env()`:
    // that call panics if `BACKEND_GRPC_URL` is unset, and a one-shot mode
    // carries no such thing (see below), so it must not be built yet.
    let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            // `rustic_core` WARNs once per restored entry whose extended
            // attributes it cannot apply, and the workspace volume accepts
            // none of them: 2.7M expected lines a day across staging, with no
            // knob on `RestoreOptions` to turn them off. `RUST_LOG` overrides
            // this whole filter and brings them back.
            EnvFilter::new(format!(
                "{log_level},rustic_core::commands::restore=error"
            ))
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
    // Load configuration
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
        backend_host: state.config.backend_api_host(),
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
    // Initialization errors retain their permanent/transient classification
    // through the executable's exit decision; panics remain supervisor failures.
    let init_state = state.clone();
    let (init_failure, init_errors) = tokio::sync::oneshot::channel();
    let init_registrar = registrar.clone();
    let init_task = tokio::spawn(async move {
        if let Err(error) = run_init(init_state, init_registrar).await {
            let _ = init_failure.send(error);
        }
    });
    registrar.supervise_finite("init", Criticality::Fatal, init_task);
    info!("Init task spawned");

    let router = crate::health::router(state.clone());

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
    // Spawn the periodic credential refresher. GitHub App installation tokens
    // expire after 1h and user access tokens after 8h — long-running tasks
    // would otherwise fail mid-turn on `git push` / `gh pr comment`.
    // See `refresher.rs` for the full rationale.
    //
    // Liveness: the loop handles its own transient errors, so only a panic ends
    // it — after which credentials silently go stale and git operations start
    // failing an hour later with no connection to the cause. A restart re-fetches
    // them; an immediate process exit would kill the turn that is still working.
    let refresher_state = state.clone();
    let refresher_cancel = state.lifecycle_cancel.clone();
    let refresher_interval = crate::credentials::refresher::read_interval_from_env();
    registrar.supervise(
        "credential-refresher",
        Criticality::Liveness,
        tokio::spawn(crate::credentials::refresher::run_credential_refresher(
            refresher_state,
            refresher_cancel,
            refresher_interval,
        )),
    );

    // Spawn the periodic CLI session-state uploader. Every interval it tars
    // Claude transcripts plus Codex rollout sessions and ships them to S3 via
    // the backend, so agent resume state persists off the per-task PVC (and is
    // the only CLI-session durability for the ephemeral tier).
    // `TRANSCRIPT_SYNC_INTERVAL_SECS=0` disables the loop; sync-on-shutdown
    // still flushes the final delta.
    //
    // Liveness: a dead loop stops persisting resume state, so an ungraceful kill
    // loses everything since the last successful sync — silently, because
    // `flush_on_shutdown` is driven by the shutdown driver rather than by this
    // task and keeps working either way. That final flush is unchanged;
    // supervision only makes the loop's death visible.
    if let Some(interval) = crate::transcript_sync::read_interval_from_env() {
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

    // Spawn the periodic workspace uploader for the `s3` storage backend. Turn
    // completion and the bounded shutdown flush are the primary durability
    // points, but a long turn killed involuntarily (node-pressure eviction, OOM,
    // spot reclaim with no grace) reaches neither and loses everything since the
    // last completed turn. This loop bounds that gap: every interval it
    // fingerprints the workspace and uploads only when the tracked file set
    // changed. `WORKSPACE_SYNC_INTERVAL_SECS=0` disables it, and so does a
    // durable volume, which survives the kill this loop exists for;
    // turn-boundary, pre-stop and shutdown uploads still run.
    //
    // Liveness: a dead loop silently drops the between-turn checkpoints, leaving
    // an ungraceful kill to fall back to the last turn boundary — supervision
    // makes the loop's death visible rather than fixing it.
    if crate::workspace_sync::is_periodic_enabled(&state.config) {
        if let Some(interval) = crate::workspace_sync::read_interval_from_env() {
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

    // Watch ~/.codex/auth.json for rotations. Codex rewrites this file every
    // time it refreshes its OAuth token; the watcher pushes the rotated blob
    // back to the backend so subsequent tasks (and pod restarts) see the new
    // refresh_token instead of the now-invalidated stored copy.
    //
    // A routed Codex authenticates by `env_key` and rotates nothing. Running
    // the watcher would push the synthesized proxy-key blob into the user's
    // stored Codex credential and destroy the OAuth token it replaced.
    if state.config.openai_base_url.is_none() {
        spawn_codex_auth_watcher(state, registrar).await;
    }

    // Watch each checkout's remote reflogs so a sha this task pushes is claimed
    // before GitHub's `pull_request.synchronize` webhook for it arrives.
    spawn_git_push_watcher(state, registrar);
}

fn spawn_git_push_watcher(state: &SandboxState, registrar: &crate::supervisor::Registrar) {
    let watcher_task = crate::git_push_watcher::spawn(
        state.config.task_id.clone(),
        state.config.repo_dirs(),
        std::sync::Arc::clone(&state.backend_client),
        state.lifecycle_cancel.clone(),
    );

    // Optional: losing the watcher costs the task its own push notifications
    // back in its chat. It costs the running turn nothing else, so it is not
    // worth a restart — but it should be named rather than logged as an
    // anonymous panic.
    registrar.supervise("git-push-watcher", Criticality::Optional, watcher_task);
}

async fn spawn_codex_auth_watcher(state: &SandboxState, registrar: &crate::supervisor::Registrar) {
    let auth_path = std::path::PathBuf::from(format!("{}/auth.json", state.config.codex_home_dir));

    // Capture the current on-disk value as the baseline so the first rotation
    // (which produces a different blob) triggers the sync. If the file is
    // absent — Codex isn't configured for this task — the watcher still runs
    // and will pick up the file once it appears.
    let initial_value = tokio::fs::read_to_string(&auth_path)
        .await
        .unwrap_or_default();

    let (handle, watcher_task) = crate::credentials::codex_watcher::spawn(
        state.config.task_id.clone(),
        auth_path,
        initial_value,
        std::sync::Arc::clone(&state.backend_client),
        state.lifecycle_cancel.clone(),
    );

    // Optional: losing the watcher means a Codex token rotation is not pushed
    // back to the backend, which costs the *next* task (or the next pod start) a
    // stale refresh_token. It costs the running turn nothing, so it is not worth
    // a restart — but it should be named rather than logged as an anonymous
    // panic.
    registrar.supervise("codex-auth-watcher", Criticality::Optional, watcher_task);

    // Stash the handle so the credential dispatcher can seed the watcher's
    // cache when a backend-driven push lands — keeps backend writes from
    // echoing back as fresh `UpdateCredential` calls.
    *state.codex_watcher_handle.write().await = Some(handle);
}
