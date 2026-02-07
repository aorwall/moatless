//! Prometheus metrics for the runtime service.

use axum::{extract::State, response::IntoResponse, routing::get, Router};
use metrics::{counter, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::Arc;

/// Metrics state containing the Prometheus handle
#[derive(Clone)]
pub struct MetricsState {
    pub handle: Arc<PrometheusHandle>,
}

/// Initialize the metrics system and return a handle
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

// Metric recording functions

/// Record a proxy request
pub fn record_request(status_category: &str) {
    counter!("proxy_requests_total", "status" => status_category.to_string()).increment(1);
}

/// Record request duration
pub fn record_request_duration(duration_secs: f64) {
    histogram!("proxy_request_duration_seconds").record(duration_secs);
}

/// Record a cache hit
pub fn record_cache_hit(cache_name: &str) {
    counter!("cache_hits_total", "cache" => cache_name.to_string()).increment(1);
}

/// Record a cache miss
pub fn record_cache_miss(cache_name: &str) {
    counter!("cache_misses_total", "cache" => cache_name.to_string()).increment(1);
}

/// Record database query duration
pub fn record_db_query_duration(query_type: &str, duration_secs: f64) {
    histogram!("db_query_duration_seconds", "query" => query_type.to_string())
        .record(duration_secs);
}

/// Record auth validation
pub fn record_auth_validation(auth_type: &str, success: bool) {
    let result = if success { "success" } else { "failure" };
    counter!("auth_validations_total", "type" => auth_type.to_string(), "result" => result)
        .increment(1);
}

/// Record upstream proxy duration
pub fn record_upstream_duration(duration_secs: f64) {
    histogram!("proxy_upstream_duration_seconds").record(duration_secs);
}

