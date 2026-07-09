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

use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
use crate::auth::provider_tokens::{PgProviderTokenAuthRepository, ProviderTokenAuthRepository};
use crate::auth::users::{PgUserRepository, UserRepository};
use crate::command::dao::{CommandRepository, PgCommandRepository};
use crate::config::Config;
use crate::credentials::{BroadcastContext, CredentialService};
use crate::db::{self, DbPool};
use crate::events::{EventPublisher, RedisEventPublisher};
use crate::file_storage::dao::{FileRepository, PgFileRepository};
use crate::message::dao::{MessageRepository, PgMessageRepository};
use crate::metrics;
use crate::repository::dao::{PgRepoRepository, RepoRepository};
use crate::sandbox::grpc_server::BackendSandboxServiceImpl;
use crate::sandbox::k8s::client::K8sClient;
use crate::secret::audit::{PgSecretAuditRepository, SecretAuditRepository};
use crate::secret::dao::{PgSecretRepository, SecretRepository};
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretService};
use crate::sse::consumer::RedisStreamConsumer;
use crate::sse::routes::SseState;
use crate::task::dao::{PgTaskRepository, TaskRepository};

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

// =========================================================================
// Metrics
// =========================================================================

/// Spawn the Prometheus metrics server on a dedicated port.
pub fn spawn_metrics_server(port: u16) {
    let handle = metrics::init_metrics();
    crate::events::describe_event_metrics();
    crate::adapters::metrics::describe_adapter_metrics();
    crate::triggers::metrics::describe_trigger_metrics();
    crate::bindings::metrics::describe_binding_metrics();
    crate::event_handler::metrics::describe_agent_metrics();
    crate::sandbox::metrics::describe_sandbox_metrics();
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
    let url = if let Some(u) = database_url {
        u
    } else {
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
    if let Some(ref url) = config.redis.url {
        RedisEventPublisher::connect(url, config.redis.max_stream_length)
            .await
            .map(|p| Arc::new(p) as Arc<dyn EventPublisher>)
    } else {
        info!("REDIS_URL not set — event publishing disabled");
        None
    }
}

// =========================================================================
// SSE
// =========================================================================

/// Create the SSE broadcast sender and optionally spawn the Redis consumer.
pub async fn init_sse(
    config: &Config,
    cancel: CancellationToken,
) -> Result<SseState, redis::RedisError> {
    let (tx, _) = broadcast::channel::<String>(config.sse.broadcast_capacity);

    let redis_conn = if let Some(ref redis_url) = config.redis.url {
        let client = redis::Client::open(redis_url.as_str())?;
        let conn = redis::aio::ConnectionManager::new_with_config(
            client,
            crate::events::redis_manager_config(),
        )
        .await?;
        let consumer = RedisStreamConsumer::new(
            conn.clone(),
            config.sse.consumer_id.clone(),
            tx.clone(),
            cancel.clone(),
        );
        tokio::spawn(consumer.run());
        info!(consumer_id = %config.sse.consumer_id, "SSE Redis consumer spawned");
        Some(conn)
    } else {
        info!("REDIS_URL not set — SSE Redis consumer disabled");
        None
    };

    Ok(SseState {
        broadcast: tx,
        cancel,
        redis: redis_conn,
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
    s3: Option<Arc<aws_sdk_s3::Client>>,
) {
    let messages: Option<Arc<dyn MessageRepository>> =
        db_pool.map(|pool| Arc::new(PgMessageRepository(pool.clone())) as _);
    let commands: Option<Arc<dyn CommandRepository>> =
        db_pool.map(|pool| Arc::new(PgCommandRepository(pool.clone())) as _);
    let files: Option<Arc<dyn FileRepository>> =
        db_pool.map(|pool| Arc::new(PgFileRepository(pool.clone())) as _);
    let provider_tokens: Option<Arc<dyn ProviderTokenRepository>> =
        db_pool.map(|pool| Arc::new(PgProviderTokenRepository(pool.clone())) as _);
    let tasks: Option<Arc<dyn TaskRepository>> =
        db_pool.map(|pool| Arc::new(PgTaskRepository(pool.clone())) as _);
    let task_relations: Option<Arc<dyn crate::task::relations::TaskRelationRepository>> =
        db_pool
            .map(|pool| Arc::new(crate::task::relations::PgTaskRelationRepository(pool.clone())) as _);
    let repositories: Option<Arc<dyn RepoRepository>> =
        db_pool.map(|pool| Arc::new(PgRepoRepository(pool.clone())) as _);
    let users: Option<Arc<dyn UserRepository>> =
        db_pool.map(|pool| Arc::new(PgUserRepository(pool.clone())) as _);
    let backend_config = Arc::new(config.clone());
    let port = config.server.grpc_port;
    let token_secret = config.sandbox.task_token_secret.clone();
    let s3_bucket = config.s3.bucket_name.clone();
    let s3_key_prefix = config.s3.key_prefix.clone();
    let transcript_s3_prefix = config.s3.transcript_prefix.clone();
    let workspace_s3_prefix = config.s3.workspace_prefix.clone();
    let github_apps = Arc::new(GitHubAppRegistry::from_config(config));
    let github_app_service = github_apps.default_service();
    let provider_token_auth: Option<Arc<dyn ProviderTokenAuthRepository>> =
        db_pool.map(|pool| Arc::new(PgProviderTokenAuthRepository(pool.clone())) as _);
    let secrets = db_pool.map(|pool| {
        let secret_records: Arc<dyn SecretRepository> = Arc::new(PgSecretRepository(pool.clone()));
        let secret_audits: Arc<dyn SecretAuditRepository> =
            Arc::new(PgSecretAuditRepository(pool.clone()));
        Arc::new(SecretService::new(
            secret_records,
            secret_audits,
            LocalEnvelopeSecretCipher::from_config(&config.secrets),
        ))
    });

    let codex_http = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "Failed to build reqwest client for Codex refresh, falling back to default"
                );
                reqwest::Client::new()
            }),
    );
    let credentials_svc = provider_tokens.as_ref().map(|tokens| {
        let mut svc = CredentialService::new(
            tokens.clone(),
            provider_token_auth.clone(),
            github_app_service.clone(),
            tasks.as_ref().map(|t| BroadcastContext {
                provider: config.sandbox_provider,
                sandbox_namespace: config.sandbox.namespace.clone(),
                sandbox_grpc_port: config.sandbox.grpc_port,
                tasks: t.clone(),
            }),
        );
        if let Some(pool) = db_pool {
            svc = svc.with_codex_refresh(pool.clone(), codex_http.clone());
        }
        Arc::new(svc)
    });

    tokio::spawn(async move {
        let svc = BackendSandboxServiceImpl {
            events: event_publisher,
            messages,
            commands,
            provider_tokens,
            tasks,
            task_relations,
            task_token_secret: token_secret,
            github_app_service,
            github_apps,
            provider_token_auth,
            secrets,
            credentials_svc,
            repositories,
            backend_config: Some(backend_config),
            s3,
            files,
            s3_bucket,
            s3_key_prefix,
            transcript_s3_prefix,
            workspace_s3_prefix,
            users,
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
// Sandbox providers
// =========================================================================

/// Build the K8s sandbox provider with optional DB-backed repositories.
///
/// The provider itself is pure infrastructure — no DB dependencies.
/// DB-backed repos are returned separately for use in `SandboxApiState`.
pub fn build_sandbox_provider(
    config: &Config,
    k8s: K8sClient,
    event_publisher: Option<Arc<dyn EventPublisher>>,
    s3: Option<Arc<aws_sdk_s3::Client>>,
) -> Arc<dyn crate::sandbox::provider::SandboxProvider> {
    use crate::sandbox::k8s::provider::K8sSandboxProvider;
    use crate::sandbox::k8s::status_resolver::StatusResolver;

    let status_resolver = StatusResolver::new(k8s.clone(), Arc::new(config.clone()));

    Arc::new(K8sSandboxProvider {
        config: Arc::new(config.clone()),
        k8s,
        status_resolver,
        event_publisher,
        s3,
    })
}

/// Build the Docker sandbox provider with optional DB-backed repositories.
///
/// The provider itself is pure infrastructure — no DB dependencies.
pub fn build_docker_sandbox_provider(
    config: &Config,
    client: crate::sandbox::docker::client::DockerClient,
    event_publisher: Option<Arc<dyn EventPublisher>>,
) -> Arc<dyn crate::sandbox::provider::SandboxProvider> {
    use crate::sandbox::docker::provider::DockerSandboxProvider;
    use crate::sandbox::docker::status_resolver::DockerStatusResolver;

    let status_resolver = DockerStatusResolver::new(client.clone(), Arc::new(config.clone()));

    Arc::new(DockerSandboxProvider {
        config: Arc::new(config.clone()),
        client,
        status_resolver,
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
    use crate::adapters::routes::AdapterApiDoc;
    use crate::agent::routes::AgentApiDoc;
    use crate::auth::routes::AuthApiDoc;
    use crate::command::routes::CommandApiDoc;
    use crate::feature_flags::{FeatureFlagsResponse, SlackConfig};
    use crate::feedback::routes::FeedbackApiDoc;
    use crate::figma::routes::FigmaApiDoc;
    use crate::file_storage::routes::FileStorageApiDoc;
    use crate::message::routes::MessageApiDoc;
    use crate::provider_settings::routes::ProviderSettingsApiDoc;
    use crate::repository::routes::RepositoryApiDoc;
    use crate::sandbox::routes::SandboxApiDoc;
    use crate::secret::routes::SecretApiDoc;
    use crate::skill::routes::SkillApiDoc;
    use crate::tag::routes::TagApiDoc;
    use crate::task::routes::TaskApiDoc;
    use crate::task_event::routes::TaskEventApiDoc;
    use crate::task_template::routes::TaskTemplateApiDoc;
    use crate::team::routes::TeamApiDoc;
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(
        components(schemas(
            // Event types + sub-types
            moatless_events::EventType,
            moatless_events::SandboxSubType,
            moatless_events::ServerSubType,
            moatless_events::AgentSubType,
            moatless_events::TaskSubType,
            moatless_events::TaskTurnSubType,
            // Event payloads
            moatless_events::ServerEventPayload,
            moatless_events::SandboxEventPayload,
            moatless_events::AgentEventPayload,
            moatless_events::TaskEventPayload,
            moatless_events::TaskTurnEventPayload,
            moatless_events::ResourcesEventPayload,
            moatless_events::StreamEvent,
            // Claude SDK types
            claude_agent_sdk::types::content::ContentBlock,
            claude_agent_sdk::types::content::ImageSource,
            claude_agent_sdk::types::content::ImageSourceType,
            claude_agent_sdk::types::content::ApiMessage,
            claude_agent_sdk::types::content::Usage,
            claude_agent_sdk::types::content::ModelUsage,
            claude_agent_sdk::types::content::PermissionDenial,
            claude_agent_sdk::types::messages::AssistantMessage,
            claude_agent_sdk::types::messages::AssistantMessageError,
            claude_agent_sdk::types::messages::ResultSuccess,
            claude_agent_sdk::types::messages::ResultError,
            // User-input types
            crate::message::schemas::QuestionOption,
            crate::message::schemas::Question,
            crate::message::schemas::AllowedPrompt,
            crate::message::schemas::AskUserQuestionInput,
            crate::message::schemas::ExitPlanModeInput,
            // Tool names
            crate::sandbox::schemas::MoatlessToolName,
        ))
    )]
    struct EventsApiDoc;

    #[derive(OpenApi)]
    #[openapi(
        paths(crate::feature_flags::get_feature_flags),
        components(schemas(FeatureFlagsResponse, SlackConfig))
    )]
    struct FeatureFlagsApiDoc;

    let mut merged = SandboxApiDoc::openapi();
    merged.merge(AgentApiDoc::openapi());
    merged.merge(AuthApiDoc::openapi());
    merged.merge(TaskApiDoc::openapi());
    merged.merge(MessageApiDoc::openapi());
    merged.merge(CommandApiDoc::openapi());
    merged.merge(FeedbackApiDoc::openapi());
    merged.merge(FileStorageApiDoc::openapi());
    merged.merge(FigmaApiDoc::openapi());
    merged.merge(RepositoryApiDoc::openapi());
    merged.merge(SecretApiDoc::openapi());
    merged.merge(TagApiDoc::openapi());
    merged.merge(ProviderSettingsApiDoc::openapi());
    merged.merge(AdapterApiDoc::openapi());
    merged.merge(TeamApiDoc::openapi());
    merged.merge(EventsApiDoc::openapi());
    merged.merge(TaskEventApiDoc::openapi());
    merged.merge(TaskTemplateApiDoc::openapi());
    merged.merge(FeatureFlagsApiDoc::openapi());
    merged.merge(SkillApiDoc::openapi());

    merged.info = utoipa::openapi::InfoBuilder::new()
        .title("Moatless Vibe Backend API")
        .version(env!("CARGO_PKG_VERSION"))
        .description(Some(
            "Sandbox orchestration and authentication for Moatless Vibe",
        ))
        .build();

    merged
}
