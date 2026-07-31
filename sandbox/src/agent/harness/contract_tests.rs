//! Contract tests for the harness command/signal seam.
//!
//! These drive the external [`Harness`] interface with the scripted mock process
//! — the same interface the coordinator uses — and assert the ordering and
//! terminal invariants every harness must satisfy.

use std::sync::Arc;
use std::time::Duration;

use moatless_api_schemas::content::CanonicalContentBlock;
use tokio::sync::mpsc;

use super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use super::scripted::ScriptedProcess;
use super::source::SourceId;
use super::*;

fn assistant_block(source: &str, message: &str, session: &str, text: &str) -> SourceEvent {
    SourceEvent {
        source_id: SourceId(source.to_string()),
        message_id: NativeMessageId(message.to_string()),
        session: SessionId(session.to_string()),
        parent_tool_use_id: None,
        role: Role::Assistant,
        block: CanonicalContentBlock::Text {
            text: text.to_string(),
        },
        tool_use_result: None,
        usage: None,
        model: None,
        diagnostics: None,
    }
}

fn failure(class: ErrorClass) -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Unknown,
        "scripted failure",
    )
}

/// Drain signals until (and including) the first terminal — `ExecutionFinished`
/// or `InputRejected` — or until the stream closes.
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
async fn new_execution_ordering_started_accepted_source_finished() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
        Step::Block(assistant_block("s1", "m1", "sess-1", "hello")),
        Step::Block(assistant_block("s2", "m1", "sess-1", "world")),
        Step::Completed,
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;

    assert!(matches!(
        signals[0],
        Signal::ExecutionStarted {
            cause: Cause::Input(ref id),
            ref session,
        } if id.0 == "in-1" && session.0 == "sess-1"
    ));
    assert!(matches!(signals[1], Signal::InputAccepted { ref input } if input.0 == "in-1"));
    assert!(matches!(signals[2], Signal::Source(ref e) if e.source_id.0 == "s1"));
    assert!(matches!(signals[3], Signal::Source(ref e) if e.source_id.0 == "s2"));
    assert!(matches!(
        signals[4],
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
    assert_eq!(signals.len(), 5);
}

#[tokio::test]
async fn no_source_event_before_execution_start() {
    // Blocks arrive before SessionReady: they must be buffered and released in
    // native order only after ExecutionStarted.
    let script = Script::new(vec![
        Step::Block(assistant_block("s1", "m1", "sess-1", "early-one")),
        Step::Block(assistant_block("s2", "m1", "sess-1", "early-two")),
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
        Step::Completed,
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;

    // Contract order holds even though the process emitted blocks first:
    // ExecutionStarted, then InputAccepted, then the buffered sources — no
    // Source precedes the start or the acceptance.
    assert!(matches!(signals[0], Signal::ExecutionStarted { .. }));
    assert!(matches!(signals[1], Signal::InputAccepted { .. }));
    assert!(matches!(signals[2], Signal::Source(ref e) if e.source_id.0 == "s1"));
    assert!(matches!(signals[3], Signal::Source(ref e) if e.source_id.0 == "s2"));
    assert!(matches!(
        signals[4],
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
}

#[tokio::test]
async fn terminal_before_acceptance_drops_buffered_source() {
    // The execution starts and buffers a block, then terminates without its input
    // ever being accepted. A terminal never synthesizes acceptance, so the
    // buffered block is dropped rather than emitted out of contract order: no
    // InputAccepted and no Source appear.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::Block(assistant_block("s1", "m1", "sess-1", "never-anchored")),
        Step::Completed,
    ]);
    let mut harness = Harness::scripted(script);
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
async fn pre_start_submit_failure_rejects_input() {
    let mut harness = Harness::scripted(Script::rejecting(HarnessError::new(
        FailureStage::Startup,
        ErrorClass::Authentication,
        SessionImpact::Unknown,
        "bad credentials",
    )));
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;
    assert_eq!(signals.len(), 1);
    match &signals[0] {
        Signal::InputRejected {
            input,
            reason: Rejection::Failed(err),
        } => {
            assert_eq!(input.0, "in-1");
            assert_eq!(err.class, ErrorClass::Authentication);
        }
        other => panic!("expected InputRejected(Failed), got {other:?}"),
    }
}

#[tokio::test]
async fn started_execution_failure_finishes_once() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
        Step::Failed(failure(ErrorClass::NativeProtocol)),
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;
    match signals.last().expect("terminal") {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(err),
        } => assert_eq!(err.class, ErrorClass::NativeProtocol),
        other => panic!("expected ExecutionFinished(Failed), got {other:?}"),
    }
    // Exactly one terminal.
    let terminals = signals
        .iter()
        .filter(|s| matches!(s, Signal::ExecutionFinished { .. }))
        .count();
    assert_eq!(terminals, 1);
}

#[tokio::test]
async fn unexpected_eof_after_start_is_a_process_exit() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
        Step::Block(assistant_block("s1", "m1", "sess-1", "partial")),
        Step::Eof,
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;
    match signals.last().expect("terminal") {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(err),
        } => assert_eq!(err.class, ErrorClass::ProcessExit),
        other => panic!("expected ExecutionFinished(Failed ProcessExit), got {other:?}"),
    }
}

#[tokio::test]
async fn cancellation_during_output_finishes_cancelled_and_is_idempotent() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
        Step::Block(assistant_block("s1", "m1", "sess-1", "working")),
        Step::WaitForCancel,
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    // Read up to the buffered source, then cancel (twice — idempotent).
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
    harness.cancel();

    let terminal = harness.next_signal().await.expect("terminal");
    assert!(matches!(
        terminal,
        Signal::ExecutionFinished {
            outcome: Outcome::Cancelled
        }
    ));
}

#[tokio::test]
async fn cancellation_during_startup_rejects_input() {
    // No SessionReady precedes the cancel wait, so the execution never starts.
    let script = Script::new(vec![Step::WaitForCancel]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    // Give the driver a moment to submit into the process and begin awaiting.
    tokio::task::yield_now().await;
    harness.cancel();

    let signals = drain_to_terminal(&mut harness).await;
    assert_eq!(signals.len(), 1);
    assert!(matches!(
        signals[0],
        Signal::InputRejected {
            reason: Rejection::Cancelled,
            ..
        }
    ));
}

/// A Stop must land while the process is still *opening*, not only once it is
/// waiting for the Agent's first event. Codex awaits `initialize`,
/// `thread/start` and `turn/start` inside `submit`, and a server that stops
/// answering parks that future indefinitely — so if the driver read `cancel_rx`
/// only after `submit` returned, Stop would be unreachable exactly when it is
/// needed.
#[tokio::test]
async fn cancellation_during_a_blocking_open_rejects_the_input() {
    let recorder: scripted::InputRecorder = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    // This script's `submit` never returns on its own; only the cancel handle
    // ends it.
    let mut harness = Harness::scripted_recording(Script::blocking_open(), Arc::clone(&recorder));
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    // Cancel only once the process is provably inside the open. Cancelling
    // earlier would race the idle driver, which drains an idle cancel as a
    // no-op, and the open would then park forever.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while recorder.lock().await.is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the opening input never reached the process"
        );
        tokio::task::yield_now().await;
    }
    harness.cancel();

    let signals = drain_to_terminal(&mut harness).await;
    assert_eq!(
        signals.len(),
        1,
        "an open that never started has no terminal"
    );
    assert!(
        matches!(
            signals[0],
            Signal::InputRejected {
                reason: Rejection::Cancelled,
                ..
            }
        ),
        // The scripted open reports a `ProcessExit` failure, because that is
        // what a torn-down transport sees. The driver knows it asked for the
        // teardown, so the input is cancelled rather than failed — otherwise a
        // user Stop would publish an error to the conversation.
        "a cancelled open is cancelled, not failed: {:?}",
        signals[0]
    );
}

#[tokio::test]
async fn cancellation_while_idle_is_a_noop() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
        Step::Completed,
    ]);
    let mut harness = Harness::scripted(script);

    // Cancel before any input: must not affect the later execution.
    harness.cancel();
    tokio::task::yield_now().await;

    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    let signals = drain_to_terminal(&mut harness).await;
    assert!(matches!(
        signals.last(),
        Some(Signal::ExecutionFinished {
            outcome: Outcome::Completed
        })
    ));
}

#[tokio::test]
async fn backpressure_delivers_every_signal_in_order() {
    // Signal capacity of 2 is far smaller than the signal count, so the driver
    // must block on send rather than drop anything.
    let mut steps = vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
    ];
    for i in 0..10 {
        steps.push(Step::Block(assistant_block(
            &format!("s{i}"),
            "m1",
            "sess-1",
            &format!("block-{i}"),
        )));
    }
    steps.push(Step::Completed);

    let mut harness = Harness::scripted_with_signal_capacity(Script::new(steps), 2);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    // Consume slowly to force backpressure.
    let mut sources = Vec::new();
    let mut finished = false;
    while let Some(signal) = harness.next_signal().await {
        tokio::task::yield_now().await;
        match signal {
            Signal::Source(e) => sources.push(e.source_id.0),
            Signal::ExecutionFinished { .. } => {
                finished = true;
                break;
            }
            _ => {}
        }
    }
    assert!(finished);
    let expected: Vec<String> = (0..10).map(|i| format!("s{i}")).collect();
    assert_eq!(sources, expected);
}

// ── Driver-level closed-channel coverage ────────────────────────────────────

#[tokio::test]
async fn driver_stops_when_signal_consumer_drops() {
    let (submit_tx, submit_rx) = mpsc::channel::<Input>(4);
    let (_cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
    let (signal_tx, signal_rx) = mpsc::channel::<Signal>(4);

    let mut first = Some(Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::Block(assistant_block("s1", "m1", "sess-1", "hi")),
        Step::Completed,
    ]));
    let factory = move || ScriptedProcess::new(first.take().unwrap_or_default());
    let handle = tokio::spawn(super::driver::run(factory, submit_rx, cancel_rx, signal_tx));

    // No consumer for signals: the first emit must fail and stop the driver
    // instead of discarding output silently.
    drop(signal_rx);
    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit accepted");

    handle.await.expect("driver task joins");
    assert!(submit_tx.is_closed(), "driver dropped its submit receiver");
}

#[tokio::test]
async fn driver_stops_when_command_side_closes_while_idle() {
    let (submit_tx, submit_rx) = mpsc::channel::<Input>(4);
    let (_cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
    let (signal_tx, _signal_rx) = mpsc::channel::<Signal>(4);

    let factory = || ScriptedProcess::new(Script::default());
    let handle = tokio::spawn(super::driver::run(factory, submit_rx, cancel_rx, signal_tx));

    // Close the command side with nothing queued: the driver stops cleanly.
    drop(submit_tx);
    handle.await.expect("driver task joins");
}

// ── Harness-neutral coordinator ─────────────────────────────────────────────

/// A minimal stand-in for the coordinator, proving the harness owns no Run
/// identity or acknowledgement: the Run is minted only on `ExecutionStarted`,
/// and input is acknowledged only after `InputAccepted`, against that Run.
#[derive(Default)]
struct NeutralCoordinator {
    run_id: Option<String>,
    session: Option<String>,
    acks: Vec<(String, String, String)>,
    finished: bool,
}

impl NeutralCoordinator {
    fn observe(&mut self, signal: &Signal) {
        match signal {
            Signal::ExecutionStarted { session, .. } => {
                self.run_id = Some(uuid::Uuid::new_v4().to_string());
                self.session = Some(session.0.clone());
            }
            Signal::InputAccepted { input } => {
                let run = self
                    .run_id
                    .clone()
                    .expect("Run must be minted before acknowledging input");
                let session = self
                    .session
                    .clone()
                    .expect("Session must be known before acknowledging input");
                self.acks.push((input.0.clone(), session, run));
            }
            Signal::ExecutionFinished { .. } => self.finished = true,
            _ => {}
        }
    }
}

#[tokio::test]
async fn coordinator_mints_run_after_start_and_acks_after_acceptance() {
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::InputAccepted(InputId("in-1".into())),
        Step::Block(assistant_block("s1", "m1", "sess-1", "hello")),
        Step::Completed,
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "hi"))
        .await
        .expect("submit");

    let mut coordinator = NeutralCoordinator::default();
    // Before consuming any signal, no Run exists.
    assert!(coordinator.run_id.is_none());

    while let Some(signal) = harness.next_signal().await {
        let terminal = matches!(signal, Signal::ExecutionFinished { .. });
        coordinator.observe(&signal);
        if terminal {
            break;
        }
    }

    assert!(coordinator.run_id.is_some(), "Run minted");
    assert!(coordinator.finished);
    let run = coordinator.run_id.clone().unwrap();
    assert_eq!(
        coordinator.acks,
        vec![("in-1".to_string(), "sess-1".to_string(), run)]
    );
}

#[tokio::test]
async fn input_submitted_mid_execution_joins_it_when_the_process_injects() {
    // The one-execution-per-process driver hands mid-execution input to a
    // process that injects, so it lands on the running execution's Turn instead
    // of waiting for the next one. Exactly one `ExecutionStarted` is emitted.
    let script = Script::injecting(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::AwaitInput,
        Step::AcceptInput,
        Step::Completed,
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "first"))
        .await
        .expect("submit");
    harness
        .submit(Input::text("in-2", "injected"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;

    assert!(matches!(
        signals[0],
        Signal::ExecutionStarted {
            cause: Cause::Input(ref id),
            ..
        } if id.0 == "in-1"
    ));
    assert!(matches!(signals[1], Signal::InputAccepted { ref input } if input.0 == "in-1"));
    assert!(matches!(signals[2], Signal::InputAccepted { ref input } if input.0 == "in-2"));
    assert!(matches!(
        signals[3],
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
    assert_eq!(
        signals.len(),
        4,
        "the injected input opens no second execution"
    );
}

#[tokio::test]
async fn input_submitted_after_cancel_is_queued_rather_than_injected() {
    // A cancelled execution finishes `Cancelled` whatever the process reports, so
    // input arriving while it unwinds must not join it — it opens the next
    // execution instead, even though the process would accept an injection.
    let mut harness = Harness::scripted_sequence(vec![
        Script::injecting(vec![
            Step::SessionReady(SessionId("sess-1".into())),
            Step::AcceptInput,
            Step::WaitForCancel,
        ]),
        Script::injecting(vec![
            Step::SessionReady(SessionId("sess-2".into())),
            Step::AcceptInput,
            Step::Completed,
        ]),
    ]);
    harness
        .submit(Input::text("in-1", "first"))
        .await
        .expect("submit");

    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::ExecutionStarted { .. })
    ));
    assert!(matches!(
        harness.next_signal().await,
        Some(Signal::InputAccepted { ref input }) if input.0 == "in-1"
    ));

    harness.cancel();
    harness
        .submit(Input::text("in-2", "after cancel"))
        .await
        .expect("submit");

    // The cancelled execution's terminal comes first, with no acceptance of the
    // late input on it.
    let cancelled = drain_to_terminal(&mut harness).await;
    assert_eq!(
        cancelled.len(),
        1,
        "the late input must not join the cancelled execution, got {cancelled:?}"
    );
    assert!(matches!(
        cancelled[0],
        Signal::ExecutionFinished {
            outcome: Outcome::Cancelled
        }
    ));

    // Bounded: without the guard the input is injected into the dying execution
    // and never opens one of its own, so this would otherwise hang rather than
    // fail.
    let recovered = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        drain_to_terminal(&mut harness),
    )
    .await
    .expect("the late input must open a follow-up execution");
    assert!(matches!(
        recovered[0],
        Signal::ExecutionStarted {
            cause: Cause::Input(ref id),
            ..
        } if id.0 == "in-2"
    ));
    assert!(matches!(recovered[1], Signal::InputAccepted { ref input } if input.0 == "in-2"));
    assert!(matches!(
        recovered[2],
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
}

#[tokio::test]
async fn input_submitted_mid_execution_is_queued_when_the_process_cannot_inject() {
    // A process that opens a fresh execution per submit must not be handed
    // mid-execution input: it is queued and opens the next execution instead.
    // No `AwaitInput` gate: the process never receives `in-2`, so waiting on it
    // would block forever — which is exactly the behaviour under test.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".into())),
        Step::AcceptInput,
        Step::Completed,
    ]);
    let mut harness = Harness::scripted(script);
    harness
        .submit(Input::text("in-1", "first"))
        .await
        .expect("submit");
    harness
        .submit(Input::text("in-2", "queued"))
        .await
        .expect("submit");

    let signals = drain_to_terminal(&mut harness).await;

    assert!(matches!(signals[1], Signal::InputAccepted { ref input } if input.0 == "in-1"));
    assert!(
        !signals
            .iter()
            .any(|s| matches!(s, Signal::InputAccepted { input } if input.0 == "in-2")),
        "the second input must not join the first execution"
    );
}
