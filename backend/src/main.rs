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

use moatless_vibe_backend::auth;
use moatless_vibe_backend::channel;
use moatless_vibe_backend::config::{Config, SandboxProviderType};
use moatless_vibe_backend::event_handler;
use moatless_vibe_backend::feature_flags;
use moatless_vibe_backend::leader::{K8sLeaderGuard, LeaderGuard};
use moatless_vibe_backend::sandbox;
use moatless_vibe_backend::sse::routes::sse_router;
use moatless_vibe_backend::startup;
use moatless_vibe_backend::state::AppState;
use moatless_vibe_backend::{
    command, figma, file_storage, message, plugin_repo, provider_settings, repository, skill, tag,
    task, task_event, team,
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
    )
    .await;
    let event_publisher = startup::init_event_publisher(&config).await;
    let cancel = CancellationToken::new();
    let sse_state = startup::init_sse(&config, cancel.clone()).await?;
    let s3_client = startup::init_s3(&config).await;
    startup::install_shutdown_handler(cancel.clone());
    startup::spawn_grpc_server(&config, db_pool.as_ref(), event_publisher.clone());

    // Build sandbox provider based on configured provider type
    let sandbox_provider: Arc<dyn sandbox::provider::SandboxProvider>;
    let reaper_k8s: Option<sandbox::k8s::client::K8sClient>;

    match config.sandbox_provider {
        SandboxProviderType::Kubernetes => {
            let k8s =
                sandbox::k8s::client::K8sClient::new(config.sandbox.namespace.clone()).await?;
            info!("K8s client initialized");

            reaper_k8s = Some(k8s.clone());
            sandbox_provider =
                startup::build_sandbox_provider(&config, k8s.clone(), event_publisher.clone());

            // Spawn pod lifecycle watcher
            if let Some(ref publisher) = event_publisher {
                sandbox::k8s::pod_watcher::spawn_pod_lifecycle_watcher(
                    k8s,
                    publisher.clone(),
                    cancel.clone(),
                    config.sandbox.max_crash_restart_count,
                );
                info!(
                    max_crash_restart_count = config.sandbox.max_crash_restart_count,
                    "Pod lifecycle watcher spawned"
                );
            } else {
                info!("Pod lifecycle watcher disabled (requires Redis)");
            }
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

    let sandbox_state = sandbox::routes::SandboxApiState {
        provider: Arc::clone(&sandbox_provider),
        tasks: sandbox_tasks,
        repos: sandbox_repos,
        provider_tokens: sandbox_provider_tokens,
        plugin_repos: sandbox_plugin_repos,
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
        let channel_providers =
            channel::provider::ChannelProviders::from_config(&config, pool.clone());
        let app_state = AppState::new(
            pool,
            Arc::new(config.clone()),
            sandbox_provider,
            event_publisher,
            s3_client,
            channel_providers.map(Arc::new),
        );

        let modules = vec![
            task::routes::task_router(app_state.clone()),
            message::routes::message_router(app_state.clone()),
            command::routes::command_router(app_state.clone()),
            file_storage::routes::file_storage_router(app_state.clone()),
            figma::routes::figma_router(app_state.clone()),
            repository::routes::repository_router(app_state.clone()),
            tag::routes::tag_router(app_state.clone()),
            provider_settings::routes::provider_settings_router(app_state.clone()),
            channel::routes::channel_router(app_state.clone()),
            team::routes::team_router(app_state.clone()),
            task_event::routes::task_event_router(app_state.clone()),
            plugin_repo::routes::plugin_repo_router(app_state.clone()),
            skill::routes::skill_router(app_state.clone()),
        ];
        for (mod_router, mod_api) in modules {
            router = router.merge(mod_router);
            api.merge(mod_api);
        }
        info!("Domain routes registered");

        // Seed default repositories
        repository::seed::seed_default_repositories(&*app_state.repositories).await;

        // Ensure local-user exists in dev mode (no auth provider)
        if config.auth.provider.is_none() {
            auth::local_user::ensure_local_user(app_state.users.as_ref()).await;
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

        if let Some(k8s) = reaper_k8s {
            spawn_idle_reaper(&app_state, k8s, cancel.clone());
        } else {
            info!("Idle sandbox reaper disabled (Docker mode — K8s leader election not available)");
        }

        // Cron scheduler (if channel providers are configured)
        if let Some(ref providers) = app_state.channel_providers {
            channel::cron::scheduler::spawn_cron_scheduler(
                app_state.clone(),
                cron_leader,
                cancel.clone(),
            );

            // Telegram long-polling (when webhook mode is not available)
            if config.telegram.bot_token.is_some() && config.telegram.mode == "polling" {
                channel::telegram::poller::spawn_telegram_poller(
                    app_state.clone(),
                    providers.clone(),
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

    // Apply auth middleware (outermost layer — runs first on every request)
    let router = if let Some(mw_state) = auth_middleware_state {
        info!("Auth middleware enabled");
        router
            .layer(axum::middleware::from_fn(auth::middleware::auth_middleware))
            .layer(axum::Extension(mw_state))
    } else {
        info!("Auth middleware disabled (no database)");
        router
    };

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

/// Spawn the Redis event-handler consumer if Redis is configured.
async fn spawn_event_handler(config: &Config, app_state: &AppState, cancel: CancellationToken) {
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
    let conn = match redis::aio::ConnectionManager::new(client).await {
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

/// Spawn the idle-sandbox reaper if enabled.
fn spawn_idle_reaper(
    app_state: &AppState,
    k8s: sandbox::k8s::client::K8sClient,
    cancel: CancellationToken,
) {
    if !app_state.config.sandbox.idle_reaper_enabled {
        info!("Idle sandbox reaper disabled");
        return;
    }
    sandbox::idle_reaper::spawn_idle_reaper(
        k8s,
        app_state.messages.clone(),
        app_state.tasks.clone(),
        Arc::clone(&app_state.sandbox),
        Arc::clone(&app_state.config),
        cancel,
    );
    info!(
        idle_timeout_minutes = app_state.config.sandbox.idle_timeout_minutes,
        check_interval_secs = app_state.config.sandbox.idle_check_interval_secs,
        "Idle sandbox reaper spawned"
    );
}
