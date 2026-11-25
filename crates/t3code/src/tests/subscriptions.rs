//! Opening, resuming and ending a subscription, and the slot cap that bounds
//! how many one connection holds.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn shell_subscription_chunks_snapshot_and_marker_then_waits_for_ack() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "2",
                "orchestration.subscribeShell",
                json!({ "requestCompletionMarker": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Chunk");
    assert_eq!(output.frames[0]["values"][0]["kind"], "snapshot");
    assert_eq!(output.frames[0]["values"][1]["kind"], "synchronized");
    assert_eq!(session.streams.awaiting_ack("2"), Some(true));

    let ack = session
        .handle_json(br#"{"_tag":"Ack","requestId":"2"}"#, &state(), &viewer())
        .await;
    assert!(ack.frames.is_empty());
    assert_eq!(session.streams.awaiting_ack("2"), Some(false));
}

/// A resuming client is not sent the listing again; the completion marker alone
/// arrives, so it stops showing as synchronizing. The stub's position 0 and
/// empty feed model a client up to date.
#[tokio::test]
async fn a_resuming_subscription_chunks_no_snapshot() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "5",
                "orchestration.subscribeShell",
                json!({ "afterSequence": 0, "requestCompletionMarker": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Chunk");
    let values = output.frames[0]["values"].as_array().unwrap();
    assert_eq!(
        values.len(),
        1,
        "a caught-up client should be sent the marker alone, got {values:#?}"
    );
    assert_eq!(values[0]["kind"], "synchronized");
}

/// `afterSequence` is `NonNegativeInt`, so no conforming client sends a
/// negative or string value. Absent, it rebuilds via listing; as a position, it
/// would resume from nothing allocated.
#[test]
fn an_after_sequence_the_contract_refuses_is_read_as_absent() {
    let after_sequence = |payload: Value| {
        serde_json::from_value::<OrchestrationV2SubscribeShellInput>(payload)
            .expect("an afterSequence never fails the decode")
            .after_sequence
    };
    assert_eq!(after_sequence(json!({ "afterSequence": 7 })), Some(7));
    assert_eq!(after_sequence(json!({ "afterSequence": 0 })), Some(0));
    assert_eq!(after_sequence(json!({})), None);
    assert_eq!(after_sequence(json!({ "afterSequence": null })), None);
    assert_eq!(after_sequence(json!({ "afterSequence": -1 })), None);
    assert_eq!(after_sequence(json!({ "afterSequence": "7" })), None);
    assert_eq!(after_sequence(json!({ "afterSequence": 1.5 })), None);
}

/// A conversation this viewer cannot see ends the subscription rather than
/// opening one that never produces a frame — a silent, open stream is
/// indistinguishable from a merely slow one.
#[tokio::test]
async fn a_thread_subscription_that_cannot_be_opened_fails_rather_than_hangs() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "6",
                "orchestration.subscribeThread",
                json!({ "threadId": "task-gone", "requestCompletionMarker": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(
        cause["error"]["_tag"],
        "OrchestrationV2GetThreadProjectionError"
    );
    assert!(!session.streams.contains("6"));
}

/// The same refusal for a returning client: a position does not skip the
/// access check.
#[tokio::test]
async fn a_resumed_thread_subscription_that_cannot_be_opened_fails() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "6",
                "orchestration.subscribeThread",
                json!({ "threadId": "task-gone", "afterSequence": 0 }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(
        cause["error"]["_tag"],
        "OrchestrationV2GetThreadProjectionError"
    );
    assert!(!session.streams.contains("6"));
}

#[tokio::test]
async fn a_thread_subscription_without_a_thread_is_refused() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("7", "orchestration.subscribeThread", json!({})),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
}

/// The stream's own ending, for a thread that stopped being readable while it
/// was open — the same refusal as the opening request's.
#[test]
fn a_subscription_ends_with_the_refusal_its_request_would_have_answered() {
    assert_eq!(
        sandbox_stream_refusal(&request_id("1"), Scope::OrchestrationRead),
        sandbox_failure_exit(
            &request_id("1"),
            Scope::OrchestrationRead,
            "sandbox.subscribeStatus",
            &SandboxRpcError::NotFound,
            &viewer(),
        )
    );
}

/// Terminal metadata is a served stream. It starts with an explicit empty
/// snapshot so the client can distinguish "no terminals yet" from a connection
/// that never produced a value. The seed is authorized per Task, so the
/// producer pushes it rather than the opening request returning it.
#[tokio::test]
async fn a_terminal_metadata_subscription_starts_with_a_snapshot() {
    let mut connection = connection();
    let output = connection
        .session
        .handle_json(
            &request("7", "subscribeTerminalMetadata", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert!(connection.session.streams.contains("7"));
    assert!(
        output.frames.is_empty(),
        "the frame loop did not wait on the seed's authorization reads"
    );
    let snapshot = connection.next_streamed().await;
    assert_eq!(snapshot["_tag"], "Chunk");
    assert_eq!(snapshot["values"][0]["type"], "snapshot");
    assert_eq!(snapshot["values"][0]["terminals"], json!([]));
}

/// A summary of a Task this viewer does not own is dropped from both halves of
/// the stream, a Task it only visits included; see `watch_terminal_metadata`.
#[tokio::test]
async fn terminal_metadata_carries_only_the_threads_the_viewer_owns() {
    let mut state = state();
    state.reads = state
        .reads
        .with_tasks(tasks_owned_and_visited("mine", "visited"));
    for (task, terminal) in [
        ("mine", "term-1"),
        ("visited", "term-2"),
        ("theirs", "term-3"),
    ] {
        state
            .terminal
            .apply_relayed_start(task, terminal, terminal_summary_for(task, terminal))
            .await;
    }

    let mut connection = connection();
    connection
        .session
        .handle_json(
            &request("7", "subscribeTerminalMetadata", json!({})),
            &state,
            &viewer(),
        )
        .await;

    let seed = connection.next_streamed().await;
    assert_eq!(
        seed["values"][0]["terminals"],
        json!([terminal_summary_for("mine", "term-1")]),
        "the seed carried a thread this viewer does not own"
    );

    // Announced in this order, so the owned one arriving first proves the
    // others were dropped rather than merely delayed.
    for (task, terminal) in [
        ("theirs", "term-4"),
        ("visited", "term-5"),
        ("mine", "term-6"),
    ] {
        state
            .terminal
            .apply_relayed_start(task, terminal, terminal_summary_for(task, terminal))
            .await;
    }

    let event = connection.next_streamed().await;
    assert_eq!(event["values"][0]["type"], "upsert");
    assert_eq!(event["values"][0]["terminal"]["terminalId"], "term-6");
}

#[tokio::test]
async fn interrupt_terminates_active_stream() {
    let mut session = session();
    // A subscription that stays open and says nothing — a thread with no
    // repository to report on.
    session
        .handle_json(
            &request("3", "subscribeVcsStatus", json!({})),
            &state(),
            &viewer(),
        )
        .await;
    assert!(session.streams.contains("3"));
    let output = session
        .handle_json(
            br#"{"_tag":"Interrupt","requestId":"3"}"#,
            &state(),
            &viewer(),
        )
        .await;
    assert!(!session.streams.contains("3"));
    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Interrupt");
}

#[tokio::test]
async fn a_subscription_that_ended_gives_its_slot_back() {
    let mut session = session();
    session
        .streams
        .start_produced(&request_id("3"), None, |_| async { Ok(()) });
    session.streams.wait_until_finished("3").await;

    session
        .handle_json(
            &request("4", "subscribeVcsStatus", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert!(!session.streams.contains("3"));
    assert!(session.streams.contains("4"));
}

#[tokio::test]
async fn a_connection_holding_live_subscriptions_is_refused_at_the_cap() {
    let mut session = session();
    for id in 0..MAX_ACTIVE_STREAMS {
        session
            .handle_json(
                &request(&id.to_string(), "subscribeVcsStatus", json!({})),
                &state(),
                &viewer(),
            )
            .await;
    }

    let output = session
        .handle_json(
            &request("9999", "subscribeVcsStatus", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
    assert_eq!(session.streams.len(), MAX_ACTIVE_STREAMS);
}

/// The client opens a stream per visible sidebar row, so reaching the cap is
/// its fan-out and must not count toward the `die` outcome that alerts.
#[tokio::test]
async fn a_subscription_refused_at_the_cap_is_counted_as_capped() {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::layer::SubscriberExt;

    let mut session = session();
    for id in 0..MAX_ACTIVE_STREAMS {
        session
            .handle_json(
                &request(&id.to_string(), "subscribeVcsStatus", json!({})),
                &state(),
                &viewer(),
            )
            .await;
    }
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(otel_layer));

    session
        .handle_json(
            &request("9999", "subscribeVcsStatus", json!({})),
            &state(),
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
    assert_eq!(outcome.as_deref(), Some("capped"));
}
