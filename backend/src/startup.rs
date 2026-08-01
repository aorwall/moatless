//! Application startup helpers.
//!
//! Each function initialises one infrastructure concern (logging, metrics,
//! database, Redis, S3, K8s, gRPC, SSE) so that `main()` reads as a short
//! sequential recipe rather than a single monolithic function.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::provider_token_dao::{PgProviderTokenDao, ProviderTokenDao};
use crate::auth::provider_tokens::{PgProviderTokenAuthDao, ProviderTokenAuthDao};
use crate::auth::users::{PgUserDao, UserDao};
use crate::config::Config;
use crate::credentials::{BroadcastContext, CredentialService};
use crate::db::{self, DbPool};
use crate::events::{EventPublisher, RedisEventPublisher};
use crate::file_storage::dao::{FileDao, PgFileDao};
use crate::message::dao::{MessageDao, PgMessageDao};
use crate::metrics;
use crate::repository::dao::{PgRepositoryDao, RepositoryDao};
use crate::sandbox::grpc_server::BackendSandboxServiceImpl;
use crate::sandbox::k8s::client::K8sClient;
use crate::secret::audit::{PgSecretAuditDao, SecretAuditDao};
use crate::secret::dao::{PgSecretDao, SecretDao};
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretService};
use crate::sse::consumer::RedisStreamConsumer;
use crate::sse::routes::SseState;
use crate::task::dao::{PgTaskDao, TaskDao};

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
///
/// Returned for the root supervisor to own: a bind failure or an early end to
/// `serve` returns from the task, and the supervisor turns that into the
/// process-level failure the old `expect` was reaching for.
pub fn spawn_metrics_server(port: u16, cancel: CancellationToken) -> JoinHandle<()> {
    let handle = metrics::init_metrics();
    crate::events::describe_event_metrics();
    crate::event_ingress::describe_ingress_metrics();
    crate::adapters::metrics::describe_adapter_metrics();
    crate::adapters::delivery::describe_adapter_delivery_metrics();
    crate::loops::firing::metrics::describe_trigger_metrics();
    crate::bindings::metrics::describe_binding_metrics();
    crate::event_handler::metrics::describe_agent_metrics();
    crate::sandbox::metrics::describe_sandbox_metrics();
    let router = metrics::metrics_router(handle);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tokio::spawn(async move {
        let listener = match TcpListener::bind(addr).await {
            Ok(listener) => listener,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    port,
                    "Failed to bind metrics server — is the port already in use?"
                );
                return;
            }
        };
        info!(port, "Metrics server started");
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async move { cancel.cancelled().await })
            .await
        {
            tracing::error!(error = %e, "Metrics server failed");
        }
    })
}

// =========================================================================
// Database
// =========================================================================

/// Optionally create a Postgres pool and run migrations.
///
/// Returns `None` when `DATABASE_URL` is unset, or when the connection still
/// fails after retrying. The initial connection is retried to ride out a
/// Postgres that is still starting (cold start / restart race) rather than
/// permanently disabling the database — a silent disable leaves a "healthy"
/// pod whose every API call fails.
///
/// When `run_migrations` is false (env `RUN_MIGRATIONS=false`), the pool is
/// created but pending migrations are NOT applied — used by the shadow-dev
/// backend so a dev instance never mutates the shared dev schema.
pub async fn init_database(
    database_url: Option<&str>,
    max_connections: u32,
    run_migrations: bool,
) -> Option<DbPool> {
    let url = if let Some(u) = database_url {
        u
    } else {
        info!("DATABASE_URL not set — database features disabled");
        return None;
    };

    const MAX_ATTEMPTS: u32 = 10;
    for attempt in 1..=MAX_ATTEMPTS {
        match db::create_pool(url, max_connections).await {
            Ok(pool) => {
                db::record_pool_metrics(&pool);
                info!("Database pool initialized");
                if run_migrations {
                    if let Err(e) = db::run_migrations(&pool).await {
                        tracing::error!(error = %e, "Database migration failed");
                        std::process::exit(1);
                    }
                    info!("Database migrations complete");
                } else {
                    info!("RUN_MIGRATIONS=false — skipping database migrations");
                }
                return Some(pool);
            }
            Err(e) if attempt < MAX_ATTEMPTS => {
                tracing::warn!(
                    error = %e,
                    attempt,
                    max_attempts = MAX_ATTEMPTS,
                    "Database connection failed, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    attempts = MAX_ATTEMPTS,
                    "Failed to create database pool after retries — database features disabled"
                );
                return None;
            }
        }
    }
    None
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
///
/// Redis feeds only the live SSE tail (via the broadcast bus). Historical event
/// replay is served from the `task_events` Postgres table by the AppState-based
/// `task_events_router` (`GET /api/v1/tasks/{task_id}/events`), which authorizes
/// the caller against the task — so it is wired separately from this firehose.
///
/// The consumer's join handle is returned for the root supervisor to own; it is
/// `None` when Redis is not configured and no consumer was started.
pub async fn init_sse(
    config: &Config,
    db_pool: Option<&DbPool>,
    cancel: CancellationToken,
) -> Result<(SseState, Option<JoinHandle<()>>), redis::RedisError> {
    let (tx, _) = broadcast::channel::<String>(config.sse.broadcast_capacity);

    // Per-tenant firehose filtering resolves each event's task against the
    // caller's scope. Without a DB the firehose can't authorize scoped users.
    let tasks: Option<Arc<dyn TaskDao>> =
        db_pool.map(|pool| Arc::new(PgTaskDao(pool.clone())) as _);

    let consumer_handle = if let Some(ref redis_url) = config.redis.url {
        let client = redis::Client::open(redis_url.as_str())?;
        let conn = redis::aio::ConnectionManager::new_with_config(
            client,
            crate::events::redis_manager_config(),
        )
        .await?;
        let consumer = RedisStreamConsumer::new(
            conn,
            config.sse.consumer_id.clone(),
            tx.clone(),
            cancel.clone(),
        );
        let handle = tokio::spawn(consumer.run());
        info!(consumer_id = %config.sse.consumer_id, "SSE Redis consumer spawned");
        Some(handle)
    } else {
        info!("REDIS_URL not set — SSE Redis consumer disabled");
        None
    };

    Ok((
        SseState {
            broadcast: tx,
            cancel,
            tasks,
        },
        consumer_handle,
    ))
}

// =========================================================================
// S3
// =========================================================================

/// Optionally initialise the S3 client.
pub async fn init_s3(config: &Config) -> Option<crate::file_storage::s3::S3Storage> {
    if config.s3.bucket_name.is_none() {
        info!("S3_BUCKET_NAME not set — file uploads disabled");
        return None;
    }
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(config.s3.region.clone()))
        .load()
        .await;
    let credentials_provider = aws_config.credentials_provider();
    let client = aws_sdk_s3::Client::new(&aws_config);
    info!("S3 client initialized");
    Some(crate::file_storage::s3::S3Storage::new(
        client,
        credentials_provider,
    ))
}

// =========================================================================
// gRPC
// =========================================================================

/// Spawn the backend ↔ sandbox gRPC server.
///
/// Returned for the root supervisor to own — every sandbox reaches the backend
/// through this server, so its termination must not be silent.
pub fn spawn_grpc_server(
    config: &Config,
    db_pool: Option<&DbPool>,
    event_publisher: Option<Arc<dyn EventPublisher>>,
    s3: Option<Arc<crate::file_storage::s3::S3Storage>>,
    connector: Arc<dyn crate::sandbox::access::SandboxConnector>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let messages: Option<Arc<dyn MessageDao>> =
        db_pool.map(|pool| Arc::new(PgMessageDao(pool.clone())) as _);
    let files: Option<Arc<dyn FileDao>> =
        db_pool.map(|pool| Arc::new(PgFileDao(pool.clone())) as _);
    let provider_tokens: Option<Arc<dyn ProviderTokenDao>> =
        db_pool.map(|pool| Arc::new(PgProviderTokenDao(pool.clone())) as _);
    let tasks: Option<Arc<dyn TaskDao>> =
        db_pool.map(|pool| Arc::new(PgTaskDao(pool.clone())) as _);
    let task_turns: Option<Arc<dyn crate::task_turn::dao::TaskTurnDao>> =
        db_pool.map(|pool| Arc::new(crate::task_turn::dao::PgTaskTurnDao(pool.clone())) as _);
    let grpc_task_events: Option<Arc<dyn crate::task_event::dao::TaskEventDao>> =
        db_pool.map(|pool| Arc::new(crate::task_event::dao::PgTaskEventDao(pool.clone())) as _);
    let ingress_outbox: Option<Arc<dyn crate::event_ingress::outbox::IngressOutbox>> = db_pool
        .map(|pool| Arc::new(crate::event_ingress::outbox::PgIngressOutbox(pool.clone())) as _);
    let task_relations: Option<Arc<dyn crate::task::relations::TaskRelationDao>> =
        db_pool.map(|pool| Arc::new(crate::task::relations::PgTaskRelationDao(pool.clone())) as _);
    let repositories: Option<Arc<dyn RepositoryDao>> =
        db_pool.map(|pool| Arc::new(PgRepositoryDao(pool.clone())) as _);
    let grpc_workspaces: Option<Arc<dyn crate::workspace::dao::WorkspaceDao>> =
        db_pool.map(|pool| Arc::new(crate::workspace::dao::PgWorkspaceDao(pool.clone())) as _);
    let users: Option<Arc<dyn UserDao>> =
        db_pool.map(|pool| Arc::new(PgUserDao(pool.clone())) as _);
    let backend_config = Arc::new(config.clone());
    let port = config.server.grpc_port;
    let token_secret = config.sandbox.task_token_secret.clone();
    let s3_bucket = config.s3.bucket_name.clone();
    let s3_key_prefix = config.s3.key_prefix.clone();
    let transcript_s3_prefix = config.s3.transcript_prefix.clone();
    let workspace_s3_prefix = config.s3.workspace_prefix.clone();
    let github_app_store: Option<Arc<dyn crate::adapter_apps::dao::AdapterAppDao>> =
        db_pool.map(|pool| Arc::new(crate::adapter_apps::dao::PgAdapterAppDao(pool.clone())) as _);
    let github_apps = Arc::new(GitHubAppRegistry::new(
        crate::auth::github::service::GitHubAppService::from_config(config).map(Arc::new),
        github_app_store,
        LocalEnvelopeSecretCipher::from_config(&config.secrets),
    ));
    let github_app_service = github_apps.default_service();
    let provider_token_auth: Option<Arc<dyn ProviderTokenAuthDao>> =
        db_pool.map(|pool| Arc::new(PgProviderTokenAuthDao(pool.clone())) as _);
    let secrets = db_pool.map(|pool| {
        let secret_records: Arc<dyn SecretDao> = Arc::new(PgSecretDao(pool.clone()));
        let secret_audits: Arc<dyn SecretAuditDao> = Arc::new(PgSecretAuditDao(pool.clone()));
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
                connector: Arc::clone(&connector),
                tasks: t.clone(),
            }),
        );
        if let Some(pool) = db_pool {
            svc = svc.with_codex_refresh(pool.clone(), codex_http.clone());
        }
        if let Some(ref secrets) = secrets {
            svc = svc.with_secrets(secrets.clone());
        }
        Arc::new(svc)
    });

    // The single verification layer. `Some(secret)` → enforcing, `None` → the
    // dev bypass (production refuses to boot without the secret; see
    // `Config::validate`).
    let authorizer = crate::sandbox::grpc_server::SandboxAuthorizer::from_config(
        token_secret.clone(),
        config.sandbox.accept_claimless_tokens,
        tasks.clone(),
        users.clone(),
    );

    tokio::spawn(async move {
        let svc = BackendSandboxServiceImpl {
            events: event_publisher,
            messages,
            provider_tokens,
            tasks,
            task_turns,
            task_events: grpc_task_events,
            ingress_outbox,
            task_relations,
            task_token_secret: token_secret,
            authorizer,
            github_app_service,
            github_apps,
            provider_token_auth,
            secrets,
            credentials_svc,
            repositories,
            workspaces: grpc_workspaces,
            backend_config: Some(backend_config),
            s3,
            files,
            s3_bucket,
            s3_key_prefix,
            transcript_s3_prefix,
            workspace_s3_prefix,
            users,
            event_dedup: Arc::new(crate::sandbox::event_dedup::EventDedup::new()),
        };
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!(port, "Starting backend gRPC server");
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(
                sandbox_proto::backend_sandbox_service_server::BackendSandboxServiceServer::new(
                    svc,
                ),
            )
            .serve_with_shutdown(addr, async move { cancel.cancelled().await })
            .await
        {
            tracing::error!(error = %e, "gRPC server failed");
        }
    })
}

// =========================================================================
// Shutdown
// =========================================================================

/// Spawn a task that cancels the root token on a shutdown signal.
///
/// Cancelling the root token is what starts the supervised shutdown, so the
/// handler listens for SIGTERM as well as Ctrl-C: SIGTERM is how Kubernetes
/// terminates a pod, and without it the bounded join never runs in production.
pub fn install_shutdown_handler(cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        shutdown_signal().await;
        info!("Received shutdown signal, cancelling background tasks");
        cancel.cancel();
    })
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            tokio::select! {
                () = ctrl_c => {}
                _ = sigterm.recv() => {}
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "SIGTERM handler unavailable; listening for Ctrl-C only");
            ctrl_c.await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

// =========================================================================
// Readiness
// =========================================================================

/// Build the `/ready` readiness probe.
///
/// Distinct from `/health`, which is a pure liveness probe (process up). The
/// probe reports unavailable when a readiness-critical supervised task has
/// terminated, the main database pool cannot serve a trivial query promptly, or
/// Redis is unreachable — task creation degrades silently when event publishes
/// time out. Either way Kubernetes sheds traffic to a replica that can still do
/// the work.
pub fn readiness_router(
    database: Option<DbPool>,
    events: Option<Arc<dyn EventPublisher>>,
    supervisor: Arc<crate::supervisor::SupervisorHealth>,
) -> Router {
    Router::new().route(
        "/ready",
        axum::routing::get(move || {
            let database = database.clone();
            let events = events.clone();
            let supervisor = Arc::clone(&supervisor);
            async move {
                use axum::http::StatusCode;
                use axum::response::IntoResponse;

                let failed = supervisor.failed_tasks();
                if !failed.is_empty() {
                    tracing::warn!(tasks = ?failed, "Readiness probe: supervised tasks terminated");
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(serde_json::json!({
                            "status": "unavailable",
                            "component": "supervisor",
                            "tasks": failed
                        })),
                    )
                        .into_response();
                }

                if let Some(pool) = database {
                    db::record_pool_metrics(&pool);
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(500),
                        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&pool),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => {
                            db::record_readiness_failure("query");
                            tracing::warn!(
                                error = %error,
                                "Readiness probe: database unavailable"
                            );
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                axum::Json(serde_json::json!({
                                    "status": "unavailable",
                                    "component": "database"
                                })),
                            )
                                .into_response();
                        }
                        Err(_) => {
                            db::record_readiness_failure("timeout");
                            tracing::warn!(
                                "Readiness probe: database pool did not answer within 500ms"
                            );
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                axum::Json(serde_json::json!({
                                    "status": "unavailable",
                                    "component": "database"
                                })),
                            )
                                .into_response();
                        }
                    }
                }

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
    )
}

// =========================================================================
// Sandbox adapters
// =========================================================================

/// Build the Kubernetes runtime/observer/connector adapter.
pub fn build_k8s_sandbox_adapter(
    config: &Config,
    k8s: K8sClient,
    s3: Option<Arc<aws_sdk_s3::Client>>,
    plan: Option<crate::sandbox::lifecycle::ProvisionDeps>,
) -> Arc<crate::sandbox::k8s::provider::K8sSandboxAdapter> {
    use crate::sandbox::k8s::provider::K8sSandboxAdapter;
    use crate::sandbox::k8s::status_resolver::StatusResolver;

    let status_resolver = StatusResolver::new(k8s.clone(), Arc::new(config.clone()));

    Arc::new(K8sSandboxAdapter {
        config: Arc::new(config.clone()),
        k8s,
        status_resolver,
        plan,
        s3,
    })
}

/// Build the Docker runtime/observer/connector adapter.
pub fn build_docker_sandbox_adapter(
    config: &Config,
    client: crate::sandbox::docker::client::DockerClient,
    s3: Option<Arc<aws_sdk_s3::Client>>,
    plan: Option<crate::sandbox::lifecycle::ProvisionDeps>,
) -> Arc<crate::sandbox::docker::provider::DockerSandboxAdapter> {
    use crate::sandbox::docker::provider::DockerSandboxAdapter;
    use crate::sandbox::docker::status_resolver::DockerStatusResolver;

    let status_resolver = DockerStatusResolver::new(client.clone(), Arc::new(config.clone()));

    Arc::new(DockerSandboxAdapter {
        config: Arc::new(config.clone()),
        client,
        status_resolver,
        plan,
        s3,
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
    use crate::feature_flags::{FeatureFlagsResponse, SlackConfig};
    use crate::feedback::routes::FeedbackApiDoc;
    use crate::file_storage::routes::FileStorageApiDoc;
    use crate::loops::http::LoopsApiDoc;
    use crate::message::routes::MessageApiDoc;
    use crate::plugin::routes::PluginApiDoc;
    use crate::provider_settings::routes::ProviderSettingsApiDoc;
    use crate::repository::routes::RepositoryApiDoc;
    use crate::sandbox::routes::SandboxApiDoc;
    use crate::secret::routes::SecretApiDoc;
    use crate::skill::routes::SkillApiDoc;
    use crate::tag::routes::TagApiDoc;
    use crate::task::routes::TaskApiDoc;
    use crate::task_event::routes::TaskEventApiDoc;
    use crate::team::routes::TeamApiDoc;
    use crate::workspace::routes::WorkspaceApiDoc;
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(
        components(schemas(
            // Event types + sub-types
            moatless_events::EventType,
            moatless_events::SandboxStatus,
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
    merged.merge(FeedbackApiDoc::openapi());
    merged.merge(FileStorageApiDoc::openapi());
    merged.merge(RepositoryApiDoc::openapi());
    merged.merge(SecretApiDoc::openapi());
    merged.merge(TagApiDoc::openapi());
    merged.merge(ProviderSettingsApiDoc::openapi());
    merged.merge(AdapterApiDoc::openapi());
    merged.merge(LoopsApiDoc::openapi());
    merged.merge(TeamApiDoc::openapi());
    merged.merge(EventsApiDoc::openapi());
    merged.merge(TaskEventApiDoc::openapi());
    merged.merge(FeatureFlagsApiDoc::openapi());
    merged.merge(PluginApiDoc::openapi());
    merged.merge(SkillApiDoc::openapi());
    merged.merge(WorkspaceApiDoc::openapi());

    merged.info = utoipa::openapi::InfoBuilder::new()
        .title("Moatless Vibe Backend API")
        .version(env!("CARGO_PKG_VERSION"))
        .description(Some(
            "Sandbox orchestration and authentication for Moatless Vibe",
        ))
        .build();

    merged
}
