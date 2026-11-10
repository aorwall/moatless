//! Reading one listing row, and the reads that answer without touching the
//! database at all.

use super::fixtures::*;
use super::*;
use crate::rpc::errors::LISTING_BUSY;

/// An unfiltered browse must be refused before it queries every readable Task.
/// `state()` sets no `expect_list_browsable`, so mockall panics if the database
/// is reached — passing proves it wasn't.
#[tokio::test]
async fn a_browse_naming_neither_owner_nor_tag_never_reaches_the_database() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.browse", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    assert_eq!(
        output.frames[0]["exit"]["cause"][0]["defect"],
        "threads.browse requires an ownerUserId or a tag"
    );

    // A field holding only whitespace is the same request with a keystroke in
    // it, and `ConversationBrowse::new` trims so it lands on the same refusal.
    let output = connection
        .exchange(
            &request("2", "threads.browse", json!({ "tag": "   " })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
}

/// The client asking is already showing the conversation. Answering `null`
/// leaves it showing what the thread subscription gave it; failing would take a
/// rendered transcript off the screen over a missing title.
#[tokio::test]
async fn a_listing_row_for_a_conversation_this_viewer_cannot_reach_is_null_not_an_error() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.getShell", json!({ "threadId": "task-1" })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"]["thread"], Value::Null);
}

/// Every client encodes against the contract, so a request without one is
/// malformed rather than a conversation that is missing — and the two must not
/// answer alike, or a client bug reads as an empty deployment.
#[tokio::test]
async fn a_listing_row_asked_for_without_a_thread_id_is_a_failure() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.getShell", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
}

/// One character matches most of the corpus and says nothing, and it costs a
/// scan of every Message body to say so.
#[tokio::test]
async fn a_thread_search_too_short_to_mean_anything_never_reaches_the_database() {
    let mut connection = connection();
    // `state()` sets no `search_conversations` expectation, so the mock
    // panics if this reaches it — which is the assertion.
    let output = connection
        .exchange(
            &request("1", "orchestration.searchThreads", json!({ "query": "a" })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"]["matches"], json!([]));
}

async fn shell_over_http(state: UiRpcState) -> Response {
    use axum::body::Body;
    use tower::ServiceExt;

    let mut request = axum::http::Request::builder()
        .uri("/api/orchestration/shell")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(viewer());
    router(state).oneshot(request).await.unwrap()
}

/// A shell bootstrap the listing gate turned away is backpressure: a 503 the
/// request trace logs at WARN, with a hint for when to ask again.
#[tokio::test]
async fn a_shell_bootstrap_refused_at_capacity_is_a_marked_503() {
    let response = shell_over_http(state_with_listing_failing(|| ListingError::AtCapacity)).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    assert!(
        response
            .extensions()
            .get::<moatless_types::http::RefusedAtCapacity>()
            .is_some()
    );
}

/// A pool that could not hand out a connection is still this backend failing,
/// so its 503 stays unmarked and logs at ERROR.
#[tokio::test]
async fn a_shell_bootstrap_the_pool_could_not_serve_is_an_unmarked_503() {
    let response = shell_over_http(state_with_listing_failing(|| {
        ListingError::Database(sqlx::Error::PoolTimedOut)
    }))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        response
            .extensions()
            .get::<moatless_types::http::RefusedAtCapacity>()
            .is_none()
    );
}

/// The `outcome` the one request `state` answers records on its span.
async fn archived_shell_outcome(state: UiRpcState) -> (Value, Option<String>) {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::layer::SubscriberExt;

    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(otel_layer));

    let output = connection()
        .exchange(
            &request("1", "orchestration.getArchivedShellSnapshot", json!({})),
            &state,
            &viewer(),
        )
        .await;

    provider.force_flush().expect("flush the simple exporter");
    let spans = exporter.get_finished_spans().expect("exporter is readable");
    let outcome = spans
        .iter()
        .find(|span| span.name == "ui_rpc.request")
        .and_then(|span| {
            span.attributes
                .iter()
                .find(|kv| kv.key.as_str() == "outcome")
        })
        .map(|kv| kv.value.as_str().to_string());
    (output.frames[0].clone(), outcome)
}

/// The archive declares no error, so a refusal at capacity is still a `Die` on
/// the wire, but it counts as `capped` rather than toward the defects alert.
#[tokio::test]
async fn an_archive_read_refused_at_capacity_is_counted_as_capped() {
    let (frame, outcome) =
        archived_shell_outcome(state_with_listing_failing(|| ListingError::AtCapacity)).await;

    assert_eq!(frame["exit"]["cause"][0]["_tag"], "Die");
    assert_eq!(outcome.as_deref(), Some("capped"));
}

/// The listing's own `statement_timeout` cancelling it is the same load
/// shedding as the permit, so it too is `capped` and not a defect.
#[tokio::test]
async fn an_archive_read_its_statement_timeout_cancelled_is_counted_as_capped() {
    let (frame, outcome) =
        archived_shell_outcome(state_with_listing_failing(|| ListingError::TimedOut)).await;

    assert_eq!(frame["exit"]["cause"][0]["_tag"], "Die");
    assert_eq!(frame["exit"]["cause"][0]["defect"], LISTING_BUSY);
    assert_eq!(outcome.as_deref(), Some("capped"));
}

#[tokio::test]
async fn a_shell_bootstrap_its_statement_timeout_cancelled_is_a_marked_503() {
    let response = shell_over_http(state_with_listing_failing(|| ListingError::TimedOut)).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        response
            .extensions()
            .get::<moatless_types::http::RefusedAtCapacity>()
            .is_some()
    );
}

#[tokio::test]
async fn an_archive_read_the_pool_could_not_serve_is_still_a_defect() {
    let (frame, outcome) = archived_shell_outcome(state_with_listing_failing(|| {
        ListingError::Database(sqlx::Error::PoolTimedOut)
    }))
    .await;

    assert_eq!(frame["exit"]["cause"][0]["_tag"], "Die");
    assert_eq!(outcome.as_deref(), Some("die"));
}
