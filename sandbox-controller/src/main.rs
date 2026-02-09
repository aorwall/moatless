//! Sandbox Controller - Workspace orchestration for Moatless Vibe
//!
//! Serves a unified API on port 8080. Authenticated via Traefik ForwardAuth.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use sandbox_controller::auth;
use sandbox_controller::config::Config;
use sandbox_controller::db;
use sandbox_controller::event_handler;
use sandbox_controller::events::RedisEventPublisher;
use sandbox_controller::feature_flags;
use sandbox_controller::metrics;
use sandbox_controller::sandbox;
use sandbox_controller::sse;
use sandbox_controller::state::AppState;
use sandbox_controller::{agent, command, figma, file_storage, message, provider_settings, repository, tag, task};

use auth::cache::AuthCache;
use auth::routes::{AuthState, OAuthProviders};
use sandbox::k8s::client::K8sClient;
use sandbox::k8s::status_cache::StatusCache;
use sse::consumer::RedisStreamConsumer;
use sse::routes::{sse_router, SseState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Handle --dump-openapi before any runtime initialization
    if std::env::args().any(|a| a == "--dump-openapi") {
        dump_openapi()?;
        return Ok(());
    }

    // Install rustls crypto provider (required before any TLS operations)
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    // Load configuration
    let config = Config::from_env();
    info!(
        sandbox_namespace = %config.sandbox_namespace,
        "Starting Sandbox Controller"
    );

    // Initialize metrics
    let metrics_handle = metrics::init_metrics();

    // Create metrics router
    let metrics_router = metrics::metrics_router(metrics_handle);

    // Start metrics server
    let metrics_port = config.metrics_port;
    let metrics_addr = SocketAddr::from(([0, 0, 0, 0], metrics_port));
    tokio::spawn(async move {
        let listener = TcpListener::bind(metrics_addr).await.unwrap();
        info!(port = metrics_port, "Metrics server started");
        axum::serve(listener, metrics_router).await.unwrap();
    });

    // Optionally initialize database pool
    let db_pool = if let Some(ref database_url) = config.database_url {
        match db::create_pool(database_url).await {
            Ok(pool) => {
                info!("Database pool initialized (auto-provisioning enabled)");
                Some(pool)
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to create database pool — auto-provisioning disabled");
                None
            }
        }
    } else {
        info!("DATABASE_URL not set — auto-provisioning on /start disabled");
        None
    };

    // Optionally initialize Redis event publisher
    let event_publisher = if let Some(ref redis_url) = config.redis_url {
        RedisEventPublisher::connect(redis_url, config.redis_max_stream_length).await
    } else {
        info!("REDIS_URL not set — environment event publishing disabled");
        None
    };

    // Create broadcast channel for SSE fan-out
    let (sse_tx, _) = broadcast::channel::<String>(config.sse_broadcast_capacity);

    // Graceful-shutdown token
    let cancel = CancellationToken::new();

    // Spawn SSE Redis consumer if Redis is configured
    if let Some(ref redis_url) = config.redis_url {
        let client = redis::Client::open(redis_url.as_str())?;
        let conn = redis::aio::ConnectionManager::new(client).await?;
        let consumer = RedisStreamConsumer::new(
            conn,
            config.sse_consumer_id.clone(),
            sse_tx.clone(),
            cancel.clone(),
        );
        tokio::spawn(consumer.run());
        info!(consumer_id = %config.sse_consumer_id, "SSE Redis consumer spawned");
    } else {
        info!("REDIS_URL not set — SSE Redis consumer disabled");
    }

    let sse_state = SseState {
        broadcast: sse_tx,
        cancel: cancel.clone(),
    };

    // Optionally initialize S3 client
    let s3_client = if config.s3_bucket_name.is_some() {
        let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(config.s3_region.clone()))
            .load()
            .await;
        let client = aws_sdk_s3::Client::new(&aws_config);
        info!("S3 client initialized");
        Some(client)
    } else {
        info!("S3_BUCKET_NAME not set — file uploads disabled");
        None
    };

    // Initialize K8s client and start API server
    let k8s = K8sClient::new(config.sandbox_namespace.clone()).await?;
    let status_cache = StatusCache::new(10);
    info!("K8s client initialized");

    // Install graceful shutdown handler
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Received shutdown signal, cancelling background tasks");
        cancel_for_signal.cancel();
    });

    start_api_server(
        &config,
        k8s,
        status_cache,
        db_pool,
        event_publisher,
        sse_state,
        s3_client,
        cancel,
    )
    .await?;

    Ok(())
}

/// Print the merged OpenAPI spec as JSON to stdout and exit.
fn dump_openapi() -> Result<(), Box<dyn std::error::Error>> {
    let merged = build_merged_openapi();
    println!("{}", merged.to_pretty_json()?);
    Ok(())
}

/// Build the merged OpenAPI spec from all API doc structs.
///
/// All `*ApiDoc` structs have their paths registered via `#[openapi(paths(...))]`,
/// so we can build the full spec without constructing any routers or providing real state.
fn build_merged_openapi() -> utoipa::openapi::OpenApi {
    use sandbox::routes::SandboxApiDoc;
    use auth::routes::AuthApiDoc;
    use task::routes::TaskApiDoc;
    use message::routes::MessageApiDoc;
    use agent::routes::AgentApiDoc;
    use command::routes::CommandApiDoc;
    use file_storage::routes::FileStorageApiDoc;
    use figma::routes::FigmaApiDoc;
    use repository::routes::RepositoryApiDoc;
    use tag::routes::TagApiDoc;
    use provider_settings::routes::ProviderSettingsApiDoc;
    use utoipa::OpenApi;

    let mut merged = SandboxApiDoc::openapi();
    merged.merge(AuthApiDoc::openapi());
    merged.merge(TaskApiDoc::openapi());
    merged.merge(MessageApiDoc::openapi());
    merged.merge(AgentApiDoc::openapi());
    merged.merge(CommandApiDoc::openapi());
    merged.merge(FileStorageApiDoc::openapi());
    merged.merge(FigmaApiDoc::openapi());
    merged.merge(RepositoryApiDoc::openapi());
    merged.merge(TagApiDoc::openapi());
    merged.merge(ProviderSettingsApiDoc::openapi());

    merged.info = utoipa::openapi::InfoBuilder::new()
        .title("Sandbox Controller API")
        .version(env!("CARGO_PKG_VERSION"))
        .description(Some("Workspace orchestration and authentication for Moatless Vibe"))
        .build();

    merged
}

/// Build OAuth providers from config.
fn build_oauth_providers(config: &Config) -> OAuthProviders {
    let github = auth::oauth::github::GitHubOAuthProvider::from_config(config).map(|p| {
        info!("GitHub OAuth provider configured");
        Arc::new(p) as Arc<dyn auth::oauth::provider::OAuthProvider>
    });

    let auth0 = auth::oauth::auth0::Auth0OAuthProvider::from_config(config).map(|p| {
        info!("Auth0 OAuth provider configured");
        Arc::new(p) as Arc<dyn auth::oauth::provider::OAuthProvider>
    });

    OAuthProviders { github, auth0 }
}

/// Start the API server on the configured port.
async fn start_api_server(
    config: &Config,
    k8s: K8sClient,
    status_cache: StatusCache,
    db_pool: Option<db::DbPool>,
    event_publisher: Option<RedisEventPublisher>,
    sse_state: SseState,
    s3_client: Option<aws_sdk_s3::Client>,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    use sandbox::k8s::provider::K8sSandboxProvider;
    use sandbox::routes::{api_router, SandboxApiState};

    // Load pod template
    let template_path = config
        .sandbox_pod_template_path
        .as_deref()
        .ok_or("SANDBOX_POD_TEMPLATE_PATH not configured")?;

    let pod_template = tokio::fs::read_to_string(template_path)
        .await
        .map_err(|e| format!("Failed to read pod template from {template_path}: {e}"))?;

    info!(template_path, "Pod template loaded");

    let provider = K8sSandboxProvider {
        config: Arc::new(config.clone()),
        k8s,
        status_cache,
        pod_template: Arc::new(pod_template),
        db_pool: db_pool.clone(),
        event_publisher: event_publisher.clone(),
    };

    let sandbox_provider: Arc<dyn sandbox::provider::SandboxProvider> = Arc::new(provider);

    let sandbox_state = SandboxApiState {
        provider: Arc::clone(&sandbox_provider),
    };

    // Build sandbox router with OpenAPI
    let (mut router, mut merged_api) = api_router(sandbox_state);

    // Add auth routes if database is available
    if let Some(ref pool) = db_pool {
        let oauth_providers = build_oauth_providers(config);
        let auth_cache = AuthCache::new(config.auth_cache_ttl_secs);
        let auth_state = AuthState {
            db: pool.clone(),
            config: Arc::new(config.clone()),
            providers: Arc::new(oauth_providers),
            cache: auth_cache,
        };
        let (auth_router, auth_api) = auth::routes::auth_router(auth_state);
        router = router.merge(auth_router);
        merged_api.merge(auth_api);
        info!(
            auth_provider = ?config.auth_provider,
            auth_cache_ttl_secs = config.auth_cache_ttl_secs,
            "Auth routes registered"
        );
    } else {
        info!("Auth routes disabled (no database)");
    }

    // Add new API routes if database is available
    if let Some(pool) = db_pool {
        let app_state = AppState {
            db: pool,
            config: Arc::new(config.clone()),
            sandbox: sandbox_provider,
            events: event_publisher,
            s3: s3_client,
        };

        let modules = vec![
            task::routes::task_router(app_state.clone()),
            message::routes::message_router(app_state.clone()),
            agent::routes::agent_router(app_state.clone()),
            command::routes::command_router(app_state.clone()),
            file_storage::routes::file_storage_router(app_state.clone()),
            figma::routes::figma_router(app_state.clone()),
            repository::routes::repository_router(app_state.clone()),
            tag::routes::tag_router(app_state.clone()),
            provider_settings::routes::provider_settings_router(app_state.clone()),
        ];
        for (module_router, module_api) in modules {
            router = router.merge(module_router);
            merged_api.merge(module_api);
        }

        info!("Task/message/agent/command/file/figma/repository/tag/provider routes registered");

        // Spawn event handler consumer if Redis is available
        if let Some(ref redis_url) = config.redis_url {
            match redis::Client::open(redis_url.as_str()) {
                Ok(client) => match redis::aio::ConnectionManager::new(client).await {
                    Ok(conn) => {
                        let handler_state = app_state;
                        let handler_cancel = cancel.clone();
                        tokio::spawn(event_handler::spawn_event_handler(
                            conn,
                            handler_state,
                            handler_cancel,
                        ));
                        info!("Event handler consumer spawned");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to connect Redis for event handler");
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to create Redis client for event handler");
                }
            }
        }
    } else {
        info!("New API routes disabled (no database)");
    }

    // Feature flags route (no database required)
    router = router.route(
        "/api/v1/feature-flags",
        axum::routing::get(feature_flags::get_feature_flags),
    );

    // Add SSE routes
    router = router.merge(sse_router(sse_state));
    info!("SSE routes registered");

    // Serve OpenAPI spec at /openapi.json
    let openapi_json = merged_api.clone();
    let router = router.route(
        "/openapi.json",
        axum::routing::get(move || async move { Json(openapi_json) }),
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    info!(port = config.port, "API server started");

    axum::serve(listener, router).await?;

    Ok(())
}
