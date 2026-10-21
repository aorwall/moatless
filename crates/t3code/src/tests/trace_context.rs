//! Reading an inbound `traceparent` so a client's span is the server span's
//! parent.

use super::fixtures::*;
use super::*;
use opentelemetry::trace::{TraceContextExt, TracerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt;

const VALID_TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const VALID_SPAN_ID: &str = "b7ad6b7169203331";

#[tokio::test]
async fn otlp_traces_enabled_reflects_whether_export_is_active() {
    let off = state();
    assert_eq!(
        server_config(&off, &viewer(), &[]).await["observability"]["otlpTracesEnabled"],
        false
    );

    let on = UiRpcState {
        rpc: Arc::new(UiRpcConfig {
            otlp_traces_enabled: true,
            ..(*off.rpc).clone()
        }),
        ..off
    };
    assert_eq!(
        server_config(&on, &viewer(), &[]).await["observability"]["otlpTracesEnabled"],
        true
    );
}

#[test]
fn well_formed_client_context_parses_into_a_remote_span_context() {
    let cx = remote_span_context(Some(VALID_TRACE_ID), Some(VALID_SPAN_ID), Some(true))
        .expect("well-formed ids parse");
    let span_context = cx.span().span_context().clone();
    assert_eq!(span_context.trace_id().to_string(), VALID_TRACE_ID);
    assert_eq!(span_context.span_id().to_string(), VALID_SPAN_ID);
    assert!(span_context.is_sampled());
    assert!(span_context.is_remote());
}

#[test]
fn unsampled_client_context_carries_no_sampled_flag() {
    let cx = remote_span_context(Some(VALID_TRACE_ID), Some(VALID_SPAN_ID), Some(false))
        .expect("well-formed ids parse");
    assert!(!cx.span().span_context().is_sampled());
}

#[test]
fn missing_ids_yield_no_remote_context() {
    assert!(remote_span_context(None, Some(VALID_SPAN_ID), Some(true)).is_none());
    assert!(remote_span_context(Some(VALID_TRACE_ID), None, Some(true)).is_none());
}

#[test]
fn malformed_ids_yield_no_remote_context() {
    // Wrong length.
    assert!(remote_span_context(Some("deadbeef"), Some(VALID_SPAN_ID), Some(true)).is_none());
    // Right length, not hex.
    assert!(
        remote_span_context(
            Some("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            Some(VALID_SPAN_ID),
            Some(true)
        )
        .is_none()
    );
    // The all-zero trace id is explicitly invalid per the W3C spec.
    assert!(
        remote_span_context(
            Some("00000000000000000000000000000000"),
            Some(VALID_SPAN_ID),
            Some(true)
        )
        .is_none()
    );
}

#[test]
fn a_well_formed_client_trace_id_becomes_the_request_spans_trace_id() {
    let (provider, exporter) = {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        (provider, exporter)
    };
    let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
    let subscriber = tracing_subscriber::registry().with(otel_layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    {
        let span = request_root_span(
            "shell.list",
            Some(VALID_TRACE_ID),
            Some(VALID_SPAN_ID),
            Some(true),
        );
        let _entered = span.enter();
    }
    provider.force_flush().expect("flush the simple exporter");

    let spans = exporter.get_finished_spans().expect("exporter is readable");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].span_context.trace_id().to_string(), VALID_TRACE_ID);
}

#[test]
fn a_malformed_client_trace_id_mints_a_fresh_root_and_records_the_raw_value() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
    let subscriber = tracing_subscriber::registry().with(otel_layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    {
        let span = request_root_span("shell.list", Some("not-a-trace-id"), None, None);
        let _entered = span.enter();
    }
    provider.force_flush().expect("flush the simple exporter");

    let spans = exporter.get_finished_spans().expect("exporter is readable");
    assert_eq!(spans.len(), 1);
    // A fresh root never shares the malformed client value verbatim as
    // its trace id, but the raw value is preserved as an attribute.
    assert_ne!(
        spans[0].span_context.trace_id().to_string(),
        "not-a-trace-id"
    );
    let recorded = spans[0]
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "client_trace_id");
    assert_eq!(
        recorded.map(|kv| kv.value.as_str().to_string()),
        Some("not-a-trace-id".to_string())
    );
}

/// A `Die` reaches Tempo as an error span, which is what makes "show me the
/// failing requests" a search rather than a read of every span.
#[test]
fn a_failed_request_marks_its_span_as_an_error() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
    let subscriber = tracing_subscriber::registry().with(otel_layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    {
        let span = request_root_span("shell.list", None, None, None);
        let _entered = span.enter();
        record_request(
            &span,
            Some("shell.list"),
            &[json!({
                "_tag": "Exit",
                "requestId": "1",
                "exit": { "_tag": "Failure", "cause": [{ "_tag": "Die", "defect": "boom" }] }
            })],
            rpc_metrics::Mark::None,
            Duration::from_millis(3),
            &viewer(),
        );
    }
    provider.force_flush().expect("flush the simple exporter");

    let spans = exporter.get_finished_spans().expect("exporter is readable");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].status, opentelemetry::trace::Status::error(""));
    let outcome = spans[0]
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "outcome");
    assert_eq!(
        outcome.map(|kv| kv.value.as_str().to_string()),
        Some("die".to_string())
    );
}

/// The counterpart: a success must not be searchable as an error, and it still
/// carries the outcome so a Tempo search can name it.
#[test]
fn a_successful_request_leaves_its_span_unset() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
    let subscriber = tracing_subscriber::registry().with(otel_layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    {
        let span = request_root_span("server.probe", None, None, None);
        let _entered = span.enter();
        record_request(
            &span,
            Some("server.probe"),
            &[json!({
                "_tag": "Exit",
                "requestId": "1",
                "exit": { "_tag": "Success", "value": {} }
            })],
            rpc_metrics::Mark::None,
            Duration::from_millis(1),
            &viewer(),
        );
    }
    provider.force_flush().expect("flush the simple exporter");

    let spans = exporter.get_finished_spans().expect("exporter is readable");
    assert_eq!(spans.len(), 1);
    assert_ne!(spans[0].status, opentelemetry::trace::Status::error(""));
    let outcome = spans[0]
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "outcome");
    assert_eq!(
        outcome.map(|kv| kv.value.as_str().to_string()),
        Some("success".to_string())
    );
}
