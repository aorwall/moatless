//! Prometheus metrics for the runtime service.

use axum::{extract::State, response::IntoResponse, routing::get, Router};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::Arc;

/// Metrics state containing the Prometheus handle
#[derive(Clone)]
pub struct MetricsState {
    pub handle: Arc<PrometheusHandle>,
}

/// Initialize the metrics system and return a handle.
#[allow(clippy::expect_used)] // fail-fast: metrics is a startup prerequisite
pub fn init_metrics() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder")
}

/// Create the metrics router
pub fn metrics_router(handle: PrometheusHandle) -> Router {
    let state = MetricsState {
        handle: Arc::new(handle),
    };

    Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(state)
}

/// Handler for /metrics endpoint
async fn metrics_handler(State(state): State<MetricsState>) -> impl IntoResponse {
    state.handle.render()
}


