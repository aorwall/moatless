use crate::backend_client::BackendGrpcClient;
use crate::config::Config;
use crate::credentials::Dispatcher;
use crate::credentials::sinks::UserAuthTokenSink;
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

    let backend_client: Arc<dyn crate::backend_client::BackendClient> = Arc::new(backend_client);

    // Lifecycle token, created up front so the event outbox drainer shares it
    // with the rest of the runtime and winds down on SIGTERM.
    let lifecycle_cancel = CancellationToken::new();

    // Root supervisor: owns the join handle of every long-lived task below under
    // a policy declared at registration. `registrar` is cloned into anything that
    // spawns supervised work later (the init task's durability loops).
    let supervisor = Supervisor::new(lifecycle_cancel.clone());
    let registrar = supervisor.registrar();

    // At-least-once send buffer backing the publisher (drainer retries
    // across rollout/network interruptions). Fatal: once stopped,
    // `enqueue` refuses admission for good. Do NOT cancel it with the rest
    // of the supervised set: `perform_shutdown` must flush before
    // cancelling, or `flush` no-ops and silently drops terminal events.
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

    // The one credential the backend pushes: a pod holds no model credential.
    let mut dispatcher = Dispatcher::new();
    let git_credentials = Arc::new(RwLock::new(Default::default()));
    dispatcher.register(
        moatless_types::credential_key::USER_AUTH_TOKEN,
        Arc::new(UserAuthTokenSink {
            config: Arc::clone(&config_arc),
            backend_client: Arc::clone(&backend_client),
            git_credentials: Arc::clone(&git_credentials),
        }),
    );
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
        setup_gate: crate::init::SetupGate::closed(),
        workspace_origin: Default::default(),
        stop_cause: Default::default(),
        git_credentials,
        credentials,
        lifecycle_cancel,
        credential_dispatcher,
        supervisor_health: supervisor.health(),
        terminals: crate::terminal::TerminalManager::with_repo_dir(
            config.workspace_dir.clone(),
            config.repo_dir.clone(),
        )
        .with_shell(&config.shell),
        workspace_sync,
        writer_admission: Default::default(),
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
