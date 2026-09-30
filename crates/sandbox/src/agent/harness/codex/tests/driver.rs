//! The one-shot lifecycle through the shared driver.

use super::fixtures::*;
use super::*;

/// A message that arrives while a Codex Turn runs is folded into that Turn, the
/// same as the Claude harness does, instead of waiting for a Turn of its own.
#[test]
fn the_codex_process_accepts_injection() {
    assert!(process().accepts_injection());
}

/// The window this covers is the whole of `open_turn` — `initialize`,
/// `thread/start`/`resume`, `turn/start` — none of which is bounded (see
/// `PendingRequests`).
///
/// The handle exists from construction and is the token
/// `codex_app_server::start` is handed, so cancelling it kills the child
/// mid-handshake, before any session exists.
#[tokio::test]
async fn cancelling_before_the_session_opens_still_tears_the_server_down() {
    let mut process = process();
    let handle = process.cancel_handle();
    assert!(
        !handle.is_cancelled(),
        "a fresh process is not already cancelled"
    );

    process.cancel().await;

    assert!(
        handle.is_cancelled(),
        "a cancel with no session yet must still fire the handle open_turn passes to start()"
    );
}

/// Spawn the shared one-shot driver over Codex processes backed by scripted
/// sessions (one per Turn), returning the command channels plus the per-Turn
/// event senders so a test can feed server events deterministically.
fn scripted_driver(
    sessions: Vec<AppServerSession>,
) -> (
    mpsc::Sender<Input>,
    mpsc::Sender<()>,
    mpsc::Receiver<Signal>,
) {
    let (submit_tx, submit_rx) = mpsc::channel(16);
    let (cancel_tx, cancel_rx) = mpsc::channel(1);
    let (signal_tx, signal_rx) = mpsc::channel(64);

    let cfg = config();
    let mut queue = std::collections::VecDeque::from(sessions);
    let factory = move || {
        let session = queue.pop_front().expect("a session per Turn");
        CodexProcess::with_session(cfg.clone(), session, "gpt-5")
    };
    tokio::spawn(crate::agent::harness::driver::run(
        factory, submit_rx, cancel_rx, signal_tx,
    ));

    (submit_tx, cancel_tx, signal_rx)
}

async fn next_signal(rx: &mut mpsc::Receiver<Signal>) -> Signal {
    timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("signal within 2s")
        .expect("harness still open")
}

#[tokio::test]
async fn fresh_turn_orders_started_accepted_source_finished() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.try_send(Ok(thread_started("thread-A"))).expect("feed");
    tx.try_send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_0", "text": "4" }),
    )))
    .expect("feed");
    tx.try_send(Ok(turn_completed("completed", None)))
        .expect("feed");

    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { session, cause: Cause::Input(id), .. }
            if session.0 == "thread-A" && id.0 == "in-1"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));
    // The agent block, then the usage-bearing turn-result block.
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionFinished {
            outcome: Outcome::Completed { .. }
        }
    ));
}

#[tokio::test]
async fn queued_input_opens_the_next_turn_and_is_accepted_against_it() {
    let (tx1, _s1, session1) = AppServerSession::scripted();
    let (tx2, _s2, session2) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session1, session2]);

    // First Turn starts.
    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx1.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-1"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));

    // Queue a second input while the first Turn is active.
    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");

    // First Turn terminates; its result block then a Completed terminal.
    tx1.try_send(Ok(turn_completed("completed", None)))
        .expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionFinished {
            outcome: Outcome::Completed { .. }
        }
    ));

    // Only now does the queued input open a replacement Turn and get accepted.
    tx2.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
}

/// A message submitted while the execution is open joins it, at the driver
/// seam.
///
/// One `ExecutionStarted`, two acceptances, and no second `SessionReady` — what
/// keeps a steered message on the running Turn's Run.
#[tokio::test]
async fn a_message_submitted_mid_execution_joins_it_and_opens_no_second_one() {
    let (tx, mut server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx.try_send(Ok(thread_started("thread-A"))).expect("feed");
    tx.try_send(Ok(turn_started("turn-1"))).expect("feed");
    tx.try_send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_0", "text": "working" }),
    )))
    .expect("feed");

    let mut signals = vec![
        next_signal(&mut signal_rx).await,
        next_signal(&mut signal_rx).await,
        // The block proves the Turn is running and `turn/started` was absorbed,
        // so the next submit takes the steer path rather than the held one.
        next_signal(&mut signal_rx).await,
    ];

    submit_tx
        .send(Input::text("in-2", "and BANANA"))
        .await
        .expect("submit 2");
    let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
    assert_eq!(request["method"], "turn/steer");

    tx.try_send(Ok(turn_completed("completed", None)))
        .expect("feed");
    loop {
        let signal = next_signal(&mut signal_rx).await;
        let terminal = matches!(signal, Signal::ExecutionFinished { .. });
        signals.push(signal);
        if terminal {
            break;
        }
    }

    let starts = signals
        .iter()
        .filter(|s| matches!(s, Signal::ExecutionStarted { .. }))
        .count();
    assert_eq!(starts, 1, "the steered message opens no second execution");
    let accepted: Vec<&str> = signals
        .iter()
        .filter_map(|s| match s {
            Signal::InputAccepted { input } => Some(input.0.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(accepted, vec!["in-1", "in-2"]);
    assert!(matches!(
        signals.last(),
        Some(Signal::ExecutionFinished {
            outcome: Outcome::Completed { .. }
        })
    ));
}

/// Messages arriving inside the steerability window are held and delivered in
/// submit order once `turn/started` lands, one acceptance each.
///
/// The window is real: `turn/start`'s response is too early to steer, and a
/// submit can land in it.
#[tokio::test]
async fn messages_held_for_the_gate_are_delivered_in_submit_order() {
    let (tx, mut server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));

    // Both arrive while the Turn is still opening: nothing may be steered yet.
    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");
    submit_tx
        .send(Input::text("in-3", "three"))
        .await
        .expect("submit 3");
    let early = timeout(Duration::from_millis(100), server.next_request()).await;
    assert!(early.is_err(), "nothing may be steered before turn/started");

    tx.try_send(Ok(turn_started("turn-1"))).expect("feed");
    for _ in 0..2 {
        let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
        assert_eq!(request["method"], "turn/steer");
    }

    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-3"
    ));
}

/// A Stop issued while a message is waiting on the gate lands then, not when
/// the wait gives up.
///
/// The driver's injection arm is the only reader of `cancel_rx`; unless it
/// races the Stop against `submit`, the wait runs to its bound with the Stop
/// unread, and the message survives to open the next Turn.
#[tokio::test]
async fn a_stop_reaches_the_driver_while_a_message_waits_on_the_gate() {
    let (tx1, mut server1, session1) = AppServerSession::scripted();
    let (tx2, _server2, session2) = AppServerSession::scripted();
    let (submit_tx, cancel_tx, mut signal_rx) = scripted_driver(vec![session1, session2]);
    drop(tx1);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    server1
        .write(json!({
            "method": "thread/started",
            "params": { "thread": { "id": "thread-A" } },
        }))
        .await;
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));

    // `turn/started` never arrives, so the driver is parked inside `submit`.
    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");
    let early = timeout(Duration::from_millis(100), server1.next_request()).await;
    assert!(early.is_err(), "nothing may be steered before turn/started");

    cancel_tx.send(()).await.expect("stop");

    // `next_signal` gives up after 2s, well inside the gate's own bound: a Stop
    // that only landed when the wait expired would not be seen here at all.
    assert!(
        matches!(
            next_signal(&mut signal_rx).await,
            Signal::ExecutionFinished {
                outcome: Outcome::Cancelled
            }
        ),
        "a Stop during a held message ends the execution without waiting out the gate"
    );

    // And the message it interrupted is still owed an answer.
    tx2.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
}

/// Every steer rejection is the same `-32600` with a different sentence, so the
/// harness never reads the sentence.
///
/// Any rejection means this Turn cannot take the message; it opens the next one
/// instead, and the running execution carries on untouched.
#[tokio::test]
async fn a_rejected_steer_leaves_the_message_for_the_next_turn() {
    let (tx1, mut server1, session1) = AppServerSession::scripted();
    let (tx2, _server2, session2) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session1, session2]);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx1.try_send(Ok(thread_started("thread-A"))).expect("feed");
    tx1.try_send(Ok(turn_started("turn-1"))).expect("feed");
    tx1.try_send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_0", "text": "working" }),
    )))
    .expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));

    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");
    let request = server1.next_request().await;
    assert_eq!(request["method"], "turn/steer");
    server1
        .write(json!({
            "id": request["id"],
            "error": { "code": -32600, "message": "no active turn to steer" },
        }))
        .await;

    // The running execution is undisturbed: it finishes on its own terms, with
    // no acceptance of the refused message against it.
    tx1.try_send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_1", "text": "done" }),
    )))
    .expect("feed");
    tx1.try_send(Ok(turn_completed("completed", None)))
        .expect("feed");
    loop {
        match next_signal(&mut signal_rx).await {
            Signal::Source(_) => continue,
            Signal::ExecutionFinished { outcome } => {
                assert!(matches!(outcome, Outcome::Completed { .. }));
                break;
            }
            other => panic!("the refused message must not join this Turn: {other:?}"),
        }
    }

    // And it opens the next Turn on the same thread instead of being lost.
    tx2.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
}

#[tokio::test]
async fn cancellation_finishes_the_turn_as_cancelled() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    // Request cancellation, then close the stream (the real server's teardown
    // ends it). Biased cancel is observed before the EOF.
    cancel_tx.send(()).await.expect("cancel");
    drop(tx);

    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionFinished {
            outcome: Outcome::Cancelled
        }
    ));
}

/// An interrupted Turn reports `Completed`; it is the driver, which knows it
/// asked, that turns that into a cancelled outcome.
#[tokio::test]
async fn a_turn_interrupted_after_cancel_still_finishes_as_cancelled() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    cancel_tx.send(()).await.expect("cancel");
    tx.try_send(Ok(turn_completed("interrupted", None)))
        .expect("feed");

    loop {
        match next_signal(&mut signal_rx).await {
            Signal::Source(_) => continue,
            Signal::ExecutionFinished { outcome } => {
                assert_eq!(outcome, Outcome::Cancelled);
                break;
            }
            other => panic!("unexpected signal {other:?}"),
        }
    }
}

/// Cancel asks the Turn to stop before killing the server, so Codex writes its
/// rollout and the Session stays resumable.
#[tokio::test]
async fn cancel_interrupts_the_running_turn_first() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    p.thread_id = Some("thread-A".to_string());
    p.turn_id = Some("turn-1".to_string());

    let cancelling = tokio::spawn(async move {
        p.cancel().await;
        p
    });
    let request = server.answer_next(json!({})).await;
    assert_eq!(request["method"], "turn/interrupt");
    assert_eq!(request["params"]["threadId"], "thread-A");
    assert_eq!(request["params"]["turnId"], "turn-1");

    let p = cancelling.await.expect("cancel finishes");
    assert!(p.session_cancelled());
}

/// Before `turn/start` answers there is no Turn to interrupt, so tearing the
/// process down is the whole of cancellation.
#[tokio::test]
async fn cancel_tears_down_the_session() {
    let (_tx, _server, session) = AppServerSession::scripted();
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    assert!(!p.session_cancelled());
    p.cancel().await;
    assert!(
        p.session_cancelled(),
        "cancel must tear down the codex process tree"
    );
}

#[tokio::test]
async fn eof_mid_turn_is_an_unexpected_exit() {
    let (tx, server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    // Stream closes with no terminal event → unexpected exit. The server end
    // goes too: the reader task holds its own handle on the event stream, and
    // it only lets go when the pipe reaches EOF, exactly as a dead subprocess
    // would.
    drop(tx);
    drop(server);
    match next_signal(&mut signal_rx).await {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(error),
        } => assert_eq!(error.class, ErrorClass::ProcessExit),
        other => panic!("expected failed terminal, got {other:?}"),
    }
}

/// A transport failure mid-Turn is a terminal, classified from the stream text.
#[tokio::test]
async fn a_stream_error_ends_the_turn_as_a_failure() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.try_send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    tx.try_send(Err("Codex app-server exited with status 1".to_string()))
        .expect("feed");
    match next_signal(&mut signal_rx).await {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(error),
        } => assert_eq!(error.class, ErrorClass::ProcessExit),
        other => panic!("expected failed terminal, got {other:?}"),
    }
}
