//! Backend process lifecycle and coordinated shutdown.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::adapters::events::TaskEventSubscribers;
use crate::compose::{Ports, compose};
use crate::event_handler;
use crate::sandbox;
use crate::startup;
use crate::state::AppState;
use crate::supervisor::{Criticality, Supervisor};
use crate::t3code;
use moatless_core::config::Config;
use moatless_identity::auth;
use moatless_tasks::agent_event::EventKey;
use moatless_tasks::task_listing;
use moatless_types::sandbox::SandboxStatus;

const SHUTDOWN_JOIN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

/// Run the backend process and supervise its servers and workers.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    #[allow(clippy::expect_used)]
    // fail-fast: TLS provider must be installed before any networking
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Read directly (not from `Config::from_env`, called next) so tracing is
    // live before that call's own parse warnings need somewhere to land.
    let otel_config = moatless_core::config::OtelConfig::from_env();
    let otel_provider = startup::init_tracing(&otel_config);

    let config = Config::from_env()?;
    moatless_sandbox_control::secrets::check_service_proxy_keys(&config.service_proxies.services)?;
    info!(sandbox_namespace = %config.sandbox.namespace, "Starting Moatless Backend");

    // Root cancellation + the one supervisor that owns every long-lived task's
    // join handle. Each `supervise` call declares what this process does when
    // that task stops (see `supervisor::Criticality`).
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());

    // Infrastructure services
    supervisor.supervise(
        "shutdown-signal",
        // Optional: this listener's whole job is to cancel the root token, and
        // by the time it returns it already has.
        Criticality::Optional,
        startup::install_shutdown_handler(cancel.clone()),
    );
    supervisor.supervise(
        "metrics-server",
        // Fatal: a metrics port that never binds leaves a pod that looks
        // healthy but exports nothing.
        Criticality::Fatal,
        startup::spawn_metrics_server(config.server.metrics_port, cancel.clone()),
    );
    let db_pool = crate::compose::infrastructure::init_database(
        config.database_url.as_deref(),
        config.database.max_connections,
        config.database.run_migrations,
    )
    .await?;
    let config = match crate::compose::adapter_config::load_adapter_app_config(&config, &db_pool)
        .await
    {
        Ok(config) => config,
        Err(e) => {
            warn!(
                error = %e,
                "Failed to load Adapter app config from the DB; Slack and Linear stay unconfigured"
            );
            config
        }
    };
    let feature_flags = config.feature_flags;
    // Set only if `ui_rpc` below mounts the T3 compatibility routes; reported
    // as-is in the feature-flags response.
    let mut device_pairing = false;
    // The composition root: every port this process has, built once from the
    // pool and the configuration it just resolved. Everything downstream
    // receives ports from here rather than building its own.
    let s3_client = crate::compose::infrastructure::init_s3(&config).await;
    let ports: Ports = compose(db_pool.clone(), &config, s3_client.clone());
    // A shadow backend (`run_migrations` off) shares the real deployment's
    // database but not its Helm values, so it would disable every declared
    // plugin there.
    if config.database.run_migrations {
        ports
            .plugins
            .reconcile_declared(&config.declared_plugins)
            .await;
    }
    let event_publisher = crate::compose::infrastructure::init_event_publisher(&db_pool);
    let (sse_state, sse_feed) = startup::init_sse(
        &config,
        db_pool.clone(),
        ports.task_reader.clone() as Arc<dyn moatless_ports::task::TaskReader>,
        cancel.clone(),
    );
    supervisor.supervise(
        "sse-event-feed",
        // Readiness: without it the live SSE tail silently stops, so this
        // replica must stop receiving traffic.
        Criticality::Readiness,
        sse_feed,
    );
    // Assumes the workspace-repository role so a sandbox can reach its own
    // repository prefix directly. `None` when no role is configured.
    let sts_client = crate::compose::infrastructure::init_sts(&config).await;
    // Mints per-task access to a workspace repository, for the sandbox over
    // gRPC and for an administrator over HTTP.
    let workspace_repository =
        crate::sandbox::workspace_repository::WorkspaceRepositoryAccess::new(
            sts_client.clone(),
            config.sandbox.workspace_repository_role_arn.clone(),
            config.sandbox.workspace_repository_secret.clone(),
            config.s3.bucket_name.clone(),
            config.s3.region.clone(),
            config.s3.workspace_prefix.clone(),
        )
        .map(std::sync::Arc::new);
    // The archive reader behind the cold transition. Present only when S3 is
    // configured — with no durable copy there is nothing to reclaim toward, and
    // an absent port disables the transition rather than risking a volume.
    let sandbox_dependencies =
        crate::compose::sandbox::build(&config, &ports, s3_client.clone()).await?;

    supervisor.supervise(
        "grpc-server",
        // Fatal: every sandbox reaches the backend through this server.
        Criticality::Fatal,
        startup::spawn_grpc_server(
            config.server.grpc_port,
            crate::compose::infrastructure::grpc_service(
                &config,
                &ports,
                event_publisher.clone(),
                s3_client.clone().map(std::sync::Arc::new),
                workspace_repository.clone(),
                sandbox_dependencies.sandbox_connector.clone(),
            ),
            cancel.clone(),
        ),
    );

    // ── Build router ────────────────────────────────────────────────────

    // Sandbox route state keeps only authorization DAOs.

    // Build the shared durable Sandbox lifecycle handle once (requires the DB).
    // The sandbox API, the pod watcher, and AppState all drive transitions
    // through this single instance, so Sandbox intent and per-Task locks stay
    // single-sourced. The lock adapter needs a dedicated lock-only pool.
    let lifecycle = crate::compose::sandbox::lifecycle(
        &config,
        &ports,
        &sandbox_dependencies,
        event_publisher.clone(),
    )
    .await?;
    let crate::compose::sandbox::SandboxDependencies {
        workspace_snapshots,
        sandbox_observer,
        sandbox_connector,
        sandbox_port_publisher,
        reaper_k8s,
        docker_watcher_client,
        sandbox_fleet_observer,
        storage_fallback,
        ..
    } = sandbox_dependencies;

    if let Some(docker_client) = docker_watcher_client {
        supervisor.supervise(
            "docker-container-watcher",
            // Readiness: sandbox lifecycle observation stops without it.
            Criticality::Readiness,
            sandbox::docker::container_watcher::spawn_container_lifecycle_watcher(
                docker_client,
                event_publisher.clone(),
                cancel.clone(),
                config.sandbox.max_crash_restart_count,
                Some(lifecycle.clone()),
            ),
        );
        info!(
            max_crash_restart_count = config.sandbox.max_crash_restart_count,
            "Container lifecycle watcher spawned"
        );
    }

    // Spawn the K8s pod lifecycle watcher now that the DB repos exist. Only runs
    // in K8s mode (reaper_k8s is None under Docker; the container watcher
    // above covers Docker).
    if let Some(k8s) = reaper_k8s.as_ref() {
        supervisor.supervise(
            "k8s-pod-watcher",
            // Readiness: sandbox lifecycle observation stops without it.
            Criticality::Readiness,
            sandbox::k8s::pod_watcher::spawn_pod_lifecycle_watcher(
                k8s.clone(),
                event_publisher.clone(),
                cancel.clone(),
                config.sandbox.max_crash_restart_count,
                config.sandbox.node_archive_enabled,
                Some(lifecycle.clone()),
                storage_fallback.clone(),
                config.server.worker_identity.clone(),
            ),
        );
        info!(
            max_crash_restart_count = config.sandbox.max_crash_restart_count,
            "Pod lifecycle watcher spawned"
        );
    }

    let sandbox_state = sandbox::routes::SandboxApiState {
        observer: sandbox_observer.clone(),
        connector: sandbox_connector.clone(),
        lifecycle: Some(lifecycle.clone()),
        turns: Some(ports.turn_counter.clone() as Arc<dyn moatless_ports::task::TurnStateReader>),
        snapshots: workspace_snapshots.clone(),
        workspace_repository: workspace_repository.clone(),
    };
    let (mut router, mut api) = sandbox::routes::api_router(sandbox_state);

    let authentication = ports
        .build_authentication(Arc::new(config.clone()))
        .map_err(|error| format!("failed to build authentication: {error}"))?;
    let auth_middleware_state = authentication.layer();
    let (auth_router, auth_api) = authentication.routes();
    router = router.merge(auth_router);
    api.merge(auth_api);
    info!(auth_provider = ?config.auth.provider, "Auth routes registered");

    let crate::compose::application::ApplicationAssembly {
        app_state,
        adapter_installations,
        sandbox_notify,
        sandbox_member_registry,
    } = crate::compose::application::build(
        &config,
        &ports,
        lifecycle,
        sandbox_connector.clone(),
        event_publisher,
        s3_client,
    );

    let (domain_router, domain_api) = crate::compose::http::domain_routes(&app_state);
    router = router.merge(domain_router);
    api.merge(domain_api);
    // Off the OpenAPI document: git's smart-HTTP protocol is not a JSON API,
    // and its only client is git.
    router = router.merge(crate::git_proxy::git_proxy_router(app_state.clone()));
    // Off the OpenAPI document too: it forwards GitHub's own API, and its
    // only client is a pod's `gh`.
    router = router.merge(crate::github_proxy::github_proxy_router(app_state.clone()));
    // Off the OpenAPI document too: it forwards Forgejo's own API, and its
    // only client is a pod's `moat api`.
    router = router.merge(crate::forgejo_proxy::forgejo_proxy_router(
        app_state.clone(),
    ));
    // Off the OpenAPI document too: it forwards each upstream's own API,
    // and its only client is a pod's service relay.
    router = router.merge(crate::service_proxy::service_proxy_router(
        app_state.clone(),
    ));
    // Off the OpenAPI document too: it forwards the providers' own APIs,
    // and its only client is a pod's agent.
    router = router.merge(crate::model_gateway::model_gateway_router(
        app_state.clone(),
    ));
    // Held for the event-subscriber registration below: the Scripts this
    // starts run through the router's own state.
    let mut startup_scripts = None;
    // Held for the supervision below, out of the same value.
    let mut preview_relay = None;
    let mut terminal_relay = None;
    let mut change_poller = None;
    if let Some(t3code) = crate::compose::http::ui_rpc(
        &config,
        &ports,
        &app_state,
        crate::compose::http::UiRpcInputs {
            connector: sandbox_connector.clone(),
            snapshots: workspace_snapshots.clone(),
            observer: sandbox_observer.clone(),
            ports: sandbox_port_publisher.clone(),
        },
    )
    .await?
    {
        router = router.merge(t3code.router);
        startup_scripts = Some(t3code.startup_scripts);
        preview_relay = Some(t3code.preview_relay);
        terminal_relay = Some(t3code.terminal_relay);
        change_poller = Some(t3code.change_poller);
        device_pairing = true;
    }
    info!("Domain routes registered");

    // Readiness probe — reports the root supervisor's view of
    // readiness-critical background tasks, plus whether the database
    // answers at all.
    router = router.merge(startup::readiness_router(
        Some(startup::ReadinessPools {
            main: app_state.db.clone(),
            probe: moatless_core::db::create_probe_pool(&app_state.db),
        }),
        supervisor.health(),
    ));

    auth::jwt::warn_on_jwt_secret_fallback(&config);

    // Seed the bootstrap admin for local password auth. Unlike the other
    // startup seeds this aborts on failure: a local-auth instance with no
    // admin credential is unreachable, and continuing would produce a
    // server nobody can log into.
    if config.auth.provider == moatless_types::auth::AuthProvider::Local
        || config.auth.break_glass_enabled
    {
        moatless_identity::compose::ensure_local_admin(ports.pool.clone(), &config.auth)
            .await
            .map_err(|e| format!("failed to seed the local admin account: {e}"))?;
    }

    if let Some(ref key) = config.auth.test_api_key {
        moatless_identity::compose::ensure_test_user(ports.pool.clone(), key).await;
    }

    // Background workers
    // The Adapters that mirror a task's own events back to their provider.
    // Built here for the same reason as the delivery handlers: the
    // reactions read process ports, and no Adapter retains the aggregate.
    let mut subscribers = adapter_installations.event_subscribers(&app_state);
    // Not an Adapter: the T3 module's own reaction, registered here because
    // this is where both its handle and the subscriber set exist.
    if let Some(startup_scripts) = startup_scripts {
        subscribers.subscribe(
            [EventKey::Sandbox(SandboxStatus::Ready)],
            Arc::new(t3code::startup_scripts::StartupScriptSubscriber::new(
                startup_scripts,
                app_state.events.clone(),
            )),
        );
    }
    let event_subscribers = Arc::new(subscribers);
    if let Some(handle) =
        spawn_event_handler(&config, &app_state, event_subscribers, cancel.clone())
    {
        supervisor.supervise(
            "event-handler-consumer",
            // Readiness: agent lifecycle events stop being processed, so
            // tasks silently never complete on this replica.
            Criticality::Readiness,
            handle,
        );
    }

    // Cross-replica preview tabs. A Script run over HTTP opens its tab on
    // whichever replica the load balancer picked, and the person who
    // started it is socketed to a replica chosen independently; this is
    // what carries the open across. Gated on the T3 module, which owns
    // both halves.
    if let Some(preview_relay) = preview_relay {
        supervisor.supervise(
            "preview-tab-relay",
            // Optional: without it a Script's tab still opens for viewers
            // on the replica that ran it, and everyone else reaches the
            // same server through the preview URL or by opening the Script
            // from the UI. Shedding traffic over it would be the larger
            // outage.
            Criticality::Optional,
            preview_relay.spawn(cancel.clone()),
        );
    }

    // Cross-replica terminal sessions, for the same reason and over the
    // same split: a Script run over HTTP starts its session on whichever
    // replica the balancer picked.
    if let Some(terminal_relay) = terminal_relay {
        supervisor.supervise(
            "terminal-session-relay",
            // Optional: without it the session still runs and still lists
            // for viewers on the replica that started it. Everyone else
            // does not list it at all until a client of theirs opens the
            // terminal, which is where this stood before the relay.
            Criticality::Optional,
            terminal_relay.spawn(cancel.clone()),
        );
    }

    // The change counters every T3 subscription on this replica waits on.
    if let Some(change_poller) = change_poller {
        supervisor.supervise(
            "t3-change-poller",
            // Readiness: without it every open sidebar and conversation on
            // this replica stops updating while still reporting itself
            // live, and a restart reconnects them to a replica that serves.
            Criticality::Readiness,
            change_poller.spawn(cancel.clone()),
        );
    }

    // The record heads every Artifact page socket on this replica waits on.
    supervisor.supervise(
        "record-collection-watch",
        // Optional: without it a socket still rereads the change log on its
        // 15 s access recheck, so a page updates late rather than never.
        // Shedding traffic over it would be the larger outage.
        Criticality::Optional,
        app_state.records.spawn_watch(cancel.clone()),
    );

    // The Task listing feed's retention. `task_listing_changes` is written
    // by Task mutations whether or not the T3 compatibility module is
    // enabled, so this is not gated on it. What it deletes is what
    // `task_listing::resume` already refuses to replay from, so falling
    // behind costs disk and nothing else.
    supervisor.supervise(
        "listing-change-pruner",
        // Optional: a stalled pruner grows one table and changes no answer
        // any client receives — `resume` refuses the same cursors either
        // way. Shedding traffic over it would be the larger outage.
        Criticality::Optional,
        task_listing::spawn_change_pruner(app_state.db.clone(), cancel.clone()),
    );

    // The same job for the conversation feed, and Optional for the same
    // reason — `moatless_core::change_feed::conversation::resume` refuses the cursors this
    // reclaims, whether or not it has run.
    supervisor.supervise(
        "conversation-change-pruner",
        Criticality::Optional,
        moatless_tasks::activities::spawn_change_pruner(app_state.db.clone(), cancel.clone()),
    );

    // The same job for the event feed, Optional for the same reason: it
    // skips anything the dispatcher has not cleared, so falling behind
    // costs disk and never an event.
    supervisor.supervise(
        "task-event-pruner",
        Criticality::Optional,
        event_handler::spawn_change_pruner(app_state.db.clone(), cancel.clone()),
    );

    // Durable Adapter delivery. Webhook handlers admit into
    // `app_state.adapter_delivery` before acknowledging their sender; this
    // worker is what makes that acknowledgment true. Handlers are
    // registered per Adapter — an empty registry simply means this build
    // claims no kinds.
    let adapter_delivery_handlers = Arc::new(adapter_installations.delivery_handlers(&app_state));
    supervisor.supervise(
        "adapter-delivery-worker",
        // Readiness, not liveness: losing this worker only delays
        // durable webhooks (any replica can claim them), so shedding
        // traffic is correct but killing the process would take the
        // API down too. Liveness deliberately excludes it, so a stall
        // doesn't restart the pod; metrics keep the backlog visible.
        Criticality::Readiness,
        app_state
            .adapter_delivery_worker
            .spawn(adapter_delivery_handlers, cancel.clone()),
    );

    // The other half of that seam: an inbound event whose Loop buffers is
    // committed to `inbound_message_buffer` and its delivery reports
    // success, so nothing reaches the task until this flusher runs.
    supervisor.supervise(
        "inbound-buffer-flusher",
        // Readiness, for the same reason as the delivery worker: the rows
        // are durable and any replica can claim them, so losing this
        // flusher delays combined messages rather than dropping them.
        Criticality::Readiness,
        app_state
            .message_buffer_worker
            .spawn(app_state.routing.clone(), cancel.clone()),
    );

    // Build cron leader guard before reaper_k8s is moved
    let cron_leader = reaper_k8s
        .as_ref()
        .map(|k8s| crate::compose::workers::leader(k8s, &config, "moatless-cron-scheduler", 120));

    let org_sync_leader = reaper_k8s
        .as_ref()
        .map(|k8s| crate::compose::workers::leader(k8s, &config, "moatless-org-sync", 900));

    let loop_sync_leader = reaper_k8s
        .as_ref()
        .map(|k8s| crate::compose::workers::leader(k8s, &config, "moatless-loop-sync", 300));

    // The desired-state reconciler runs in both modes: lease-elected on K8s,
    // lease-free single-replica on Docker (a `None` guard always reconciles).
    let reconciler_leader = reaper_k8s.as_ref().map(|k8s| {
        crate::compose::workers::leader(k8s, &config, "moatless-sandbox-reconciler", 120)
    });
    let reconciler = ports.sandbox_graph.reconciler(
        app_state.sandbox.clone(),
        moatless_sandbox_control::compose::ReconcilerInputs {
            observer: sandbox_fleet_observer,
            notify: sandbox_notify,
            pending_work: ports.pending_work.clone(),
            member_registry: sandbox_member_registry,
            config: sandbox::lifecycle::ReconcilerConfig::default(),
        },
    );
    supervisor.supervise(
        "sandbox-reconciler",
        // Readiness: durable desired-state synchronization stops, so
        // Sandbox intent and the real fleet drift apart unnoticed.
        Criticality::Readiness,
        sandbox::lifecycle::reconciler::spawn(reconciler, reconciler_leader, cancel.clone()),
    );

    if let Some(k8s) = reaper_k8s {
        if let Some(handle) = spawn_task_reaper(
            &app_state,
            sandbox_connector.clone(),
            k8s.clone(),
            cancel.clone(),
        ) {
            supervisor.supervise(
                "task-reaper",
                // Readiness: required cleanup — idle sandboxes and inactive
                // tasks accumulate for as long as it is gone.
                Criticality::Readiness,
                handle,
            );
        }
        if let Some(handle) = spawn_codex_refresher(&app_state, k8s, cancel.clone()) {
            supervisor.supervise(
                "codex-refresher",
                // Readiness: durable credential synchronization — agents
                // start failing once tokens expire unrefreshed.
                Criticality::Readiness,
                handle,
            );
        }
    } else {
        info!("Task reaper disabled (Docker mode — K8s leader election not available)");
    }

    // Schedule runner and long-polling adapters (when configured).
    if app_state.adapters.is_some() {
        if config.sandbox.cron_scheduler_enabled {
            supervisor.supervise(
                "cron-scheduler",
                // Readiness: durable scheduled jobs stop firing entirely.
                Criticality::Readiness,
                crate::compose::workers::schedule_runner(&ports, &app_state)
                    .spawn(cron_leader, cancel.clone()),
            );
            supervisor.supervise(
                "default-branch-loop-sync",
                // Optional: a merged `LOOP.md` waits for `sync-config`
                // instead, and every synced Loop keeps firing. Gated with
                // the scheduler so a shadow backend sharing the database
                // never writes its Loops.
                Criticality::Optional,
                crate::repository_sync::watch::DefaultBranchWatch::new(
                    app_state.repositories.clone(),
                    app_state.connection_routing.clone(),
                    app_state.loop_sync.clone(),
                    app_state.git_sync.clone(),
                )
                .spawn(
                    loop_sync_leader,
                    crate::repository_sync::watch::DEFAULT_BRANCH_POLL_INTERVAL,
                    cancel.clone(),
                ),
            );
            if let Some(org_sync) = moatless_workspaces::compose::org_sync(
                app_state.db.clone(),
                app_state.repositories.clone(),
                app_state.git_sync.clone(),
                app_state.git_host_tokens.clone(),
                config.http.git_content,
                config.github_api_base.clone(),
                &config.workspace_sync,
            ) {
                supervisor.supervise(
                    "github-org-sync",
                    // Optional: a new organization repository waits for an
                    // explicit registration, and every registered one keeps
                    // working. Gated with the scheduler, as above.
                    Criticality::Optional,
                    crate::repository_sync::org_watch::spawn(
                        org_sync,
                        app_state.connection_routing.clone(),
                        org_sync_leader,
                        cancel.clone(),
                    ),
                );
            }
        } else {
            info!("Cron scheduler disabled (CRON_SCHEDULER_ENABLED=false)");
        }
    }

    let router = crate::compose::http::finish(
        router,
        api,
        feature_flags,
        device_pairing,
        sse_state,
        auth_middleware_state,
    );

    // ── Serve ───────────────────────────────────────────────────────────
    // The API server is the last supervised task; binding still fails startup
    // directly, but once it is serving its termination follows the same policy
    // as every other owned task.
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    let listener = TcpListener::bind(addr).await?;
    info!(port = config.server.port, "API server started");
    let api_cancel = cancel.clone();
    supervisor.supervise(
        "api-server",
        // Fatal: without it the process serves nothing.
        Criticality::Fatal,
        tokio::spawn(async move {
            // `into_make_service`, rather than the `Router` itself: as a
            // make-service the `Router` rebuilds every route for each accepted
            // connection, ~6 MiB here, so a preview's burst of `ForwardAuth`
            // connections took the backend past its memory limit.
            if let Err(e) = axum::serve(listener, router.into_make_service())
                .with_graceful_shutdown(async move { api_cancel.cancelled().await })
                .await
            {
                tracing::error!(error = %e, "API server failed");
            }
        }),
    );

    // Run until a critical task terminates or a shutdown signal arrives, then
    // join the rest within a bounded deadline. A fatal exit becomes this
    // process's error, and so its non-zero exit status.
    supervisor.run(SHUTDOWN_JOIN_DEADLINE).await?;

    // The API server returned, so the process is going down: cancel the owned
    // background work and give it a bounded window to finish.
    cancel.cancel();
    if let Some(provider) = otel_provider {
        shutdown_otel_provider(provider).await;
    }

    Ok(())
}

/// Apply pending migrations from `DATABASE_URL` and exit, for the chart's
/// `migrate` init container. No startup probe runs until it exits, so a long
/// backfill finishes rather than being killed and rolled back.
pub async fn migrate() -> Result<(), Box<dyn std::error::Error>> {
    #[allow(clippy::expect_used)]
    // fail-fast: TLS provider must be installed before any networking
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    let otel_provider = startup::init_tracing(&moatless_core::config::OtelConfig::from_env());
    let url = Config::database_url_from_env();
    let result = crate::compose::infrastructure::init_database(url.as_deref(), 2, true).await;
    if let Some(provider) = otel_provider {
        shutdown_otel_provider(provider).await;
    }
    result?.close().await;
    Ok(())
}

/// How long shutdown waits for the OTel batch span processor to flush before
/// giving up. Best effort: a slow collector loses buffered spans, not the
/// process shutdown.
const OTEL_SHUTDOWN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Flush and shut down the OTel tracer provider off the async runtime
/// (`shutdown` blocks the calling thread while it drains the batch processor),
/// bounded by [`OTEL_SHUTDOWN_DEADLINE`].
async fn shutdown_otel_provider(provider: opentelemetry_sdk::trace::SdkTracerProvider) {
    let shutdown = tokio::task::spawn_blocking(move || provider.shutdown());
    match tokio::time::timeout(OTEL_SHUTDOWN_DEADLINE, shutdown).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => warn!(error = %e, "OTel tracer provider shutdown reported an error"),
        Ok(Err(e)) => warn!(error = %e, "OTel tracer provider shutdown task panicked"),
        Err(_) => warn!("OTel tracer provider shutdown timed out; buffered spans may be lost"),
    }
}

// =========================================================================
// Small private helpers (kept in main to avoid over-abstracting)
// =========================================================================

/// Spawn the fleet's event dispatcher, unless this process is a shadow.
///
/// `None` means nothing started, so there is nothing for the supervisor to own.
fn spawn_event_handler(
    config: &Config,
    app_state: &AppState,
    subscribers: Arc<TaskEventSubscribers>,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    if !crate::compose::workers::event_handler_enabled(config) {
        return None;
    }
    let state = app_state.clone();
    let handle = tokio::spawn(event_handler::spawn_event_handler(
        state,
        subscribers,
        cancel,
    ));
    info!("Event dispatcher spawned");
    Some(handle)
}

/// Spawn the unified task reaper (idle-sandbox stop, auto-settle and auto-close) if
/// enabled. `None` when the reaper is switched off.
fn spawn_task_reaper(
    app_state: &AppState,
    connector: Arc<dyn sandbox_proto::client::SandboxConnector>,
    k8s: sandbox::k8s::client::K8sClient,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    let handle = crate::compose::workers::task_reaper(app_state, connector, k8s)?.spawn(cancel);
    info!(
        idle_timeout_minutes = app_state.config.sandbox.idle_timeout_minutes,
        check_interval_secs = app_state.config.sandbox.idle_check_interval_secs,
        settle_after_hours = app_state.config.sandbox.settle_after_hours,
        close_after_settled_days = app_state.config.sandbox.close_after_settled_days,
        "Task reaper spawned"
    );
    Some(handle)
}

/// Spawn the background Codex token refresher if enabled. Leader-elected via a
/// dedicated K8s Lease (`moatless-codex-refresher`). `None` when disabled.
fn spawn_codex_refresher(
    app_state: &AppState,
    k8s: sandbox::k8s::client::K8sClient,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    let interval_secs = app_state.config.sandbox.codex_refresher_interval_secs;
    if interval_secs == 0 {
        info!("Codex refresher disabled (CODEX_REFRESHER_INTERVAL_SECS=0)");
        return None;
    }
    let interval = std::time::Duration::from_secs(interval_secs);
    let lease_duration_secs: i32 = (interval.as_secs() * 2).try_into().unwrap_or(i32::MAX);
    let leader_guard = crate::compose::workers::leader(
        &k8s,
        &app_state.config,
        "moatless-codex-refresher",
        lease_duration_secs,
    );
    let handle = moatless_tasks::credential_sync::refresher::spawn_codex_refresher(
        app_state.harness_credentials.clone(),
        Arc::clone(&app_state.credential_sync),
        leader_guard,
        app_state.config.server.worker_identity.clone(),
        interval,
        cancel,
    );
    info!(interval_secs, "Codex refresher spawned");
    Some(handle)
}
