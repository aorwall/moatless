//! Application startup helpers.
//!
//! Each function initialises one infrastructure concern (logging, metrics,
//! database, Redis, S3, K8s, gRPC, SSE) so that `main()` reads as a short
//! sequential recipe rather than a single monolithic function.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
use crate::config::Config;
use crate::db::{self, DbPool};
use crate::events::{EventPublisher, RedisEventPublisher};
use crate::message::dao::{MessageRepository, PgMessageRepository};
use crate::metrics;
use crate::sandbox::k8s::client::K8sClient;
use crate::sandbox::grpc_server::BackendSandboxServiceImpl;
use crate::sse::consumer::RedisStreamConsumer;
use crate::sse::routes::SseState;

// =========================================================================
// Logging
// =========================================================================

/// Install JSON tracing subscriber with `RUST_LOG` env-filter support.
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();
}

// =========================================================================
// Metrics
// =========================================================================

/// Spawn the Prometheus metrics server on a dedicated port.
pub fn spawn_metrics_server(port: u16) {
    let handle = metrics::init_metrics();
    crate::events::describe_event_metrics();
    let router = metrics::metrics_router(handle);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tokio::spawn(async move {
        #[allow(clippy::expect_used)] // fail-fast: if metrics port is unavailable, crash early
        let listener = TcpListener::bind(addr)
            .await
            .expect("Failed to bind metrics server — is the port already in use?");
        info!(port, "Metrics server started");
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!(error = %e, "Metrics server failed");
        }
    });
}

// =========================================================================
// Database
// =========================================================================

/// Optionally create a Postgres pool and run migrations.
///
/// Returns `None` when `DATABASE_URL` is unset or connection fails.
pub async fn init_database(database_url: Option<&str>, max_connections: u32) -> Option<DbPool> {
    let url = if let Some(u) = database_url { u } else {
        info!("DATABASE_URL not set — database features disabled");
        return None;
    };

    match db::create_pool(url, max_connections).await {
        Ok(pool) => {
            info!("Database pool initialized");
            if let Err(e) = db::run_migrations(&pool).await {
                tracing::error!(error = %e, "Database migration failed");
                std::process::exit(1);
            }
            info!("Database migrations complete");
            Some(pool)
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to create database pool");
            None
        }
    }
}

// =========================================================================
// Redis event publisher
// =========================================================================

/// Optionally connect a Redis-backed event publisher.
pub async fn init_event_publisher(config: &Config) -> Option<Arc<dyn EventPublisher>> {
    if let Some(ref url) = config.redis.url { RedisEventPublisher::connect(url, config.redis.max_stream_length)
    .await
    .map(|p| Arc::new(p) as Arc<dyn EventPublisher>) } else {
        info!("REDIS_URL not set — event publishing disabled");
        None
    }
}

// =========================================================================
// SSE
// =========================================================================

/// Create the SSE broadcast channel and optionally spawn the Redis consumer.
pub async fn init_sse(
    config: &Config,
    cancel: CancellationToken,
) -> Result<SseState, redis::RedisError> {
    let (tx, _) = broadcast::channel::<String>(config.sse.broadcast_capacity);

    if let Some(ref redis_url) = config.redis.url {
        let client = redis::Client::open(redis_url.as_str())?;
        let conn = redis::aio::ConnectionManager::new(client).await?;
        let consumer = RedisStreamConsumer::new(
            conn,
            config.sse.consumer_id.clone(),
            tx.clone(),
            cancel.clone(),
        );
        tokio::spawn(consumer.run());
        info!(consumer_id = %config.sse.consumer_id, "SSE Redis consumer spawned");
    } else {
        info!("REDIS_URL not set — SSE Redis consumer disabled");
    }

    Ok(SseState {
        broadcast: tx,
        cancel,
    })
}

// =========================================================================
// S3
// =========================================================================

/// Optionally initialise the S3 client.
pub async fn init_s3(config: &Config) -> Option<aws_sdk_s3::Client> {
    if config.s3.bucket_name.is_none() {
        info!("S3_BUCKET_NAME not set — file uploads disabled");
        return None;
    }
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(config.s3.region.clone()))
        .load()
        .await;
    let client = aws_sdk_s3::Client::new(&aws_config);
    info!("S3 client initialized");
    Some(client)
}

// =========================================================================
// gRPC
// =========================================================================

/// Spawn the backend ↔ sandbox gRPC server.
pub fn spawn_grpc_server(
    config: &Config,
    db_pool: Option<&DbPool>,
    event_publisher: Option<Arc<dyn EventPublisher>>,
) {
    let messages: Option<Arc<dyn MessageRepository>> =
        db_pool.map(|pool| Arc::new(PgMessageRepository(pool.clone())) as _);
    let provider_tokens: Option<Arc<dyn ProviderTokenRepository>> =
        db_pool.map(|pool| Arc::new(PgProviderTokenRepository(pool.clone())) as _);
    let port = config.server.grpc_port;
    let token_secret = config.sandbox.task_token_secret.clone();

    tokio::spawn(async move {
        let svc = BackendSandboxServiceImpl {
            events: event_publisher,
            messages,
            provider_tokens,
            task_token_secret: token_secret,
        };
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!(port, "Starting backend gRPC server");
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(
                sandbox_proto::backend_sandbox_service_server::BackendSandboxServiceServer::new(
                    svc,
                ),
            )
            .serve(addr)
            .await
        {
            tracing::error!(error = %e, "gRPC server failed");
        }
    });
}

// =========================================================================
// Shutdown
// =========================================================================

/// Spawn a task that cancels the token on Ctrl-C.
pub fn install_shutdown_handler(cancel: CancellationToken) {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Received shutdown signal, cancelling background tasks");
        cancel.cancel();
    });
}

// =========================================================================
// K8s sandbox provider
// =========================================================================

/// Build the K8s sandbox provider with optional DB-backed repositories.
pub fn build_sandbox_provider(
    config: &Config,
    k8s: K8sClient,
    db_pool: Option<&DbPool>,
    event_publisher: Option<Arc<dyn EventPublisher>>,
) -> Arc<dyn crate::sandbox::provider::SandboxProvider> {
    use crate::auth::provider_token_dao::PgProviderTokenRepository;
    use crate::auth::users::PgUserRepository;
    use crate::repository::dao::PgRepoRepository;
    use crate::sandbox::k8s::provider::K8sSandboxProvider;
    use crate::sandbox::k8s::status_resolver::StatusResolver;
    use crate::task::dao::PgTaskRepository;

    let tasks: Option<Arc<dyn crate::task::dao::TaskRepository>> =
        db_pool.map(|p| Arc::new(PgTaskRepository(p.clone())) as _);
    let repos: Option<Arc<dyn crate::repository::dao::RepoRepository>> =
        db_pool.map(|p| Arc::new(PgRepoRepository(p.clone())) as _);
    let users: Option<Arc<dyn crate::auth::users::UserRepository>> =
        db_pool.map(|p| Arc::new(PgUserRepository(p.clone())) as _);
    let provider_tokens: Option<Arc<dyn crate::auth::provider_token_dao::ProviderTokenRepository>> =
        db_pool.map(|p| Arc::new(PgProviderTokenRepository(p.clone())) as _);

    let status_resolver = StatusResolver::new(
        k8s.clone(),
        Arc::new(config.clone()),
        tasks.clone(),
        repos.clone(),
    );

    Arc::new(K8sSandboxProvider {
        config: Arc::new(config.clone()),
        k8s,
        status_resolver,
        users,
        tasks,
        repos,
        provider_tokens,
        event_publisher,
    })
}

// =========================================================================
// OpenAPI
// =========================================================================

/// Build the merged OpenAPI spec from all API doc structs.
///
/// All `*ApiDoc` structs register their paths via `#[openapi(paths(...))]`,
/// so we can build the full spec without constructing any routers.
pub fn build_merged_openapi() -> utoipa::openapi::OpenApi {
    use crate::auth::routes::AuthApiDoc;
    use crate::channel::routes::ChannelApiDoc;
    use crate::command::routes::CommandApiDoc;
    use crate::figma::routes::FigmaApiDoc;
    use crate::file_storage::routes::FileStorageApiDoc;
    use crate::message::routes::MessageApiDoc;
    use crate::provider_settings::routes::ProviderSettingsApiDoc;
    use crate::repository::routes::RepositoryApiDoc;
    use crate::sandbox::routes::SandboxApiDoc;
    use crate::tag::routes::TagApiDoc;
    use crate::task::routes::TaskApiDoc;
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(
        components(schemas(
            moatless_events::EventType,
            moatless_events::SandboxSubType,
            moatless_events::ServerSubType,
            moatless_events::AgentSubType,
            moatless_events::TaskSubType,
            moatless_events::TaskTurnSubType,
            moatless_events::ServerEventPayload,
            moatless_events::StreamEvent,
        ))
    )]
    struct EventsApiDoc;

    let mut merged = SandboxApiDoc::openapi();
    merged.merge(AuthApiDoc::openapi());
    merged.merge(TaskApiDoc::openapi());
    merged.merge(MessageApiDoc::openapi());
    merged.merge(CommandApiDoc::openapi());
    merged.merge(FileStorageApiDoc::openapi());
    merged.merge(FigmaApiDoc::openapi());
    merged.merge(RepositoryApiDoc::openapi());
    merged.merge(TagApiDoc::openapi());
    merged.merge(ProviderSettingsApiDoc::openapi());
    merged.merge(ChannelApiDoc::openapi());
    merged.merge(EventsApiDoc::openapi());

    merged.info = utoipa::openapi::InfoBuilder::new()
        .title("Moatless Vibe Backend API")
        .version(env!("CARGO_PKG_VERSION"))
        .description(Some(
            "Workspace orchestration and authentication for Moatless Vibe",
        ))
        .build();

    merged
}
