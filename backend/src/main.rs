//! Moatless Vibe Backend — entry point.
//!
//! Orchestrates startup by delegating to focused helpers in [`startup`],
//! factory constructors on state structs, and per-module router builders.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use moatless_vibe_backend::agent;
use moatless_vibe_backend::auth;
use moatless_vibe_backend::compose::{compose, register_builtin_delivery_handlers, Ports};
use moatless_core::config::{Config, SandboxProviderType};
use moatless_vibe_backend::event_handler;
use moatless_vibe_backend::event_ingress::relay::{IngressRelay, IngressRelayHandle};
use moatless_core::feature_flags;
use moatless_core::leader::LeaderGuard;
use moatless_core::tag;
use moatless_vibe_backend::sandbox;
use moatless_vibe_backend::sandbox::k8s::leader_election::K8sLeaderGuard;
use moatless_vibe_backend::sse::routes::sse_router;
use moatless_vibe_backend::startup;
use moatless_vibe_backend::state::AppState;
use moatless_vibe_backend::supervisor::{Criticality, Supervisor};
use moatless_vibe_backend::{
    adapters, feedback, file_storage, loops, message, plugin, provider_settings, repository,
    repository_sync, secret, skill, t3code, task, task_connections, task_event, team, workspace,
};

/// How long shutdown waits for supervised tasks to finish before giving up and
/// naming the stragglers. Sits comfortably inside Kubernetes' default 30s
/// termination grace period so the report is emitted before SIGKILL.
const SHUTDOWN_JOIN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Handle --dump-openapi before any runtime initialization
    if std::env::args().any(|a| a == "--dump-openapi") {
        let spec = startup::build_merged_openapi();
        #[allow(clippy::print_stdout)] // CLI tool output, not server logging
        {
            println!("{}", spec.to_pretty_json()?);
        }
        return Ok(());
    }

    // Dispatch `secrets …` subcommand before server runtime initialization.
    if std::env::args().nth(1).as_deref() == Some("secrets") {
        use clap::Parser;

        #[derive(Parser)]
        #[command(name = "moatless-vibe-backend")]
        struct SecretsEntry {
            #[command(subcommand)]
            command: SecretsTop,
        }

        #[derive(clap::Subcommand)]
        enum SecretsTop {
            #[command(subcommand)]
            Secrets(secret::cli::SecretsCommand),
        }

        let SecretsEntry {
            command: SecretsTop::Secrets(cmd),
        } = SecretsEntry::parse();
        startup::init_tracing(&moatless_core::config::OtelConfig::from_env());
        return secret::cli::run(secret::cli::SecretsCli { command: cmd }).await;
    }

    // Install rustls crypto provider (required before any TLS operations)
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
    info!(sandbox_namespace = %config.sandbox.namespace, "Starting Moatless Vibe Backend");

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
    let db_pool = startup::init_database(
        config.database_url.as_deref(),
        config.database.max_connections,
        config.database.run_migrations,
    )
    .await;
    let config = if let Some(ref pool) = db_pool {
        match moatless_vibe_backend::adapter_apps::carry_over_env_and_load_config(&config, pool)
            .await
        {
            Ok(config) => config,
            Err(e) => {
                warn!(
                    error = %e,
                    "Failed to load DB-backed Adapter app config; using deployment config"
                );
                config
            }
        }
    } else {
        config
    };
    // The composition root: every port this process has, built once from the
    // pool and the configuration it just resolved. Everything downstream
    // receives ports from here rather than building its own.
    let ports: Option<Ports> = db_pool.as_ref().map(|pool| compose(pool.clone(), &config));
    if let Some(ref ports) = ports {
        // Off the boot path: carrying over proves each App against GitHub, and
        // the API must not wait on GitHub to start answering.
        tokio::spawn({
            let github_apps = ports.github_apps.clone();
            let entries = config.auth.github_apps.clone();
            async move { github_apps.carry_over_env_apps(&entries).await }
        });
    }
    let event_publisher = startup::init_event_publisher(&config).await;
    let (sse_state, sse_consumer) = startup::init_sse(
        &config,
        ports.as_ref().map(|p| p.tasks.clone()),
        cancel.clone(),
    )
    .await?;
    if let Some(handle) = sse_consumer {
        supervisor.supervise(
            "sse-redis-consumer",
            // Readiness: without it the live SSE tail silently stops, so this
            // replica must stop receiving traffic.
            Criticality::Readiness,
            handle,
        );
    }
    let s3_client = startup::init_s3(&config).await;
    let provision_deps = ports.as_ref().map(|p| {
        moatless_vibe_backend::sandbox::lifecycle::ProvisionDeps::new(
            p.tasks.clone(),
            p.repositories.clone(),
            p.provider_tokens.clone(),
            p.plugins.clone(),
            p.workspaces.clone(),
        )
    });
    // Build the configured runtime/observer/connector adapter once. The rich
    // 13-field read the HTTP routes need is the backend-local
    // `SandboxStatusReader`; the T3 contract observer is projected from it at
    // its one construction site, and the one connector serves both surfaces.
    let sandbox_runtime: Arc<dyn sandbox::lifecycle::SandboxRuntime>;
    let sandbox_observer: Arc<dyn sandbox::status_reader::SandboxStatusReader>;
    let sandbox_connector: Arc<dyn moatless_backend_core::sandbox::SandboxConnector>;
    let cluster_diagnostics: Arc<dyn moatless_backend_core::sandbox::ClusterDiagnostics>;
    let default_sandbox_image: String;
    let reaper_k8s: Option<sandbox::k8s::client::K8sClient>;
    let docker_watcher_client: Option<sandbox::docker::client::DockerClient>;
    // The reconciler's per-tick fleet observation, built per runtime backend.
    let sandbox_fleet_observer: Arc<dyn sandbox::lifecycle::FleetObserver>;

    match config.sandbox_provider {
        SandboxProviderType::Kubernetes => {
            let k8s =
                sandbox::k8s::client::K8sClient::new(config.sandbox.namespace.clone()).await?;
            info!("K8s client initialized");

            reaper_k8s = Some(k8s.clone());
            docker_watcher_client = None;
            let adapter = startup::build_k8s_sandbox_adapter(
                &config,
                k8s.clone(),
                s3_client
                    .clone()
                    .map(|storage| std::sync::Arc::new(storage.client)),
                provision_deps.clone(),
            );
            sandbox_runtime = adapter.clone();
            sandbox_observer = adapter.clone();
            sandbox_connector = adapter.clone();
            cluster_diagnostics = adapter;
            default_sandbox_image = config.sandbox.image.clone();
            sandbox_fleet_observer = Arc::new(sandbox::k8s::fleet_observer::K8sFleetObserver::new(
                k8s.clone(),
                sandbox_observer.clone(),
            ));

            // The K8s pod lifecycle watcher is spawned after the DB repos are
            // built (it needs them to auto-disable crash-looping servers).
        }
        SandboxProviderType::Docker => {
            let docker_client = sandbox::docker::client::DockerClient::new(
                config.docker.network.clone(),
                config.docker.data_dir.clone(),
                config.docker.container_data_dir.clone(),
            )?;
            info!("Docker client initialized");

            reaper_k8s = None;
            docker_watcher_client = Some(docker_client.clone());
            let adapter = startup::build_docker_sandbox_adapter(
                &config,
                docker_client.clone(),
                s3_client
                    .clone()
                    .map(|storage| std::sync::Arc::new(storage.client)),
                provision_deps.clone(),
            );
            sandbox_runtime = adapter.clone();
            sandbox_observer = adapter.clone();
            sandbox_connector = adapter.clone();
            cluster_diagnostics = adapter;
            default_sandbox_image = config.docker.image.clone();
            sandbox_fleet_observer =
                Arc::new(sandbox::docker::fleet_observer::DockerFleetObserver::new(
                    docker_client.clone(),
                    sandbox_observer.clone(),
                ));

            // The Docker watcher is spawned after lifecycle is assembled so
            // every observation is classified against durable intent.
        }
    }

    supervisor.supervise(
        "grpc-server",
        // Fatal: every sandbox reaches the backend through this server.
        Criticality::Fatal,
        startup::spawn_grpc_server(
            &config,
            ports.as_ref(),
            event_publisher.clone(),
            s3_client.clone().map(std::sync::Arc::new),
            sandbox_connector.clone(),
            cancel.clone(),
        ),
    );

    // ── Build router ────────────────────────────────────────────────────

    // Sandbox route state keeps only authorization DAOs. The pod watcher
    // separately needs workspace and task-event DAOs.
    let sandbox_tasks = ports.as_ref().map(|p| p.tasks.clone());
    let sandbox_teams = ports.as_ref().map(|p| p.teams.clone());
    let sandbox_workspaces = ports.as_ref().map(|p| p.workspaces.clone());
    let sandbox_task_events = ports.as_ref().map(|p| p.task_events.clone());

    // Build the shared durable Sandbox lifecycle handle once (requires the DB).
    // The sandbox API, the pod watcher, and AppState all drive transitions
    // through this single instance, so Sandbox intent and per-Task locks stay
    // single-sourced. The lock adapter needs a dedicated lock-only pool.
    let lifecycle: Option<Arc<moatless_vibe_backend::sandbox::lifecycle::Lifecycle>> =
        match ports.as_ref() {
            Some(ports) => {
                let db_url = config
                    .database_url
                    .as_deref()
                    .ok_or("DATABASE_URL required to build the lifecycle lock pool")?;
                let lock_pool = moatless_core::db::create_pool_kind(
                    db_url,
                    config.database.max_connections,
                    moatless_core::db::PoolKind::Lock,
                )
                .await?;
                Some(
                    ports.build_lifecycle(
                        lock_pool,
                        provision_deps
                            .clone()
                            .ok_or("provision dependencies missing with database pool")?,
                        sandbox_runtime.clone(),
                        sandbox_connector.clone(),
                        default_sandbox_image.clone(),
                        event_publisher.clone(),
                    ),
                )
            }
            None => None,
        };

    if let Some(docker_client) = docker_watcher_client {
        if let Some(ref publisher) = event_publisher {
            supervisor.supervise(
                "docker-container-watcher",
                // Readiness: sandbox lifecycle observation stops without it.
                Criticality::Readiness,
                sandbox::docker::container_watcher::spawn_container_lifecycle_watcher(
                    docker_client,
                    publisher.clone(),
                    cancel.clone(),
                    config.sandbox.max_crash_restart_count,
                    lifecycle.clone(),
                ),
            );
            info!(
                max_crash_restart_count = config.sandbox.max_crash_restart_count,
                "Container lifecycle watcher spawned"
            );
        } else {
            info!("Container lifecycle watcher disabled (requires Redis)");
        }
    }

    // Spawn the K8s pod lifecycle watcher now that the DB repos exist. It needs
    // task + repo access (plus credentials) to auto-disable crash-looping server
    // containers and redeploy without them. Only runs in K8s mode (reaper_k8s is
    // None under Docker) and when an event publisher (Redis) is available.
    match (reaper_k8s.as_ref(), event_publisher.as_ref()) {
        (Some(k8s), Some(publisher)) => {
            supervisor.supervise(
                "k8s-pod-watcher",
                // Readiness: sandbox lifecycle observation stops without it.
                Criticality::Readiness,
                sandbox::k8s::pod_watcher::spawn_pod_lifecycle_watcher(
                    k8s.clone(),
                    publisher.clone(),
                    cancel.clone(),
                    config.sandbox.max_crash_restart_count,
                    config.sandbox.node_archive_enabled,
                    lifecycle.clone(),
                    sandbox_tasks.clone(),
                    sandbox_workspaces.clone(),
                    sandbox_task_events.clone(),
                ),
            );
            info!(
                max_crash_restart_count = config.sandbox.max_crash_restart_count,
                "Pod lifecycle watcher spawned"
            );
        }
        (Some(_), None) => info!("Pod lifecycle watcher disabled (requires Redis)"),
        (None, _) => {} // Docker mode — container watcher already spawned above
    }

    let sandbox_state = sandbox::routes::SandboxApiState {
        observer: sandbox_observer.clone(),
        connector: sandbox_connector.clone(),
        lifecycle: lifecycle.clone(),
        tasks: sandbox_tasks,
        teams: sandbox_teams,
    };
    let (mut router, mut api) = sandbox::routes::api_router(sandbox_state);

    // Auth routes (needs DB)
    let mut auth_middleware_state: Option<auth::middleware::AuthMiddlewareState> = None;
    if let Some(ref ports) = ports {
        let auth_state = ports.build_auth_state(Arc::new(config.clone()));

        // Build middleware state sharing the same repos and cache
        auth_middleware_state = Some(auth::middleware::AuthMiddlewareState {
            api_keys: auth_state.api_keys.clone(),
            users: auth_state.users.clone(),
            cache: auth_state.cache.clone(),
            config: auth_state.config.clone(),
        });

        let (auth_router, auth_api) = auth::routes::auth_router(auth_state);
        router = router.merge(auth_router);
        api.merge(auth_api);
        info!(auth_provider = ?config.auth.provider, "Auth routes registered");
    } else {
        info!("Auth routes disabled (no database)");
    }

    // Owned handle to the ingress-outbox relay (see `spawn_ingress_relay`). Held
    // here rather than detached because the relay is critical: it is the only
    // path an admitted `agent.message` payload has back to the consumer after a
    // failed publish.
    let mut ingress_relay: Option<IngressRelayHandle> = None;

    // Domain routes (need DB)
    if let Some(ports) = ports {
        let (adapter_registry, provider_handles) = adapters::Adapters::from_config(
            &config,
            ports.pool.clone(),
            ports.provider_tokens.clone(),
            ports.provider_token_auth.clone(),
            ports.github_apps.clone(),
            ports.github_app_service.clone(),
            ports.secrets.clone(),
        )
        .await;
        // The shared lifecycle was built above; it is present whenever the DB is.
        let lifecycle = lifecycle
            .ok_or("lifecycle handle required to build AppState (missing database pool)")?;
        let sandbox_notify: Arc<dyn sandbox::lifecycle::NotifyPort> = Arc::new(
            sandbox::lifecycle::GrpcNotify::new(sandbox_connector.clone()),
        );
        // The reaper's consumer-owned agent-status port. Handed to
        // `spawn_task_reaper` at construction rather than carried on `AppState`.
        let agent_status: Arc<dyn crate::task::reaper::AgentStatusPort> = Arc::new(
            crate::task::reaper::ConnectorAgentStatus::new(sandbox_connector.clone()),
        );
        let app_state = AppState::new(
            &ports,
            Arc::new(config.clone()),
            lifecycle,
            sandbox_connector.clone(),
            sandbox_notify.clone(),
            event_publisher,
            s3_client,
            Some(Arc::new(adapter_registry)),
            provider_handles,
        );

        // The browser tabs open on each Task. Built at the composition root
        // because the choice between Redis and this process is a deployment
        // fact, and its subscriber task is the root supervisor's to own.
        let (preview_tabs, preview_event_forwarder) = moatless_t3code::preview_tabs::build(
            config.redis.url.as_deref(),
            moatless_t3code::preview_tabs::PreviewTabTtl(std::time::Duration::from_secs(
                config.sandbox.idle_timeout_minutes * 60,
            )),
        )
        .await;
        if let Some(handle) = preview_event_forwarder {
            supervisor.supervise(
                "preview-event-subscriber",
                // Optional: without it this replica still serves every tab
                // read and write, and only stops hearing about tabs opened on
                // the other replica.
                Criticality::Optional,
                handle,
            );
        }

        let modules = vec![
            agent::routes::agent_router(app_state.clone()),
            task::routes::task_router(app_state.clone()),
            task_connections::routes::task_connections_router(app_state.clone()),
            feedback::routes::feedback_router(app_state.clone()),
            message::routes::message_router(app_state.clone()),
            file_storage::routes::file_storage_router(app_state.clone()),
            repository::routes::repository_router(app_state.clone()),
            repository_sync::routes::repository_sync_router(app_state.clone()),
            secret::routes::secret_router(app_state.clone()),
            tag::routes::tag_router(app_state.clone()),
            provider_settings::routes::provider_settings_router(app_state.clone()),
            adapters::routes::adapter_router(app_state.clone()),
            loops::http::loops_router(app_state.clone()),
            team::routes::team_router(app_state.clone()),
            task_event::routes::task_event_router(app_state.clone()),
            plugin::routes::plugin_router(app_state.clone()),
            skill::routes::skill_router(app_state.clone()),
            workspace::routes::workspace_router(app_state.clone()),
        ];
        for (mod_router, mod_api) in modules {
            router = router.merge(mod_router);
            api.merge(mod_api);
        }
        // Historical task-events replay. Authorizes the caller against the task
        // (AuthUser), so it lives on the AppState router rather than the SSE
        // firehose router.
        router = router.merge(moatless_vibe_backend::sse::routes::task_events_router(
            app_state.clone(),
        ));
        if let Some(t3code_router) = {
            let ui_rpc_conversations = ports.ui_rpc_conversations.clone();
            moatless_t3code::router_from_env(moatless_t3code::UiRpcDependencies {
                session_cookie_name: config.auth.session_cookie_name.clone(),
                session_jwt_secret: auth::jwt::get_jwt_secret(&config).map(String::from),
                pairing: ports.pairing.clone(),
                workspaces: ui_rpc_conversations.clone(),
                conversations: ui_rpc_conversations.clone(),
                sandboxes: sandbox_connector.clone(),
                observer: t3code::projection::contract_observer(sandbox_observer.clone()),
                diagnostics: cluster_diagnostics.clone(),
                tasks: ui_rpc_conversations,
                intent: app_state.sandbox.clone(),
                starter: app_state.sandbox.clone(),
                stopper: app_state.sandbox.clone(),
                preview_tabs: preview_tabs.clone(),
                agents: Arc::new(t3code::backend::AppStateAgents(app_state.clone())),
                branches: Arc::new(t3code::backend::AppStateBranches(app_state.clone())),
                pull_requests: Arc::new(t3code::backend::AppStateBoundPullRequests(
                    app_state.clone(),
                )),
                source_control_auth: Arc::new(t3code::backend::AppStateSourceControlAuth(
                    app_state.clone(),
                )),
                commands: Arc::new(t3code::backend::AppStateCommands(app_state.clone())),
                model_defaults: moatless_t3code::AgentModelDefaults {
                    claude_code: config.sandbox.default_model.clone(),
                    codex: config.sandbox.default_codex_model.clone(),
                },
                otlp_traces_enabled: config.otel.otlp_endpoint.is_some(),
                frontend_url: config.auth.frontend_url.clone(),
            })
            .await?
        } {
            router = router.merge(t3code_router);
        }
        info!("Domain routes registered");

        // Readiness probe — reports Redis connectivity and the root
        // supervisor's view of readiness-critical background tasks, plus
        // whether the main database pool can still serve work promptly.
        router = router.merge(startup::readiness_router(
            Some(app_state.db.clone()),
            app_state.events.clone(),
            supervisor.health(),
        ));

        // Seed default repositories
        repository::seed::seed_default_repositories(&*app_state.repositories).await;

        auth::jwt::warn_on_jwt_secret_fallback(&config);

        // Seed the bootstrap admin for local password auth. Unlike the other
        // startup seeds this aborts on failure: a local-auth instance with no
        // admin credential is unreachable, and continuing would produce a
        // server nobody can log into.
        if config.auth.provider == moatless_core::config::AuthProvider::Local {
            auth::password::bootstrap::ensure_local_admin(
                ports.users.as_ref(),
                ports.user_credentials.as_ref(),
                &config.auth,
            )
            .await
            .map_err(|e| format!("failed to seed the local admin account: {e}"))?;
        }

        // Ensure test API user + key (if configured)
        if let Some(ref key) = config.auth.test_api_key {
            auth::test_user::ensure_test_user(ports.users.as_ref(), ports.api_keys.as_ref(), key)
                .await;
        }

        // Background workers
        if let Some(handle) = spawn_event_handler(&config, &app_state, cancel.clone()).await {
            supervisor.supervise(
                "event-handler-consumer",
                // Readiness: agent lifecycle events stop being processed, so
                // tasks silently never complete on this replica.
                Criticality::Readiness,
                handle,
            );
        }

        // The listing feed's retention. `task_listing_changes` is written by
        // Moatless mutations whether or not the T3 compatibility module is
        // enabled, so this is not gated on it. What it deletes is what
        // `task_listing::resume` already refuses to replay from, so falling
        // behind costs disk and nothing else.
        supervisor.supervise(
            "listing-change-pruner",
            // Optional: a stalled pruner grows one table and changes no answer
            // any client receives — `resume` refuses the same cursors either
            // way. Shedding traffic over it would be the larger outage.
            Criticality::Optional,
            moatless_core::task_listing::spawn_change_pruner(
                app_state.db.clone(),
                cancel.clone(),
            ),
        );

        // The same job for the conversation feed, and Optional for the same
        // reason — `task_activities::resume` refuses the cursors this
        // reclaims, whether or not it has run.
        supervisor.supervise(
            "conversation-change-pruner",
            Criticality::Optional,
            moatless_vibe_backend::task_activities::spawn_change_pruner(
                app_state.db.clone(),
                cancel.clone(),
            ),
        );

        // Durable Adapter delivery. Webhook handlers admit into
        // `app_state.adapter_delivery` before acknowledging their sender; this
        // worker is what makes that acknowledgment true. Handlers are
        // registered per Adapter — an empty registry simply means this build
        // claims no kinds.
        let mut adapter_delivery_handlers = adapters::delivery::AdapterDeliveryHandlers::new();
        register_builtin_delivery_handlers(&mut adapter_delivery_handlers, &app_state);
        let adapter_delivery_handlers = Arc::new(adapter_delivery_handlers);
        supervisor.supervise(
            "adapter-delivery-worker",
            // Readiness: what it owns is durable, so losing this worker
            // delays admitted webhooks rather than dropping them — any replica
            // can claim the same records. Shedding traffic from a replica that
            // can no longer execute what it admits is the proportionate
            // response; killing the process would take the API down with it.
            // The consequence is deliberate: liveness stays on `/health`, so
            // nothing restarts the pod, and on a single-replica deployment the
            // backlog stalls until an operator acts. That is why the worker
            // publishes `moatless_adapter_delivery_backlog` and
            // `moatless_adapter_delivery_oldest_pending_seconds` — a stalled
            // backlog has to be visible, not merely survivable.
            Criticality::Readiness,
            adapters::delivery::spawn_adapter_delivery_worker(
                app_state.adapter_delivery.clone(),
                adapter_delivery_handlers,
                cancel.clone(),
            ),
        );

        // The ingress-outbox relay keeps its own handle rather than joining the
        // supervisor: it is shut down explicitly after `supervisor.run` returns,
        // on its own deadline (see `RELAY_SHUTDOWN_DEADLINE`).
        ingress_relay = spawn_ingress_relay(&app_state, cancel.clone());

        // Build cron leader guard before reaper_k8s is moved
        let cron_leader: Option<Arc<dyn LeaderGuard>> = reaper_k8s.as_ref().map(|k8s| {
            let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
            let election = sandbox::k8s::leader_election::LeaderElection::new_with_name(
                k8s,
                identity,
                "moatless-cron-scheduler".to_string(),
            )
            .with_lease_duration(120); // 2x the 60s tick interval
            Arc::new(K8sLeaderGuard(election)) as Arc<dyn LeaderGuard>
        });

        // The desired-state reconciler runs in both modes: lease-elected on K8s,
        // lease-free single-replica on Docker (a `None` guard always reconciles).
        let reconciler_leader: Option<Arc<dyn LeaderGuard>> = reaper_k8s.as_ref().map(|k8s| {
            let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
            let election = sandbox::k8s::leader_election::LeaderElection::new_with_name(
                k8s,
                identity,
                "moatless-sandbox-reconciler".to_string(),
            )
            .with_lease_duration(120);
            Arc::new(K8sLeaderGuard(election)) as Arc<dyn LeaderGuard>
        });
        let reconciler = ports.build_reconciler(
            app_state.sandbox.clone(),
            sandbox_fleet_observer,
            sandbox_notify,
            sandbox::lifecycle::ReconcilerConfig::default(),
        );
        supervisor.supervise(
            "sandbox-reconciler",
            // Readiness: durable desired-state synchronization stops, so
            // Sandbox intent and the real fleet drift apart unnoticed.
            Criticality::Readiness,
            spawn_sandbox_reconciler(reconciler, reconciler_leader, cancel.clone()),
        );

        if let Some(k8s) = reaper_k8s {
            if let Some(handle) = spawn_task_reaper(
                &app_state,
                agent_status.clone(),
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
                    loops::firing::schedule_runner::ScheduleRunner::new(
                        app_state.loops.clone(),
                        app_state.tasks.clone(),
                        app_state.message_send.clone(),
                        app_state.adapters.clone(),
                        app_state.task_creation.clone(),
                        app_state.sandbox_waker.clone(),
                    )
                    .spawn(cron_leader, cancel.clone()),
                );
            } else {
                info!("Cron scheduler disabled (CRON_SCHEDULER_ENABLED=false)");
            }

            // Telegram long-polling (when webhook mode is not available)
            if config.telegram.bot_token.is_some() && config.telegram.mode == "polling" {
                if let Some(telegram) = app_state.provider_handles.telegram.clone() {
                    supervisor.supervise(
                        "telegram-poller",
                        // Optional: losing it degrades the Telegram channel alone,
                        // and its own early-exit branches already log the reason.
                        Criticality::Optional,
                        adapters::telegram::spawn_telegram_poller(
                            app_state.clone(),
                            telegram,
                            cancel.clone(),
                        ),
                    );
                }
            }
        }
    } else {
        info!("Domain routes disabled (no database)");
    }

    // Routes that don't require a database
    router = router.route(
        "/api/v1/feature-flags",
        axum::routing::get(feature_flags::get_feature_flags),
    );
    router = router.merge(sse_router(sse_state));
    info!("SSE routes registered");

    let openapi_json = api.clone();
    let router = router.route(
        "/openapi.json",
        axum::routing::get(move || async move { Json(openapi_json) }),
    );

    // Apply auth middleware. Runs inside the request-trace span (added below)
    // so it can record `user_id` onto the current span.
    let router = if let Some(mw_state) = auth_middleware_state {
        info!("Auth middleware enabled");
        router
            .layer(axum::middleware::from_fn(auth::middleware::auth_middleware))
            .layer(axum::Extension(mw_state))
    } else {
        info!("Auth middleware disabled (no database)");
        router
    };

    // Outermost layer — opens one `http_request` span (request_id, method,
    // path, …) per request so every downstream log line inherits the context,
    // emits a single access-log line, and echoes `x-request-id` to clients.
    let router = router.layer(axum::middleware::from_fn(request_trace_middleware));

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
            if let Err(e) = axum::serve(listener, router)
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
    if let Some(relay) = ingress_relay {
        relay.shutdown(RELAY_SHUTDOWN_DEADLINE).await;
    }
    if let Some(provider) = otel_provider {
        shutdown_otel_provider(provider).await;
    }

    Ok(())
}

/// How long shutdown waits for the OTel batch span processor to flush and
/// close its exporter connection before giving up and exiting anyway. Best
/// effort: a slow or unreachable collector loses buffered spans, not the
/// process's own shutdown.
const OTEL_SHUTDOWN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Flush and shut down the OTel tracer provider off the async runtime
/// (`shutdown` blocks the calling thread while it drains the batch
/// processor), bounded by [`OTEL_SHUTDOWN_DEADLINE`].
async fn shutdown_otel_provider(provider: opentelemetry_sdk::trace::SdkTracerProvider) {
    let shutdown = tokio::task::spawn_blocking(move || provider.shutdown());
    match tokio::time::timeout(OTEL_SHUTDOWN_DEADLINE, shutdown).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => warn!(error = %e, "OTel tracer provider shutdown reported an error"),
        Ok(Err(e)) => warn!(error = %e, "OTel tracer provider shutdown task panicked"),
        Err(_) => warn!("OTel tracer provider shutdown timed out; buffered spans may be lost"),
    }
}

/// How long shutdown waits for the ingress-outbox relay before reporting it as
/// a straggler and exiting anyway.
///
/// Separate from `SHUTDOWN_JOIN_DEADLINE` because the two waits are sequential:
/// the supervisor is joined first, then the relay. Keeping this at 5s holds the
/// worst case to 25s, inside Kubernetes' default 30s termination grace period.
const RELAY_SHUTDOWN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Spawn the ingress-outbox relay, returning its owned handle.
///
/// Requires an event transport: with none configured the gRPC ingress refuses
/// to admit `agent.message` payloads at all (they would have no delivery path),
/// so there is nothing for a relay to re-publish.
fn spawn_ingress_relay(
    app_state: &AppState,
    cancel: CancellationToken,
) -> Option<IngressRelayHandle> {
    let publisher = app_state.events.clone()?;
    let handle = IngressRelay::new(app_state.ingress_outbox.clone(), publisher).spawn(cancel);
    info!("Ingress outbox relay spawned");
    Some(handle)
}

// =========================================================================
// Small private helpers (kept in main to avoid over-abstracting)
// =========================================================================

/// Per-request tracing span + access log + `x-request-id` correlation.
///
/// As the outermost middleware it opens one `http_request` span carrying a
/// generated `request_id`, the `method`, and the `path` (with `user_id`,
/// `status`, and `latency_ms` recorded later), and enters it for the whole
/// downstream stack so every child log line inherits the context (rendered by
/// the JSON subscriber under `span` → queryable in Loki as `span_request_id`,
/// `span_user_id`, …). On completion it records status/latency, emits a single
/// access-log line (skipped for K8s probes to avoid log spam), and echoes
/// `x-request-id` back to the client for support correlation.
async fn request_trace_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use tracing::Instrument;

    let method = req.method().clone();
    let path = req.uri().path().to_string();
    // K8s liveness/readiness probes hit these on a tight interval; access-logging
    // them would recreate the high-volume probe noise we are trying to remove.
    let is_probe = matches!(path.as_str(), "/health" | "/healthz" | "/ready");
    let request_id = uuid::Uuid::new_v4().to_string();

    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %path,
        user_id = tracing::field::Empty,
        status = tracing::field::Empty,
        latency_ms = tracing::field::Empty,
    );

    let start = std::time::Instant::now();
    async move {
        let mut res = next.run(req).await;
        let latency_ms = start.elapsed().as_millis() as u64;
        let span = tracing::Span::current();
        span.record("status", res.status().as_u16());
        span.record("latency_ms", latency_ms);
        if !is_probe {
            // Successful requests are high-volume access-log noise → log at
            // DEBUG. Escalate failures so they stay visible by default: 5xx at
            // ERROR, 4xx at WARN.
            if res.status().is_server_error() {
                tracing::error!("request completed");
            } else if res.status().is_client_error() {
                tracing::warn!("request completed");
            } else {
                tracing::debug!("request completed");
            }
        }
        if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
            res.headers_mut().insert("x-request-id", value);
        }
        res
    }
    .instrument(span)
    .await
}

/// Spawn the Redis event-handler consumer if Redis is configured.
///
/// `None` means no consumer was started — the feature is switched off or Redis
/// is not reachable at startup — so there is nothing for the supervisor to own.
async fn spawn_event_handler(
    config: &Config,
    app_state: &AppState,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    if !config.redis.event_handler_enabled {
        info!("Event handler consumer disabled (EVENT_HANDLER_ENABLED=false)");
        return None;
    }
    let redis_url = config.redis.url.as_ref()?;

    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to create Redis client for event handler");
            return None;
        }
    };
    let conn = match redis::aio::ConnectionManager::new_with_config(
        client,
        moatless_core::events::redis_manager_config(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to connect Redis for event handler");
            return None;
        }
    };

    let state = app_state.clone();
    let handle = tokio::spawn(event_handler::spawn_event_handler(conn, state, cancel));
    info!("Event handler consumer spawned");
    Some(handle)
}

/// Spawn the desired-state reconciler loop. On Kubernetes `leader` gates each
/// tick to the elected replica; on Docker `leader` is `None` and the loop runs
/// unconditionally (single replica).
fn spawn_sandbox_reconciler(
    reconciler: sandbox::lifecycle::Reconciler,
    leader: Option<Arc<dyn LeaderGuard>>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                _ = ticker.tick() => {
                    if let Some(l) = &leader {
                        match l.try_acquire_or_renew().await {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(e) => {
                                tracing::warn!(error = %e, "reconciler: leader check failed; skipping tick");
                                continue;
                            }
                        }
                    }
                    reconciler.reconcile_once().await;
                }
            }
        }
        info!("Sandbox reconciler stopped");
    });
    info!("Sandbox reconciler spawned");
    handle
}

/// Spawn the unified task reaper (idle-sandbox stop + inactive-task close) if
/// enabled. `None` when the reaper is switched off.
fn spawn_task_reaper(
    app_state: &AppState,
    agent_status: Arc<dyn crate::task::reaper::AgentStatusPort>,
    k8s: sandbox::k8s::client::K8sClient,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    if !app_state.config.sandbox.idle_reaper_enabled {
        info!("Task reaper disabled");
        return None;
    }
    let agent_error_sink: Arc<dyn crate::task::reaper::AgentErrorSink> =
        Arc::new(app_state.clone());
    // Build the reaper's leader guard at the composition root; the reaper depends
    // only on the `LeaderGuard` port, not on K8s. Lease duration is 2x the check
    // interval so the lease doesn't expire between cycles.
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let lease_duration_secs = (app_state.config.sandbox.idle_check_interval_secs * 2) as i32;
    let election = sandbox::k8s::leader_election::LeaderElection::new_with_name(
        &k8s,
        identity,
        "moatless-task-reaper".to_string(),
    )
    .with_lease_duration(lease_duration_secs);
    let leader: Arc<dyn LeaderGuard> = Arc::new(K8sLeaderGuard(election));
    let sandbox_resources: Arc<dyn crate::task::reaper::SandboxResources> = Arc::new(
        sandbox::k8s::sandbox_resources::K8sSandboxResources::new(k8s, app_state.tasks.clone()),
    );
    let handle = task::reaper::TaskReaper::new(
        agent_status,
        leader,
        app_state.messages.clone(),
        app_state.tasks.clone(),
        app_state.task_turns.clone(),
        app_state.task_events.clone(),
        app_state.sandbox.clone(),
        app_state.task_bindings.clone(),
        app_state.events.clone(),
        app_state.config.clone(),
        agent_error_sink,
        sandbox_resources,
    )
    .spawn(cancel);
    info!(
        idle_timeout_minutes = app_state.config.sandbox.idle_timeout_minutes,
        check_interval_secs = app_state.config.sandbox.idle_check_interval_secs,
        inactive_task_timeout_days = app_state.config.sandbox.inactive_task_timeout_days,
        "Task reaper spawned"
    );
    Some(handle)
}

/// Spawn the background Codex token refresher if enabled. Leader-elected via
/// a dedicated K8s Lease (`moatless-codex-refresher`). `None` when disabled.
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
    // Build the K8s leader guard at the composition root; the refresher depends
    // only on the `LeaderGuard` port, not on K8s.
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let lease_duration_secs: i32 = (interval.as_secs() * 2).try_into().unwrap_or(i32::MAX);
    let election = sandbox::k8s::leader_election::LeaderElection::new_with_name(
        &k8s,
        identity,
        "moatless-codex-refresher".to_string(),
    )
    .with_lease_duration(lease_duration_secs);
    let leader_guard: Arc<dyn LeaderGuard> = Arc::new(K8sLeaderGuard(election));
    let handle = moatless_vibe_backend::credential_sync::refresher::spawn_codex_refresher(
        app_state.db.clone(),
        Arc::clone(&app_state.credential_sync),
        leader_guard,
        interval,
        cancel,
    );
    info!(interval_secs, "Codex refresher spawned");
    Some(handle)
}
