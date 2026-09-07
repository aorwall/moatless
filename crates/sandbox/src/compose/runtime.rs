use crate::backend_client::BackendGrpcClient;
use crate::config::Config;
use crate::credentials::Dispatcher;
use crate::credentials::sinks::{ClaudeOauthSink, CodexAuthJsonSink, GitTokenSink};
use crate::events::GrpcEventPublisher;
use crate::init::InitPhase;
use crate::state::SandboxState;
use crate::supervisor::{Criticality, Supervisor};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub(crate) async fn compose_runtime(config: &Config) -> (SandboxState, Supervisor) {
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
    let backend_client: Arc<dyn crate::backend_client::BackendClient> = Arc::new(backend_client);

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
    let (outbox, outbox_drainer) = crate::outbox::EventOutbox::spawn(
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

    let config_arc = Arc::new(config.clone());
    let credentials = Arc::new(RwLock::new(std::collections::HashMap::new()));

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

    // Liveness heartbeat: the coordinator service bumps it, the `/livez` probe reads it.
    let heartbeat = crate::agent::heartbeat::Heartbeat::new();

    // One workspace-upload baseline for the pod, shared by the coordinator's
    // turn-boundary uploads and every path that reaches it through SandboxState.
    let workspace_sync = crate::workspace_sync::WorkspaceSyncTracker::new();

    let task_contexts = Arc::new(RwLock::new(std::collections::HashMap::new()));
    let task_context_registration = Arc::new(tokio::sync::Mutex::new(()));

    let state = SandboxState {
        config: config_arc,
        events: event_publisher,
        task_contexts,
        task_context_registration,
        heartbeat,
        backend_client,
        init_phase: Arc::new(RwLock::new(InitPhase::Starting)),
        credentials,
        lifecycle_cancel,
        credential_dispatcher,
        codex_watcher_handle,
        supervisor_health: supervisor.health(),
        terminals: crate::terminal::TerminalManager::with_repo_dir(
            config.workspace_dir.clone(),
            config.repo_dir.clone(),
        ),
        workspace_sync,
    };

    let context = super::compose_task_context(
        Arc::clone(&state.config),
        crate::state::CheckoutMode::Shared,
        &state,
    )
    .await;
    state
        .task_contexts
        .write()
        .await
        .insert(context.task_id.clone(), context);
    (state, supervisor)
}
