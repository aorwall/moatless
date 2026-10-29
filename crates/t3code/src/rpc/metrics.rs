//! UI-RPC request spans, bounded metric labels and outcome classification.
//!
//! Emitted at [`answered`], which wraps every handler, from the frames it
//! produced. The exceptions say where they are emitted; nothing else in this
//! crate increments any of them.
//!
//! The `tag` label is bounded by the method table rather than by trust: it is
//! the matched entry's own `&'static str`, never the tag the client sent. See
//! [`Outcome::tag_label`].

use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use std::cell::Cell;
use std::future::Future;
use std::time::Duration;
use tracing::Instrument;
use tracing::warn;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Completed UI-RPC requests by `tag` and `outcome`.
pub(crate) const REQUESTS: &str = "moatless_ui_rpc_requests_total";

/// Handler wall-clock in seconds by `tag` and `outcome`. Measures the frames a
/// request answered with, so a subscription's is its open, not its lifetime.
pub(crate) const REQUEST_DURATION: &str = "moatless_ui_rpc_request_duration_seconds";

/// Frames the socket refused before any dispatch, by `reason`.
pub(crate) const CONNECTION_DEFECTS: &str = "moatless_ui_rpc_connection_defects_total";

/// Seconds one connection spent answering a frame and so unable to read its
/// socket.
///
/// Unlabeled deliberately — this is how long a single frame holds the loop,
/// not which method is slow. Emitted from `serve_websocket` rather than
/// [`answered`], since it measures the connection.
pub(crate) const CONNECTION_BLOCKED: &str = "moatless_ui_rpc_connection_blocked_seconds";

/// Thread subscriptions open on this process, each reading its conversation
/// when the change poller reports it moved. Held by `watch_thread` rather than
/// [`answered`], since a stream outlives its request.
pub(crate) const THREAD_SUBSCRIPTIONS: &str = "moatless_ui_rpc_thread_subscriptions_open";

/// Register the descriptions during compatibility router assembly.
pub(crate) fn describe() {
    metrics::describe_counter!(REQUESTS, "T3 UI-RPC requests by method tag and outcome");
    metrics::describe_histogram!(
        REQUEST_DURATION,
        "T3 UI-RPC handler duration in seconds by method tag and outcome"
    );
    metrics::describe_counter!(
        CONNECTION_DEFECTS,
        "T3 UI-RPC frames refused before dispatch, by reason"
    );
    metrics::describe_histogram!(
        CONNECTION_BLOCKED,
        "T3 UI-RPC seconds a connection could not read its socket"
    );
    metrics::describe_gauge!(
        THREAD_SUBSCRIPTIONS,
        "T3 UI-RPC thread subscriptions currently open"
    );
}

/// What one request told the client, read off the frames it answered with.
///
/// The names are the client's vocabulary: `Fail` is an outcome the method's
/// contract declares and the UI renders in its own words; `Die` is a defect the
/// UI can only render as this server being broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// `Exit` carrying `Success`.
    Success,
    /// A stream that opened and has not exited. The producer answers later on
    /// the socket, so this counts the subscribe and never its end.
    Open,
    /// The frame never reached the dispatch — a malformed or duplicate request
    /// id. Its `tag` is unverified and is not used as a label.
    Rejected,
    /// `Fail` carrying `UnsupportedMethodError`. Reachable for any tag at all,
    /// so its `tag` is not used as a label either.
    Unsupported,
    /// `Fail` carrying `EnvironmentAuthorizationError`: this deployment does
    /// not grant the scope the method needs.
    Unauthorized,
    /// A `Fail` its site marked with [`refused`]: the request named something
    /// absent or asked in a state that cannot answer it, such as a missing file
    /// or a stopped sandbox. Not a server failure.
    Refused,
    /// Any other `Fail`.
    Fail,
    /// A `Die` its site marked with [`capped`]: the connection already holds
    /// [`crate::MAX_ACTIVE_STREAMS`] streams, or another full listing held the
    /// replica's permit for the whole queue wait. A limit was reached; the
    /// server is not broken.
    Capped,
    /// `Die`.
    Die,
    /// The request stopped without answering: the client sent `Interrupt`, or
    /// the connection carrying it went away. See [`answered`].
    Interrupted,
}

impl Outcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Open => "open",
            Self::Rejected => "rejected",
            Self::Unsupported => "unsupported",
            Self::Unauthorized => "unauthorized",
            Self::Refused => "refused",
            Self::Fail => "fail",
            Self::Capped => "capped",
            Self::Die => "die",
            Self::Interrupted => "interrupted",
        }
    }

    /// The `tag` label for a request that ended this way, given the tag of the
    /// table entry it matched.
    ///
    /// [`Self::Rejected`] and [`Self::Unsupported`] collapse to a constant even
    /// for a matched entry, so a method without a handler counts with the tags
    /// no entry matched. No other outcome is reachable without an entry.
    pub(crate) fn tag_label(self, method: Option<&'static str>) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Unsupported => "unsupported",
            _ => method.unwrap_or("unsupported"),
        }
    }

    /// Whether the client was told the request failed, for the span status and
    /// the log line. An [`Self::Interrupted`] request is not a failure: the
    /// client asked for it to stop. Nor is a [`Self::Refused`] one: the client
    /// got the answer the state it asked in has. Nor a [`Self::Capped`] one:
    /// a deliberate limit refused it.
    pub(crate) fn is_failure(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Unsupported | Self::Unauthorized | Self::Fail | Self::Die
        )
    }
}

/// What the running request's site said about the failure it answers with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mark {
    #[default]
    None,
    /// Set by [`refused`].
    Refused,
    /// Set by [`capped`].
    Capped,
}

tokio::task_local! {
    /// Set inside the request [`answered`] is running.
    static MARK: Cell<Mark>;
}

/// Mark the `Fail` the running request answers with as [`Outcome::Refused`].
///
/// Call it where the typed failure is built, from the handler's own task: a
/// task it spawns does not see the mark, and outside [`answered`] it does
/// nothing. A `Die` or an `EnvironmentAuthorizationError` keeps its outcome.
pub(crate) fn refused() {
    let _ = MARK.try_with(|mark| mark.set(Mark::Refused));
}

/// Mark the `Die` the running request answers with as [`Outcome::Capped`].
///
/// Bound like [`refused`]: the handler's own task, inside [`answered`]. A
/// `Fail` keeps its outcome.
pub(crate) fn capped() {
    let _ = MARK.try_with(|mark| mark.set(Mark::Capped));
}

/// Read the outcome off the frames one request answered with.
///
/// A `Defect` frame outranks an `Exit` because it's what the pre-dispatch
/// checks emit, and those run before the tag is looked at.
pub(crate) fn classify(frames: &[Value]) -> Outcome {
    if frames.iter().any(|frame| frame["_tag"] == "Defect") {
        return Outcome::Rejected;
    }
    let Some(exit) = frames.iter().find(|frame| frame["_tag"] == "Exit") else {
        return Outcome::Open;
    };
    let exit = &exit["exit"];
    if exit["_tag"] == "Success" {
        return Outcome::Success;
    }
    let cause = &exit["cause"][0];
    match cause["_tag"].as_str() {
        Some("Interrupt") => Outcome::Interrupted,
        Some("Fail") => match cause["error"]["_tag"].as_str() {
            Some("UnsupportedMethodError") => Outcome::Unsupported,
            Some("EnvironmentAuthorizationError") => Outcome::Unauthorized,
            _ => Outcome::Fail,
        },
        // A `Die` and a cause shape this crate does not produce read the same
        // to the client: an unexpected server defect.
        _ => Outcome::Die,
    }
}

/// The contract error tag a typed failure carried, for the span.
///
/// Bounded without trust: every `_tag` this crate puts inside a `Fail` is a
/// literal or `&'static str` from its own error enums, so nothing the client
/// sent reaches this — but it's not a metric label, since the counter is
/// already split by `tag`/`outcome`.
pub(crate) fn fail_tag(frames: &[Value]) -> Option<&str> {
    frames
        .iter()
        .find_map(|frame| frame["exit"]["cause"][0]["error"]["_tag"].as_str())
}

/// The defect string a [`Outcome::Die`] carried, for the log line.
pub(crate) fn defect(frames: &[Value]) -> Option<&str> {
    frames.iter().find_map(|frame| {
        frame["exit"]["cause"][0]["defect"]
            .as_str()
            .or_else(|| frame["defect"].as_str())
    })
}

/// Root span for a single UI-RPC request; honors a well-formed client trace
/// context (W3C-style 32-hex trace id / 16-hex span id) as the parent.
///
/// A missing or malformed context mints a fresh root instead of failing the
/// request, and records the raw client-supplied value for debugging.
pub(crate) fn request_root_span(
    tag: &str,
    trace_id: Option<&str>,
    span_id: Option<&str>,
    sampled: Option<bool>,
) -> tracing::Span {
    // `otel.*` are `tracing-opentelemetry`'s field names; a span records only
    // what it's declared with here. `outcome`, `duration_ms` and status are
    // filled in by [`record_request`]; `queued_ms` by the caller, the only
    // one that knows when the frame arrived.
    let span = tracing::info_span!(
        "ui_rpc.request",
        tag = %tag,
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
        outcome = tracing::field::Empty,
        error.type = tracing::field::Empty,
        client_trace_id = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
        queued_ms = tracing::field::Empty,
    );
    match remote_span_context(trace_id, span_id, sampled) {
        Some(cx) => {
            let _ = span.set_parent(cx);
        }
        None => {
            if let Some(raw) = trace_id {
                span.record("client_trace_id", raw);
            }
        }
    }
    span
}

/// Run one request inside its span, time it, and record what it answered with.
///
/// The one seam this surface is measured at: a detached request is spawned
/// already wrapped in it. A request that never answers is recorded too, as
/// [`Outcome::Interrupted`], when `DetachedRequest` aborts its task.
pub(crate) async fn answered(
    span: &tracing::Span,
    method: Option<&'static str>,
    identity: &UiRpcAuthIdentity,
    request: impl Future<Output = Vec<Value>>,
) -> Vec<Value> {
    let mut abandoned = Abandoned {
        span,
        method,
        identity,
        started: std::time::Instant::now(),
        answered: false,
    };
    let (frames, mark) = MARK
        .scope(Cell::new(Mark::None), async {
            let frames = request.instrument(span.clone()).await;
            (frames, MARK.with(Cell::get))
        })
        .await;
    abandoned.answered = true;
    record_request(
        span,
        method,
        &frames,
        mark,
        abandoned.started.elapsed(),
        identity,
    );
    frames
}

/// Records the request [`answered`] is running, for as long as it has not
/// answered. Dropping it before then is the only account anything gets of a
/// request that stopped without frames.
struct Abandoned<'a> {
    span: &'a tracing::Span,
    method: Option<&'static str>,
    identity: &'a UiRpcAuthIdentity,
    started: std::time::Instant,
    answered: bool,
}

impl Drop for Abandoned<'_> {
    fn drop(&mut self) {
        if !self.answered {
            record(
                self.span,
                self.method,
                Outcome::Interrupted,
                &[],
                self.started.elapsed(),
                self.identity,
            );
        }
    }
}

/// Count one answered request, mark its span with what the client was told,
/// and log the failures nothing else logs.
///
/// Called by [`answered`] alone; `mark` is what the handler set with
/// [`refused`] or [`capped`]. A `Die` a per-domain builder logs is logged here
/// again.
pub(crate) fn record_request(
    span: &tracing::Span,
    method: Option<&'static str>,
    frames: &[Value],
    mark: Mark,
    elapsed: Duration,
    identity: &UiRpcAuthIdentity,
) {
    let outcome = match (classify(frames), mark) {
        (Outcome::Fail, Mark::Refused) => Outcome::Refused,
        (Outcome::Die, Mark::Capped) => Outcome::Capped,
        (outcome, _) => outcome,
    };
    record(span, method, outcome, frames, elapsed, identity);
}

/// [`record_request`] for an outcome the frames cannot state, because there are
/// none. `frames` is read only for what a failure carries, and every outcome
/// reachable with an empty slice is one that carries nothing.
fn record(
    span: &tracing::Span,
    method: Option<&'static str>,
    outcome: Outcome,
    frames: &[Value],
    elapsed: Duration,
    identity: &UiRpcAuthIdentity,
) {
    let label = outcome.tag_label(method);

    metrics::counter!(
        REQUESTS,
        "tag" => label,
        "outcome" => outcome.as_str(),
    )
    .increment(1);
    metrics::histogram!(
        REQUEST_DURATION,
        "tag" => label,
        "outcome" => outcome.as_str(),
    )
    .record(elapsed.as_secs_f64());

    span.record("outcome", outcome.as_str());
    // Read `duration_ms`, not the span's own duration. A span's exported end
    // time is restamped on every exit, and something below this surface enters
    // a handle again after the handler answered: measured on staging,
    // `ui_rpc.request` spans of 30s and 144s against requests this histogram
    // put under 5s.
    span.record("duration_ms", elapsed.as_secs_f64() * 1000.0);
    if outcome.is_failure() {
        span.record("otel.status_code", "ERROR");
        if let Some(error) = fail_tag(frames) {
            span.record("error.type", error);
        }
    }

    // A `Die` is the only outcome the client can render nothing but "this
    // server is broken" for, so it is the only one worth a line on its own. The
    // rest are counted, and the refusals that are worth reading have their own
    // `warn!` at the site that decided them.
    if outcome == Outcome::Die {
        warn!(
            tag = label,
            user_id = %identity.user_id,
            defect = defect(frames).unwrap_or("(none)"),
            "A T3 UI request failed as an unexpected server defect"
        );
    }
}

/// Parses a client-supplied trace/span id pair into a remote OTel span context,
/// or `None` if either is missing or malformed. Never fails the request —
/// callers fall back to minting a fresh root span.
pub(crate) fn remote_span_context(
    trace_id: Option<&str>,
    span_id: Option<&str>,
    sampled: Option<bool>,
) -> Option<opentelemetry::Context> {
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
    };

    // `from_hex` parses any valid hex string regardless of length (it does not
    // zero-pad), so a short client value like "deadbeef" would otherwise parse
    // into a spuriously "valid" trace id. The W3C format is fixed-width: reject
    // anything but exactly 32/16 hex characters before parsing.
    let trace_id = trace_id.filter(|s| s.len() == 32)?;
    let span_id = span_id.filter(|s| s.len() == 16)?;
    let trace_id = TraceId::from_hex(trace_id)
        .ok()
        .filter(|id| *id != TraceId::INVALID)?;
    let span_id = SpanId::from_hex(span_id)
        .ok()
        .filter(|id| *id != SpanId::INVALID)?;
    let flags = if sampled.unwrap_or(false) {
        TraceFlags::SAMPLED
    } else {
        TraceFlags::default()
    };
    let span_context = SpanContext::new(trace_id, span_id, flags, true, TraceState::default());
    Some(opentelemetry::Context::new().with_remote_span_context(span_context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_types::ui_rpc::UserRole;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use serde_json::json;
    use tracing_subscriber::layer::SubscriberExt;
    use uuid::Uuid;

    fn viewer() -> UiRpcAuthIdentity {
        UiRpcAuthIdentity {
            user_id: Uuid::nil(),
            login: "viewer".into(),
            role: UserRole::User,
            provider: "test".into(),
            group_id: None,
        }
    }

    fn exit(cause: Value) -> Vec<Value> {
        vec![json!({
            "_tag": "Exit",
            "requestId": "1",
            "exit": { "_tag": "Failure", "cause": [cause] }
        })]
    }

    #[test]
    fn a_success_exit_is_a_success() {
        let frames = vec![json!({
            "_tag": "Exit",
            "requestId": "1",
            "exit": { "_tag": "Success", "value": {} }
        })];
        assert_eq!(classify(&frames), Outcome::Success);
        assert!(!classify(&frames).is_failure());
    }

    #[test]
    fn a_stream_that_only_opened_has_not_exited() {
        let frames = vec![json!({ "_tag": "Chunk", "requestId": "1", "values": [] })];
        assert_eq!(classify(&frames), Outcome::Open);
    }

    #[test]
    fn a_chunk_before_an_exit_does_not_hide_the_exit() {
        let mut frames = vec![json!({ "_tag": "Chunk", "requestId": "1", "values": [] })];
        frames.extend(exit(json!({ "_tag": "Die", "defect": "boom" })));
        assert_eq!(classify(&frames), Outcome::Die);
    }

    /// The two tagged errors that get their own outcome, because an operator
    /// reads them differently: one is a client asking for a method this server
    /// does not have, the other is this deployment refusing a scope.
    #[test]
    fn the_two_named_tagged_errors_split_out_of_fail() {
        assert_eq!(
            classify(&exit(
                json!({ "_tag": "Fail", "error": { "_tag": "UnsupportedMethodError" } })
            )),
            Outcome::Unsupported
        );
        assert_eq!(
            classify(&exit(
                json!({ "_tag": "Fail", "error": { "_tag": "EnvironmentAuthorizationError" } })
            )),
            Outcome::Unauthorized
        );
        assert_eq!(
            classify(&exit(
                json!({ "_tag": "Fail", "error": { "_tag": "ProjectReadFileError" } })
            )),
            Outcome::Fail
        );
    }

    /// `outcome="fail"` and `"refused"` are each one bucket for every declared
    /// error a method has, so the error type on the span is what makes them
    /// readable.
    #[test]
    fn a_typed_failure_carries_the_tag_the_client_was_given() {
        assert_eq!(
            fail_tag(&exit(json!({
                "_tag": "Fail",
                "error": { "_tag": "AssetWorkspaceAssetNotFoundError" }
            }))),
            Some("AssetWorkspaceAssetNotFoundError")
        );
        assert_eq!(
            fail_tag(&exit(json!({ "_tag": "Die", "defect": "boom" }))),
            None
        );
    }

    #[test]
    fn an_interrupt_is_not_a_failure() {
        let outcome = classify(&exit(json!({ "_tag": "Interrupt" })));
        assert_eq!(outcome, Outcome::Interrupted);
        assert!(!outcome.is_failure());
    }

    /// A `Defect` frame answers the socket, not the request, so it is what the
    /// pre-dispatch id checks emit — and the tag beside it was never matched.
    #[test]
    fn a_defect_frame_is_rejected_and_does_not_label_with_its_tag() {
        let frames = vec![json!({ "_tag": "Defect", "defect": "Duplicate id" })];
        let outcome = classify(&frames);
        assert_eq!(outcome, Outcome::Rejected);
        assert_eq!(outcome.tag_label(None), "rejected");
        assert_eq!(defect(&frames), Some("Duplicate id"));
    }

    #[test]
    fn an_unsupported_method_does_not_label_with_its_tag() {
        assert_eq!(
            Outcome::Unsupported.tag_label(Some("orchestration.replayEvents")),
            "unsupported"
        );
        assert_eq!(
            Outcome::Fail.tag_label(Some("projects.readFile")),
            "projects.readFile"
        );
    }

    /// The outcome and span status [`answered`] recorded for `request`.
    async fn answered_outcome(
        request: impl Future<Output = Vec<Value>>,
    ) -> (String, opentelemetry::trace::Status) {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);
        {
            let span = request_root_span("projects.readFile", None, None, None);
            answered(&span, Some("projects.readFile"), &viewer(), request).await;
        }
        provider.force_flush().expect("flush the simple exporter");
        let spans = exporter.get_finished_spans().expect("exporter is readable");
        let outcome = spans[0]
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "outcome")
            .map(|kv| kv.value.as_str().to_string())
            .expect("every answered request records an outcome");
        (outcome, spans[0].status.clone())
    }

    fn read_file_error() -> Vec<Value> {
        exit(json!({ "_tag": "Fail", "error": { "_tag": "ProjectReadFileError" } }))
    }

    /// `fail` is kept for what went wrong on this side, so a missing file the
    /// handler marked counts apart from it and is not a span error.
    #[tokio::test]
    async fn a_fail_its_handler_marked_refused_is_refused() {
        let (outcome, status) = answered_outcome(async {
            refused();
            read_file_error()
        })
        .await;
        assert_eq!(outcome, "refused");
        assert_ne!(status, opentelemetry::trace::Status::error(""));

        let (outcome, _) = answered_outcome(async { read_file_error() }).await;
        assert_eq!(outcome, "fail", "unmarked, a typed failure is a failure");
    }

    /// The mark qualifies a `Fail`; it cannot hide a defect.
    #[tokio::test]
    async fn a_die_stays_a_die_when_marked_refused() {
        let (outcome, _) = answered_outcome(async {
            refused();
            exit(json!({ "_tag": "Die", "defect": "boom" }))
        })
        .await;
        assert_eq!(outcome, "die");
    }

    /// A connection at its stream cap is told with a `Die`, since no error
    /// every stream declares fits, but the server is not broken.
    #[tokio::test]
    async fn a_die_marked_capped_is_capped() {
        let (outcome, status) = answered_outcome(async {
            capped();
            exit(json!({ "_tag": "Die", "defect": "cap" }))
        })
        .await;
        assert_eq!(outcome, "capped");
        assert_ne!(status, opentelemetry::trace::Status::error(""));
    }

    /// The mark qualifies a `Die`; a typed failure keeps its own outcome.
    #[tokio::test]
    async fn a_fail_stays_a_fail_when_marked_capped() {
        let (outcome, _) = answered_outcome(async {
            capped();
            read_file_error()
        })
        .await;
        assert_eq!(outcome, "fail");
    }

    #[test]
    fn marking_refused_outside_a_request_does_nothing() {
        refused();
    }

    /// `duration_ms` remains the handler duration when a later span entry
    /// stretches the exported span duration.
    #[test]
    fn a_span_reentered_past_the_handler_stretches_but_duration_ms_does_not() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let holder = {
            let span = request_root_span("shell.list", None, None, None);
            let entered = span.enter();
            record_request(
                &span,
                Some("shell.list"),
                &[json!({
                    "_tag": "Exit",
                    "requestId": "1",
                    "exit": { "_tag": "Success", "value": {} }
                })],
                Mark::None,
                Duration::from_millis(7),
                &viewer(),
            );
            drop(entered);
            span.clone()
        };
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            exporter
                .get_finished_spans()
                .expect("exporter is readable")
                .is_empty(),
            "a live handle keeps the span open"
        );
        // What a task polled after the handler answered does, once.
        drop(holder.enter());
        drop(holder);
        provider.force_flush().expect("flush the simple exporter");

        let spans = exporter.get_finished_spans().expect("exporter is readable");
        assert_eq!(spans.len(), 1);
        let exported = spans[0]
            .end_time
            .duration_since(spans[0].start_time)
            .expect("a span ends after it starts");
        assert!(
            exported >= Duration::from_millis(25),
            "the exported span carries the re-entry, not the handler: {exported:?}"
        );
        let recorded = spans[0]
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "duration_ms")
            .map(|kv| kv.value.clone());
        assert!(
            matches!(recorded, Some(opentelemetry::Value::F64(ms)) if (ms - 7.0).abs() < 0.001),
            "duration_ms is what the handler took: {recorded:?}"
        );
    }
}
