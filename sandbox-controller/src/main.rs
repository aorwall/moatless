//! Sandbox Controller - Workspace orchestration for Moatless Vibe
//!
//! Serves a unified API on port 8080. Authenticated via Traefik ForwardAuth.

mod config;
mod db;
mod events;
mod metrics;
mod sandbox;
mod target;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use config::Config;
use events::RedisEventPublisher;
use sandbox::k8s::client::K8sClient;
use sandbox::k8s::status_cache::StatusCache;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Install rustls crypto provider (required before any TLS operations)
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,sandbox_controller=debug")),
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

    // Initialize K8s client and start API server
    let k8s = K8sClient::new(config.sandbox_namespace.clone()).await?;
    let status_cache = StatusCache::new(10);
    info!("K8s client initialized");
    start_api_server(&config, k8s, status_cache, db_pool, event_publisher).await?;

    Ok(())
}

/// Start the API server on the configured port.
async fn start_api_server(
    config: &Config,
    k8s: K8sClient,
    status_cache: StatusCache,
    db_pool: Option<db::DbPool>,
    event_publisher: Option<RedisEventPublisher>,
) -> Result<(), Box<dyn std::error::Error>> {
    use sandbox::k8s::provider::K8sSandboxProvider;
    use sandbox::routes::{api_router, SandboxApiState};

    // Load pod template
    let template_path = config
        .worker_pod_template_path
        .as_deref()
        .ok_or("WORKER_POD_TEMPLATE_PATH not configured")?;

    let pod_template = tokio::fs::read_to_string(template_path)
        .await
        .map_err(|e| format!("Failed to read pod template from {}: {}", template_path, e))?;

    info!(template_path, "Pod template loaded");

    let provider = K8sSandboxProvider {
        config: Arc::new(config.clone()),
        k8s,
        status_cache,
        pod_template: Arc::new(pod_template),
        db_pool,
        event_publisher,
    };

    let state = SandboxApiState {
        provider: Arc::new(provider),
    };

    let router = api_router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    info!(port = config.port, "API server started");

    axum::serve(listener, router).await?;

    Ok(())
}
