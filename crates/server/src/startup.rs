//! Application startup helpers.
//!
//! Each function initialises one infrastructure concern (logging, metrics,
//! Redis, gRPC, SSE) so that `main()` reads as a short sequential recipe
//! rather than a single monolithic function.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

use crate::sandbox::grpc_server::BackendSandboxServiceImpl;
use crate::sse::consumer::RedisStreamConsumer;
use crate::sse::routes::SseState;
use moatless_core::config::{Config, OtelConfig};
use moatless_core::db::{self, DbPool};
use moatless_core::events::EventPublisher;
use moatless_core::metrics;
use moatless_ports::task::TaskReader;

// =========================================================================
// Logging
// =========================================================================

/// Install the layered tracing subscriber: the JSON `fmt` layer (stdout,
/// shipped to Loki) plus, when `otel.otlp_endpoint` is set, a
/// `tracing-opentelemetry` layer exporting spans to that OTLP collector.
///
/// With no endpoint configured the OTel layer is omitted and behaviour is
/// identical to the plain `fmt`-only subscriber this replaced. Returns the
/// `SdkTracerProvider` when export is active — the caller (`main`) owns its
/// shutdown so buffered spans flush before the process exits.
///
/// `RUST_LOG` still controls verbosity via the shared `EnvFilter`; it gates
/// both layers equally; a lost span there means fewer exported traces, not a
/// filter divergence between what's logged and what's traced.
pub fn init_tracing(otel: &OtelConfig) -> Option<SdkTracerProvider> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let base_format = tracing_subscriber::fmt::format::json()
        // Lift event fields (incl. `message`) to the JSON root so Loki's
        // `| json | message=…` works directly instead of `fields_message`.
        .flatten_event(true)
        // Render the current span's fields under a compact `span` object
        // (queryable in Loki as `span_<field>`); drop the `spans` array since
        // JSON arrays aren't field-queryable there.
        .with_current_span(true)
        .with_span_list(false);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .event_format(TraceContextJson(base_format));

    let (otel_layer, provider, otel_error) = match otel.otlp_endpoint.as_deref() {
        Some(endpoint) => match build_otel_layer(otel, endpoint) {
            Ok((layer, provider)) => (Some(layer), Some(provider), None),
            Err(error) => (None, None, Some(error.to_string())),
        },
        None => (None, None, None),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();

    // Only reachable once the subscriber above is live, so this actually logs
    // (a `tracing::warn!` any earlier would have nowhere to land).
    if let Some(error) = otel_error {
        tracing::warn!(
            error,
            endpoint = otel.otlp_endpoint.as_deref().unwrap_or_default(),
            "Failed to build the OTLP trace exporter; continuing without trace export"
        );
    }

    provider
}

/// Build the `tracing-opentelemetry` layer and its owning `SdkTracerProvider`
/// for one configured OTLP/gRPC endpoint (default port 4317).
///
/// Sampling is parent-based: a span with an already-sampled remote parent
/// (the client's `sampled` flag — see `ui_rpc::mod::request_root_span`) is
/// always kept; a root span is sampled at `otel.sampling_ratio`.
fn build_otel_layer<S>(
    otel: &OtelConfig,
    endpoint: &str,
) -> Result<
    (
        impl tracing_subscriber::Layer<S> + Send + Sync + 'static,
        SdkTracerProvider,
    ),
    opentelemetry_otlp::ExporterBuildError,
>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    use opentelemetry::trace::TracerProvider as _;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(otel.service_name.clone())
        .build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            otel.sampling_ratio,
        ))))
        .with_resource(resource)
        .build();

    let tracer = provider.tracer(otel.service_name.clone());
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    Ok((layer, provider))
}

/// The active span's OTel trace/span ids as lowercase hex, when a valid OTel
/// context is attached. `None` whenever export is off (no OTel layer
/// installed) or no span is currently active — the JSON line then carries no
/// `trace_id`/`span_id`, exactly as before this existed.
fn current_trace_context() -> Option<(String, String)> {
    let cx = tracing::Span::current().context();
    let span_context = cx.span().span_context().clone();
    if !span_context.is_valid() {
        return None;
    }
    Some((
        span_context.trace_id().to_string(),
        span_context.span_id().to_string(),
    ))
}

/// Wraps the standard JSON event formatter to splice `trace_id`/`span_id`
/// from the active OTel span onto every log line, so a Loki line can be
/// pivoted to its Tempo trace. Delegates the whole line to the wrapped
/// formatter and only post-processes when a valid trace context exists, so
/// output is byte-for-byte the same as before with OTel export off.
struct TraceContextJson<F>(F);

impl<S, N, F> FormatEvent<S, N> for TraceContextJson<F>
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let Some((trace_id, span_id)) = current_trace_context() else {
            return self.0.format_event(ctx, writer, event);
        };

        let mut line = String::new();
        self.0.format_event(ctx, Writer::new(&mut line), event)?;

        let Ok(serde_json::Value::Object(mut object)) = serde_json::from_str(line.trim_end())
        else {
            // Formatter output that doesn't parse as a JSON object is
            // unexpected but not this layer's problem to fail the line over —
            // emit it unmodified rather than lose it.
            return writer.write_str(&line);
        };
        object.insert("trace_id".into(), trace_id.into());
        object.insert("span_id".into(), span_id.into());
        writeln!(writer, "{}", serde_json::Value::Object(object))
    }
}

// =========================================================================
// Metrics
// =========================================================================

/// Spawn the Prometheus metrics server on a dedicated port.
///
/// Returned for the root supervisor to own: a bind failure or an early end to
/// `serve` returns from the task, and the supervisor turns that into a
/// process-level failure.
pub fn spawn_metrics_server(port: u16, cancel: CancellationToken) -> JoinHandle<()> {
    let handle = metrics::init_metrics();
    moatless_core::events::describe_event_metrics();
    moatless_tasks::event_ingress::describe_ingress_metrics();
    moatless_adapter_model::metrics::describe_adapter_metrics();
    crate::adapters::delivery::describe_adapter_delivery_metrics();
    moatless_loops::loops::firing::metrics::describe_loop_firing_metrics();
    moatless_loops::bindings::metrics::describe_binding_metrics();
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
///
/// Per-tenant firehose filtering resolves each event's task against the
/// caller's scope through `tasks`. Without a database the firehose can't
/// authorize scoped users.
pub async fn init_sse(
    config: &Config,
    tasks: Option<Arc<dyn TaskReader>>,
    cancel: CancellationToken,
) -> Result<(SseState, Option<JoinHandle<()>>), redis::RedisError> {
    let (tx, _) = broadcast::channel::<String>(config.sse.broadcast_capacity);

    let consumer_handle = if let Some(ref redis_url) = config.redis.url {
        let client = redis::Client::open(redis_url.as_str())?;
        let conn = redis::aio::ConnectionManager::new_with_config(
            client,
            moatless_core::events::redis_manager_config(),
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
// gRPC
// =========================================================================

/// Spawn the backend ↔ sandbox gRPC server.
///
/// Returned for the root supervisor to own — every sandbox reaches the backend
/// through this server, so its termination must not be silent.
pub fn spawn_grpc_server(
    port: u16,
    svc: BackendSandboxServiceImpl,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
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
                        moatless_core::db::probe(&pool),
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
// OpenAPI (CLI)
// =========================================================================

// The only public path into the crate-private `compose::infrastructure`:
// `main.rs` is a separate binary crate and needs this for `--dump-openapi`.
pub use crate::compose::infrastructure::build_merged_openapi;

#[cfg(test)]
mod openapi_dump_tests {
    use super::build_merged_openapi;

    /// The `--dump-openapi` builder feeds CI's TS-client generation and is a
    /// second, independent assembly site from the runtime router. A router
    /// whose `ApiDoc` is not merged here compiles clean but silently drops its
    /// endpoints from the generated client. Pin the repository sync-config
    /// endpoint, which lives in its own `ApiDoc` rather than the repository
    /// router's.
    #[test]
    fn dump_includes_the_repository_sync_endpoint() {
        let spec = serde_json::to_value(build_merged_openapi()).unwrap();
        let p = "/api/v1/repositories/{repository_id}/sync-config";
        assert!(
            spec["paths"].get(p).is_some(),
            "build_merged_openapi() is missing {p}; its ApiDoc is not merged into the dump builder"
        );
    }

    #[test]
    fn dump_includes_the_feature_flags_contract() {
        let spec = serde_json::to_value(build_merged_openapi()).unwrap();

        assert!(spec["paths"]["/api/v1/feature-flags"]["get"].is_object());
        let schema = &spec["components"]["schemas"]["FeatureFlagsResponse"];
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(
            properties.keys().map(String::as_str).collect::<Vec<_>>(),
            vec![
                "device_pairing",
                "forgejo_enabled",
                "gitlab_enabled",
                "netlify_publish",
                "show_claude_settings",
                "show_codex_settings",
                "show_model_selector",
                "workspace_settings",
            ]
        );
    }
}
