//! Prometheus metrics for the runtime service.

use axum::{Router, extract::State, response::IntoResponse, routing::get};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use std::sync::Arc;

/// Bucket bounds, in seconds, for every histogram whose name ends `_seconds`.
///
/// One ladder has to serve a 5ms UI-RPC call and a 300s sandbox provision, so
/// it is coarse at both ends and dense across the 10ms-1s band most request
/// work lands in.
const DURATION_SECONDS_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0,
];

/// Metrics state containing the Prometheus handle
#[derive(Clone)]
struct MetricsState {
    handle: Arc<PrometheusHandle>,
}

/// The exporter as this process configures it, before a recorder is built
/// from it.
///
/// Duration histograms are given explicit buckets. Without them this exporter
/// renders every `metrics::histogram!` as a Prometheus *summary*: quantiles
/// computed inside one process over a rolling window. Those cannot be
/// aggregated, so `histogram_quantile` has nothing to read and a deployment
/// running more than one replica has no fleet-wide p95 at all. Matching by
/// suffix leaves the histograms that do not measure time — `..._usd`,
/// `..._bytes`, `..._turns` — as summaries, which is what suits them.
#[allow(clippy::expect_used)] // fail-fast: metrics is a startup prerequisite
fn builder() -> PrometheusBuilder {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Suffix("_seconds".to_owned()),
            DURATION_SECONDS_BUCKETS,
        )
        .expect("the duration bucket ladder is non-empty")
}

/// Initialize the metrics system and return a handle.
#[allow(clippy::expect_used)] // fail-fast: metrics is a startup prerequisite
pub fn init_metrics() -> PrometheusHandle {
    builder()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A duration histogram has to reach Prometheus as a histogram. Rendered
    /// as a summary its quantiles are per-process, so they cannot be summed
    /// across replicas and `histogram_quantile` has nothing to read.
    #[test]
    fn a_duration_histogram_is_bucketed_and_a_non_duration_one_is_not() {
        let recorder = builder().build_recorder();
        metrics::with_local_recorder(&recorder, || {
            metrics::histogram!("moatless_example_duration_seconds").record(0.42);
            metrics::histogram!("moatless_example_bytes").record(1024.0);
        });
        let rendered = recorder.handle().render();

        assert!(
            rendered.contains("moatless_example_duration_seconds_bucket{le=\"0.5\"}"),
            "no buckets rendered:\n{rendered}"
        );
        assert!(
            !rendered.contains("moatless_example_duration_seconds{quantile="),
            "still a summary:\n{rendered}"
        );
        assert!(
            rendered.contains("moatless_example_bytes{quantile="),
            "the suffix match leaked onto a non-duration histogram:\n{rendered}"
        );
    }
}
