//! Moatless Vibe Backend — entry point.
//!
//! Orchestrates startup by delegating to focused helpers in [`startup`],
//! factory constructors on state structs, and per-module router builders.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

use moatless_vibe_backend::agent;
use moatless_vibe_backend::auth;
use moatless_vibe_backend::config::{Config, SandboxProviderType};
use moatless_vibe_backend::event_handler;
use moatless_vibe_backend::feature_flags;
use moatless_vibe_backend::leader::{K8sLeaderGuard, LeaderGuard};
use moatless_vibe_backend::sandbox;
use moatless_vibe_backend::sse::routes::sse_router;
use moatless_vibe_backend::startup;
use moatless_vibe_backend::state::AppState;
use moatless_vibe_backend::{
    adapters, feedback, file_storage, message, plugin_repo, provider_settings, repository, secret,
    skill, tag, task, task_event, team, triggers, workspace,
};

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
        startup::init_tracing();
        return secret::cli::run(secret::cli::SecretsCli { command: cmd }).await;
    }

    // Install rustls crypto provider (required before any TLS operations)
    #[allow(clippy::expect_used)]
    // fail-fast: TLS provider must be installed before any networking
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    startup::init_tracing();

    let config = Config::from_env()?;
    info!(sandbox_namespace = %config.sandbox.namespace, "Starting Moatless Vibe Backend");

    // Infrastructure services
    startup::spawn_metrics_server(config.server.metrics_port);
    let db_pool = startup::init_database(
        config.database_url.as_deref(),
        config.database.max_connections,
        config.database.run_migrations,
    )
    .await;
    let event_publisher = startup::init_event_publisher(&config).await;
    let cancel = CancellationToken::new();
    let sse_state = startup::init_sse(&config, db_pool.as_ref(), cancel.clone()).await?;
    let s3_client = startup::init_s3(&config).await;
    startup::install_shutdown_handler(cancel.clone());
    startup::spawn_grpc_server(
        &config,
        db_pool.as_ref(),
        event_publisher.clone(),
        s3_client.clone().map(std::sync::Arc::new),
    );

    // Build sandbox provider based on configured provider type
    let sandbox_provider: Arc<dyn sandbox::provider::SandboxProvider>;
    let reaper_k8s: Option<sandbox::k8s::client::K8sClient>;
    // The reconciler's per-tick fleet observation, built per provider.
    let sandbox_fleet_observer: Arc<dyn sandbox::lifecycle::FleetObserver>;

    match config.sandbox_provider {
        SandboxProviderType::Kubernetes => {
            let k8s =
                sandbox::k8s::client::K8sClient::new(config.sandbox.namespace.clone()).await?;
            info!("K8s client initialized");

            reaper_k8s = Some(k8s.clone());
            sandbox_provider = startup::build_sandbox_provider(
                &config,
                k8s.clone(),
                event_publisher.clone(),
                s3_client.clone().map(std::sync::Arc::new),
            );
            sandbox_fleet_observer = Arc::new(sandbox::lifecycle::K8sFleetObserver::new(
                k8s.clone(),
                sandbox_provider.clone(),
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
            sandbox_provider = startup::build_docker_sandbox_provider(
                &config,
                docker_client.clone(),
                event_publisher.clone(),
            );
            sandbox_fleet_observer = Arc::new(sandbox::lifecycle::DockerFleetObserver::new(
                docker_client.clone(),
                sandbox_provider.clone(),
            ));

            // Spawn container lifecycle watcher
            if let Some(ref publisher) = event_publisher {
                sandbox::docker::container_watcher::spawn_container_lifecycle_watcher(
                    docker_client,
                    publisher.clone(),
                    cancel.clone(),
                    config.sandbox.max_crash_restart_count,
                );
                info!(
                    max_crash_restart_count = config.sandbox.max_crash_restart_count,
                    "Container lifecycle watcher spawned"
                );
            } else {
                info!("Container lifecycle watcher disabled (requires Redis)");
            }
        }
    }

    // ── Build router ────────────────────────────────────────────────────

    // Build optional repos for SandboxApiState (server config resolution in handlers)
    let sandbox_tasks: Option<Arc<dyn moatless_vibe_backend::task::dao::TaskRepository>> =
        db_pool.as_ref().map(|p| {
            Arc::new(moatless_vibe_backend::task::dao::PgTaskRepository(
                p.clone(),
            )) as Arc<dyn moatless_vibe_backend::task::dao::TaskRepository>
        });
    let sandbox_repos: Option<Arc<dyn moatless_vibe_backend::repository::dao::RepoRepository>> =
        db_pool.as_ref().map(|p| {
            Arc::new(moatless_vibe_backend::repository::dao::PgRepoRepository(
                p.clone(),
            )) as Arc<dyn moatless_vibe_backend::repository::dao::RepoRepository>
        });
    let sandbox_teams: Option<Arc<dyn moatless_vibe_backend::team::dao::TeamRepository>> =
        db_pool.as_ref().map(|p| {
            Arc::new(moatless_vibe_backend::team::dao::PgTeamRepository(
                p.clone(),
            )) as Arc<dyn moatless_vibe_backend::team::dao::TeamRepository>
        });
    let sandbox_provider_tokens: Option<
        Arc<dyn moatless_vibe_backend::auth::provider_token_dao::ProviderTokenRepository>,
    > = db_pool.as_ref().map(|p| {
        Arc::new(
            moatless_vibe_backend::auth::provider_token_dao::PgProviderTokenRepository(p.clone()),
        )
            as Arc<dyn moatless_vibe_backend::auth::provider_token_dao::ProviderTokenRepository>
    });
    let sandbox_plugin_repos: Option<
        Arc<dyn moatless_vibe_backend::plugin_repo::dao::PluginRepoRepository>,
    > = db_pool.as_ref().map(|p| {
        Arc::new(moatless_vibe_backend::plugin_repo::dao::PgPluginRepoRepository(p.clone()))
            as Arc<dyn moatless_vibe_backend::plugin_repo::dao::PluginRepoRepository>
    });
    let sandbox_workspaces: Option<
        Arc<dyn moatless_vibe_backend::workspace::dao::WorkspaceRepository>,
    > = db_pool.as_ref().map(|p| {
        Arc::new(moatless_vibe_backend::workspace::dao::PgWorkspaceRepository(
            p.clone(),
        )) as Arc<dyn moatless_vibe_backend::workspace::dao::WorkspaceRepository>
    });
    let sandbox_task_events: Option<
        Arc<dyn moatless_vibe_backend::task_event::dao::TaskEventRepository>,
    > = db_pool.as_ref().map(|p| {
        Arc::new(moatless_vibe_backend::task_event::dao::PgTaskEventRepository(p.clone()))
            as Arc<dyn moatless_vibe_backend::task_event::dao::TaskEventRepository>
    });
    let sandbox_secrets: Option<Arc<moatless_vibe_backend::secret::service::SecretService>> =
        db_pool.as_ref().map(|p| {
            let secret_records: Arc<dyn moatless_vibe_backend::secret::dao::SecretRepository> =
                Arc::new(moatless_vibe_backend::secret::dao::PgSecretRepository(
                    p.clone(),
                ));
            let secret_audits: Arc<
                dyn moatless_vibe_backend::secret::audit::SecretAuditRepository,
            > = Arc::new(moatless_vibe_backend::secret::audit::PgSecretAuditRepository(p.clone()));
            Arc::new(moatless_vibe_backend::secret::service::SecretService::new(
                secret_records,
                secret_audits,
                moatless_vibe_backend::secret::service::LocalEnvelopeSecretCipher::from_config(
                    &config.secrets,
                ),
            ))
        });

    // Build the shared durable Sandbox lifecycle handle once (requires the DB).
    // The sandbox API, the pod watcher, and AppState all drive transitions
    // through this single instance, so Sandbox intent and per-Task locks stay
    // single-sourced. The lock adapter needs a dedicated lock-only pool.
    let lifecycle: Option<Arc<moatless_vibe_backend::sandbox::lifecycle::Lifecycle>> =
        match db_pool.as_ref() {
            Some(pool) => {
                let db_url = config
                    .database_url
                    .as_deref()
                    .ok_or("DATABASE_URL required to build the lifecycle lock pool")?;
                let lock_pool = moatless_vibe_backend::db::create_pool_kind(
                    db_url,
                    config.database.max_connections,
                    moatless_vibe_backend::db::PoolKind::Lock,
                )
                .await?;
                Some(moatless_vibe_backend::state::build_lifecycle(
                    pool.clone(),
                    lock_pool,
                    Arc::clone(&sandbox_provider),
                    event_publisher.clone(),
                ))
            }
            None => None,
        };

    // Spawn the K8s pod lifecycle watcher now that the DB repos exist. It needs
    // task + repo access (plus credentials) to auto-disable crash-looping server
    // containers and redeploy without them. Only runs in K8s mode (reaper_k8s is
    // None under Docker) and when an event publisher (Redis) is available.
    match (reaper_k8s.as_ref(), event_publisher.as_ref()) {
        (Some(k8s), Some(publisher)) => {
            sandbox::k8s::pod_watcher::spawn_pod_lifecycle_watcher(
                k8s.clone(),
                publisher.clone(),
                cancel.clone(),
                config.sandbox.max_crash_restart_count,
                lifecycle.clone(),
                sandbox_tasks.clone(),
                sandbox_workspaces.clone(),
                sandbox_task_events.clone(),
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
        provider: Arc::clone(&sandbox_provider),
        lifecycle: lifecycle.clone(),
        tasks: sandbox_tasks,
        teams: sandbox_teams,
        repos: sandbox_repos,
        provider_tokens: sandbox_provider_tokens,
        plugin_repos: sandbox_plugin_repos,
        workspaces: sandbox_workspaces,
        secrets: sandbox_secrets,
    };
    let (mut router, mut api) = sandbox::routes::api_router(sandbox_state);

    // Auth routes (needs DB)
    let mut auth_middleware_state: Option<auth::middleware::AuthMiddlewareState> = None;
    if let Some(ref pool) = db_pool {
        let auth_state = auth::routes::AuthState::new(pool.clone(), Arc::new(config.clone()));

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

    // Domain routes (need DB)
    if let Some(pool) = db_pool {
        // Build the GitHub App registry once and share it between AppState and
        // the adapter layer so they hit the same token cache.
        let github_apps =
            Arc::new(crate::auth::github_app_registry::GitHubAppRegistry::from_config(&config));
        let adapter_registry =
            adapters::Adapters::from_config(&config, pool.clone(), github_apps.clone()).await;
        // The shared lifecycle was built above; it is present whenever the DB is.
        let lifecycle = lifecycle
            .ok_or("lifecycle handle required to build AppState (missing database pool)")?;
        let app_state = AppState::new(
            pool,
            lifecycle,
            Arc::new(config.clone()),
            sandbox_provider,
            event_publisher,
            s3_client,
            adapter_registry.map(Arc::new),
            github_apps,
        );

        let modules = vec![
            agent::routes::agent_router(app_state.clone()),
            task::routes::task_router(app_state.clone()),
            feedback::routes::feedback_router(app_state.clone()),
            message::routes::message_router(app_state.clone()),
            file_storage::routes::file_storage_router(app_state.clone()),
            repository::routes::repository_router(app_state.clone()),
            secret::routes::secret_router(app_state.clone()),
            tag::routes::tag_router(app_state.clone()),
            provider_settings::routes::provider_settings_router(app_state.clone()),
            adapters::routes::adapter_router(app_state.clone()),
            team::routes::team_router(app_state.clone()),
            task_event::routes::task_event_router(app_state.clone()),
            plugin_repo::routes::plugin_repo_router(app_state.clone()),
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
        info!("Domain routes registered");

        // Readiness probe — checks Redis connectivity. Distinct from /health
        // which is a pure liveness probe (process up). When Redis is dead,
        // task creation degrades silently because event publishes time out;
        // /ready surfaces that to Kubernetes so traffic is shed.
        let readiness_events = app_state.events.clone();
        router = router.route(
            "/ready",
            axum::routing::get(move || {
                let events = readiness_events.clone();
                async move {
                    use axum::http::StatusCode;
                    use axum::response::IntoResponse;
                    match events {
                        Some(publisher) => match publisher.health_check().await {
                            Ok(()) => (
                                StatusCode::OK,
                                axum::Json(serde_json::json!({"status": "ok"})),
                            )
                                .into_response(),
                            Err(e) => {
                                tracing::warn!(error = %e, "Readiness probe: event publisher unhealthy");
                                (
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    axum::Json(serde_json::json!({
                                        "status": "unavailable",
                                        "component": "events",
                                        "error": e
                                    })),
                                )
                                    .into_response()
                            }
                        },
                        None => (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({"status": "ok", "events": "disabled"})),
                        )
                            .into_response(),
                    }
                }
            }),
        );

        // Seed default repositories
        repository::seed::seed_default_repositories(&*app_state.repositories).await;

        auth::jwt::warn_on_jwt_secret_fallback(&config);

        // Seed the bootstrap admin for local password auth. Unlike the other
        // startup seeds this aborts on failure: a local-auth instance with no
        // admin credential is unreachable, and continuing would produce a
        // server nobody can log into.
        if config.auth.provider == moatless_vibe_backend::config::AuthProvider::Local {
            let credentials = moatless_vibe_backend::auth::password::dao::PgUserCredentialRepository(
                app_state.db.clone(),
            );
            auth::password::bootstrap::ensure_local_admin(
                app_state.users.as_ref(),
                &credentials,
                &config.auth,
            )
            .await
            .map_err(|e| format!("failed to seed the local admin account: {e}"))?;
        }

        // Ensure test API user + key (if configured)
        if let Some(ref key) = config.auth.test_api_key {
            let api_keys =
                moatless_vibe_backend::auth::api_keys::PgApiKeyRepository(app_state.db.clone());
            auth::test_user::ensure_test_user(app_state.users.as_ref(), &api_keys, key).await;
        }

        // Background workers
        spawn_event_handler(&config, &app_state, cancel.clone()).await;

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
        let reconciler = moatless_vibe_backend::state::build_reconciler(
            app_state.db.clone(),
            app_state.lifecycle.clone(),
            sandbox_fleet_observer,
            sandbox::lifecycle::ReconcilerConfig::default(),
        );
        spawn_sandbox_reconciler(reconciler, reconciler_leader, cancel.clone());

        if let Some(k8s) = reaper_k8s {
            spawn_task_reaper(&app_state, k8s.clone(), cancel.clone());
            spawn_codex_refresher(&app_state, k8s, cancel.clone());
        } else {
            info!("Task reaper disabled (Docker mode — K8s leader election not available)");
        }

        // Schedule runner and long-polling adapters (when configured).
        if let Some(ref adapter_registry) = app_state.adapters {
            triggers::schedule_runner::spawn_cron_scheduler(
                app_state.clone(),
                cron_leader,
                cancel.clone(),
            );

            // Telegram long-polling (when webhook mode is not available)
            if config.telegram.bot_token.is_some() && config.telegram.mode == "polling" {
                adapters::telegram::poller::spawn_telegram_poller(
                    app_state.clone(),
                    adapter_registry.clone(),
                    cancel.clone(),
                );
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
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    let listener = TcpListener::bind(addr).await?;
    info!(port = config.server.port, "API server started");
    axum::serve(listener, router).await?;

    Ok(())
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
            tracing::info!("request completed");
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
async fn spawn_event_handler(config: &Config, app_state: &AppState, cancel: CancellationToken) {
    if !config.redis.event_handler_enabled {
        info!("Event handler consumer disabled (EVENT_HANDLER_ENABLED=false)");
        return;
    }
    let Some(ref redis_url) = config.redis.url else {
        return;
    };

    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to create Redis client for event handler");
            return;
        }
    };
    let conn = match redis::aio::ConnectionManager::new_with_config(
        client,
        moatless_vibe_backend::events::redis_manager_config(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to connect Redis for event handler");
            return;
        }
    };

    let state = app_state.clone();
    tokio::spawn(event_handler::spawn_event_handler(conn, state, cancel));
    info!("Event handler consumer spawned");
}

/// Spawn the desired-state reconciler loop. On Kubernetes `leader` gates each
/// tick to the elected replica; on Docker `leader` is `None` and the loop runs
/// unconditionally (single replica).
fn spawn_sandbox_reconciler(
    reconciler: sandbox::lifecycle::Reconciler,
    leader: Option<Arc<dyn LeaderGuard>>,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
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
}

/// Spawn the unified task reaper (idle-sandbox stop + inactive-task close) if enabled.
fn spawn_task_reaper(
    app_state: &AppState,
    k8s: sandbox::k8s::client::K8sClient,
    cancel: CancellationToken,
) {
    if !app_state.config.sandbox.idle_reaper_enabled {
        info!("Task reaper disabled");
        return;
    }
    task::reaper::spawn_task_reaper(
        app_state.clone(),
        k8s,
        app_state.messages.clone(),
        app_state.tasks.clone(),
        app_state.events.clone(),
        Arc::clone(&app_state.config),
        cancel,
    );
    info!(
        idle_timeout_minutes = app_state.config.sandbox.idle_timeout_minutes,
        check_interval_secs = app_state.config.sandbox.idle_check_interval_secs,
        inactive_task_timeout_days = app_state.config.sandbox.inactive_task_timeout_days,
        "Task reaper spawned"
    );
}

/// Spawn the background Codex token refresher if enabled. Leader-elected via
/// a dedicated K8s Lease (`moatless-codex-refresher`).
fn spawn_codex_refresher(
    app_state: &AppState,
    k8s: sandbox::k8s::client::K8sClient,
    cancel: CancellationToken,
) {
    let interval_secs = app_state.config.sandbox.codex_refresher_interval_secs;
    if interval_secs == 0 {
        info!("Codex refresher disabled (CODEX_REFRESHER_INTERVAL_SECS=0)");
        return;
    }
    moatless_vibe_backend::credentials::codex::refresher::spawn_codex_refresher(
        app_state.db.clone(),
        Arc::clone(&app_state.credentials_svc),
        k8s,
        std::time::Duration::from_secs(interval_secs),
        cancel,
    );
    info!(interval_secs, "Codex refresher spawned");
}
