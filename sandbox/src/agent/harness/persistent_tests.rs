//! Contract tests for the persistent-process seam (`driver::run_persistent`).
//!
//! These drive the external [`Harness`] interface with a scripted *persistent*
//! process — one long-lived process hosting many executions — and assert the
//! behaviors ticket 06 requires: mid-execution injection, fingerprint
//! replacement, harness-internal executions, cancellation, EOF, and queued
//! input. The coordinator sees only the same [`Signal`] contract as the one-shot
//! path; nothing here is tmux-specific.

use super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use super::source::SourceId;
use super::*;

fn assistant_block(source: &str, session: &str, text: &str) -> SourceEvent {
    SourceEvent {
        source_id: SourceId(source.to_string()),
        message_id: NativeMessageId(format!("m-{source}")),
        session: SessionId(session.to_string()),
        parent_tool_use_id: None,
        role: Role::Assistant,
        block: serde_json::json!({ "type": "text", "text": text }),
        tool_use_result: None,
        usage: None,
        model: None,
        diagnostics: None,
    }
}

/// Drain signals up to and including the next terminal (`ExecutionFinished` /
/// `InputRejected`), or until the stream closes.
async fn drain_to_terminal(harness: &mut Harness) -> Vec<Signal> {
    let mut out = Vec::new();
    while let Some(signal) = harness.next_signal().await {
        let terminal = matches!(
            signal,
            Signal::ExecutionFinished { .. } | Signal::InputRejected { .. }
        );
        out.push(signal);
        if terminal {
            break;
        }
    }
    out
}

#[tokio::test]
async fn injection_accepts_into_active_execution_without_new_start() {
    // I1 opens an execution; I2 is injected while it is still active. I2 must be
    // accepted with no second ExecutionStarted, joining the current execution.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s1", "sess-1", "first")),
        Step::AwaitInput,
        Step::AcceptInput,
        Step::Block(assistant_block("s2", "sess-1", "second")),
        Step::ExecutionEnded,
    ]);
    let mut harness = Harness::scripted_persistent(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    // The execution opens and streams its first block, then blocks on injection.
    let started = harness.next_signal().await.expect("started");
    assert!(matches!(
        started,
        Signal::ExecutionStarted { cause: Cause::Input(ref id), .. } if id.0 == "in-1"
    ));
    assert!(
        matches!(harness.next_signal().await, Some(Signal::InputAccepted { ref input }) if input.0 == "in-1")
    );
    assert!(
        matches!(harness.next_signal().await, Some(Signal::Source(ref e)) if e.source_id.0 == "s1")
    );

    // Inject a second input mid-execution.
    harness
        .submit(Input::text("in-2", "more"))
        .await
        .expect("submit");

    let rest = drain_to_terminal(&mut harness).await;
    // Acceptance of the injected input, no new start, then its source and the
    // single terminal.
    assert!(matches!(rest[0], Signal::InputAccepted { ref input } if input.0 == "in-2"));
    assert!(matches!(rest[1], Signal::Source(ref e) if e.source_id.0 == "s2"));
    assert!(matches!(
        rest[2],
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
    assert!(
        !rest
            .iter()
            .any(|s| matches!(s, Signal::ExecutionStarted { .. })),
        "injection must not open a new execution"
    );
    assert_eq!(rest.len(), 3);
}

#[tokio::test]
async fn terminal_before_acceptance_drops_buffered_source() {
    // A persistent execution starts and buffers a block, then reaches its Turn
    // boundary without the opening input ever being accepted. The terminal must
    // not synthesize acceptance, so the buffered block is dropped: no
    // InputAccepted and no Source appear.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::Block(assistant_block("s1", "sess-1", "never-anchored")),
        Step::ExecutionEnded,
    ]);
    let mut harness = Harness::scripted_persistent(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;

    assert!(matches!(signals[0], Signal::ExecutionStarted { .. }));
    assert!(
        !signals
            .iter()
            .any(|s| matches!(s, Signal::InputAccepted { .. })),
        "no acceptance was emitted, so none must be synthesized: {signals:?}"
    );
    assert!(
        !signals.iter().any(|s| matches!(s, Signal::Source(_))),
        "unanchored buffered source must be dropped, not emitted: {signals:?}"
    );
    assert!(matches!(
        signals.last().expect("terminal"),
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
}

#[tokio::test]
async fn incompatible_input_opens_replacement_execution_on_the_intended_session() {
    // I1 runs on sess-1; I2 is incompatible, so the process replaces itself and
    // resumes as sess-2. I2 is accepted only against the replacement Run.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s1", "sess-1", "first")),
        Step::AwaitInput,
        Step::ExecutionEnded,
        Step::SessionReady(SessionId("sess-2".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s2", "sess-2", "second")),
        Step::ExecutionEnded,
    ]);
    let mut harness = Harness::scripted_persistent(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    // First execution up to its streamed block, while it is still awaiting the
    // next input.
    let started = harness.next_signal().await.expect("started");
    assert!(matches!(
        started,
        Signal::ExecutionStarted { cause: Cause::Input(ref id), ref session } if id.0 == "in-1" && session.0 == "sess-1"
    ));
    assert!(
        matches!(harness.next_signal().await, Some(Signal::InputAccepted { ref input }) if input.0 == "in-1")
    );
    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::Source(_))
    ));

    // The incompatible input arrives; the first execution completes before the
    // replacement starts.
    harness
        .submit(Input::text("in-2", "switch"))
        .await
        .expect("submit");
    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::ExecutionFinished {
            outcome: Outcome::Completed
        })
    ));

    let second = drain_to_terminal(&mut harness).await;
    assert!(matches!(
        second[0],
        Signal::ExecutionStarted { cause: Cause::Input(ref id), ref session } if id.0 == "in-2" && session.0 == "sess-2"
    ));
    assert!(matches!(second[1], Signal::InputAccepted { ref input } if input.0 == "in-2"));
    assert!(matches!(second[2], Signal::Source(ref e) if e.session.0 == "sess-2"));
    assert!(matches!(
        second[3],
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
}

#[tokio::test]
async fn internal_execution_starts_without_input_or_acknowledgement() {
    // A CLI-initiated (wakeup/Monitor) execution: ExecutionStarted with an
    // internal cause, source output, terminal — and no InputAccepted at all.
    let script = Script::new(vec![
        Step::InternalStart(SessionId("sess-1".into())),
        Step::Block(assistant_block("s1", "sess-1", "self-started")),
        Step::ExecutionEnded,
    ]);
    let mut harness = Harness::scripted_persistent(script);

    let signals = drain_to_terminal(&mut harness).await;
    assert!(matches!(
        signals[0],
        Signal::ExecutionStarted { cause: Cause::Internal, ref session } if session.0 == "sess-1"
    ));
    assert!(matches!(signals[1], Signal::Source(ref e) if e.source_id.0 == "s1"));
    assert!(matches!(
        signals[2],
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
    assert!(
        !signals
            .iter()
            .any(|s| matches!(s, Signal::InputAccepted { .. })),
        "an internal execution acknowledges no pending Message"
    );
}

#[tokio::test]
async fn cancellation_ends_execution_and_process_persists_for_the_next() {
    // Cancel ends the active execution with a single Cancelled terminal; the
    // process survives and runs a later execution.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s1", "sess-1", "working")),
        Step::WaitForCancelThenEnd,
        Step::AwaitInput,
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s2", "sess-1", "after-cancel")),
        Step::ExecutionEnded,
    ]);
    let mut harness = Harness::scripted_persistent(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::ExecutionStarted { .. })
    ));
    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::InputAccepted { .. })
    ));
    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::Source(_))
    ));

    harness.cancel();
    harness.cancel(); // idempotent
    let terminal = harness.next_signal().await.expect("terminal");
    assert!(matches!(
        terminal,
        Signal::ExecutionFinished {
            outcome: Outcome::Cancelled
        }
    ));

    // The process persisted: a later input runs a fresh execution.
    harness
        .submit(Input::text("in-2", "again"))
        .await
        .expect("submit");
    let second = drain_to_terminal(&mut harness).await;
    assert!(matches!(
        second[0],
        Signal::ExecutionStarted { cause: Cause::Input(ref id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(
        second.last(),
        Some(Signal::ExecutionFinished {
            outcome: Outcome::Completed
        })
    ));
}

#[tokio::test]
async fn queued_input_opens_the_next_execution_and_is_acked_against_it() {
    // A second input submitted while the first execution is active is not
    // injected here; it waits and opens the next execution, acknowledged against
    // the next Run.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s1", "sess-1", "first")),
        Step::AwaitInput,
        Step::ExecutionEnded,
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s2", "sess-1", "second")),
        Step::ExecutionEnded,
    ]);
    let mut harness = Harness::scripted_persistent(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    // First execution up to its streamed block, then queue the second input.
    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::ExecutionStarted { .. })
    ));
    assert!(
        matches!(harness.next_signal().await, Some(Signal::InputAccepted { ref input }) if input.0 == "in-1")
    );
    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::Source(_))
    ));
    harness
        .submit(Input::text("in-2", "queued"))
        .await
        .expect("submit");

    // First execution finishes; the queued input is not yet acknowledged.
    let first_terminal = harness.next_signal().await.expect("terminal");
    assert!(matches!(
        first_terminal,
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));

    let second = drain_to_terminal(&mut harness).await;
    assert!(matches!(
        second[0],
        Signal::ExecutionStarted { cause: Cause::Input(ref id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(second[1], Signal::InputAccepted { ref input } if input.0 == "in-2"));
    assert!(matches!(
        second.last(),
        Some(Signal::ExecutionFinished {
            outcome: Outcome::Completed
        })
    ));
}

#[tokio::test]
async fn eof_mid_execution_is_a_process_exit() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Block(assistant_block("s1", "sess-1", "partial")),
        Step::Eof,
    ]);
    let mut harness = Harness::scripted_persistent(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;
    match signals.last().expect("terminal") {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(err),
        } => {
            assert_eq!(err.class, ErrorClass::ProcessExit);
        }
        other => panic!("expected ExecutionFinished(Failed ProcessExit), got {other:?}"),
    }
}

#[tokio::test]
async fn started_execution_stream_failure_finishes_failed_once() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Failed(HarnessError::new(
            FailureStage::Execution,
            ErrorClass::NativeProtocol,
            SessionImpact::Unknown,
            "scripted stream failure",
        )),
    ]);
    let mut harness = Harness::scripted_persistent(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;
    let terminals = signals
        .iter()
        .filter(|s| matches!(s, Signal::ExecutionFinished { .. }))
        .count();
    assert_eq!(terminals, 1, "exactly one terminal");
    match signals.last().expect("terminal") {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(err),
        } => {
            assert_eq!(err.class, ErrorClass::NativeProtocol);
        }
        other => panic!("expected ExecutionFinished(Failed), got {other:?}"),
    }
}
