//! Steering a message into a running Turn, and the gate it waits on.

use super::fixtures::*;
use super::*;

/// A process whose opening input has been submitted against an injected session
/// and whose thread is announced, with `turn/started` absorbed only when
/// `steerable` — the gate is what most of these tests are about.
async fn opened_process(session: AppServerSession, steerable: bool) -> CodexProcess {
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    p.submit(Input::text("in-1", "one"))
        .await
        .expect("submit the opening input");
    p.absorb(thread_started("thread-A"));
    if steerable {
        p.absorb(turn_started("turn-1"));
    }
    drain(&mut p);
    p
}

/// The steer request the harness sends, and the acceptance it reports for it.
/// `expectedTurnId` is a precondition the server checks: an empty or stale one
/// is rejected.
#[tokio::test]
async fn a_message_sent_while_the_turn_runs_is_steered_into_it() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, true).await;

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "reply with BANANA")).await;
        (p, outcome)
    });
    let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
    let (mut p, outcome) = steering.await.expect("the steering task joins");

    outcome.expect("the running Turn takes the message");
    assert_eq!(request["method"], "turn/steer");
    assert_eq!(request["params"]["threadId"], "thread-A");
    assert_eq!(request["params"]["expectedTurnId"], "turn-1");
    assert_eq!(request["params"]["input"][0]["text"], "reply with BANANA");
    assert_eq!(
        drain(&mut p),
        vec![ProcessEvent::InputAccepted {
            input: InputId("in-2".to_string()),
        }],
        "a steered message reports one bare acceptance and nothing else"
    );
}

/// The gate: a message that arrives before `turn/started` is not sent
/// optimistically — steering that early is rejected with `-32600 "no active
/// turn to steer"` — so it waits, and goes the moment the notification lands.
#[tokio::test]
async fn a_message_that_arrives_before_the_gate_opens_is_held_until_it_does() {
    let (tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, false).await;

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "held")).await;
        (p, outcome)
    });

    // Nothing may reach the server while the Turn is not steerable.
    let early = timeout(Duration::from_millis(100), server.next_request()).await;
    assert!(
        early.is_err(),
        "a message must not be steered before turn/started, got {early:?}"
    );

    tx.send(Ok(turn_started("turn-1"))).expect("feed");
    let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
    let (mut p, outcome) = steering.await.expect("the steering task joins");

    outcome.expect("the held message is delivered once the gate opens");
    assert_eq!(request["method"], "turn/steer");
    assert_eq!(request["params"]["expectedTurnId"], "turn-1");
    assert_eq!(
        drain(&mut p),
        vec![ProcessEvent::InputAccepted {
            input: InputId("in-2".to_string()),
        }]
    );
}

/// A Stop must not wait out the gate: `cancel` is unreachable while this
/// `submit` borrows the process, so the handle is what ends the wait.
///
/// The message is handed back for the next Turn rather than dying with this
/// one; in production the driver fires that handle, racing it against this call
/// (`a_stop_reaches_the_driver_while_a_message_waits_on_the_gate`).
#[tokio::test]
async fn a_cancel_while_a_message_is_held_hands_it_back_at_once() {
    let (_tx, _server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, false).await;
    let handle = p.cancel_handle();

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "held")).await;
        (p, outcome)
    });
    // Let the submit reach the gate before cancelling, so this is the wait being
    // interrupted and not a cancel that was already set.
    tokio::task::yield_now().await;
    handle.cancel();

    let (p, outcome) = timeout(Duration::from_secs(1), steering)
        .await
        .expect("a cancelled wait ends promptly")
        .expect("the steering task joins");
    assert!(
        outcome.is_err(),
        "a cancelled Turn cannot take the message, so the driver must get it back"
    );
    assert!(p.held.is_none(), "nothing may stay held after a failure");
}

/// The bound on the wait: `turn/started` follows `turn/start` immediately in a
/// healthy Turn, so this only fires once the server has gone quiet.
///
/// Safe only because it can fire *before* anything was sent — this asserts the
/// wire stayed empty, since a message the server took cannot also be handed
/// back without being answered twice.
#[tokio::test(start_paused = true)]
async fn a_gate_that_never_opens_hands_the_message_back_having_sent_nothing() {
    // `_tx` stays alive: dropping it would end the stream, which is a different
    // reason to give up than the one under test.
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, false).await;

    let outcome = p.submit(Input::text("in-2", "held")).await;

    assert!(
        outcome.is_err(),
        "a gate that never opens hands the message back"
    );
    assert!(p.held.is_none(), "nothing may stay held after a failure");
    let sent = timeout(Duration::from_millis(100), server.next_request()).await;
    assert!(
        sent.is_err(),
        "the timeout may only fire before anything was written, got {sent:?}"
    );
}

/// The timeout bounds the gate, not the request: once `turn/steer` is on the
/// wire the server may accept it at any moment.
///
/// Abandoning it would hand the driver a message the Turn had already taken,
/// acting on it twice; a steer instead waits as long as the Turn does, released
/// only when the stream ends.
#[tokio::test(start_paused = true)]
async fn a_steer_already_on_the_wire_is_never_abandoned_for_the_timeout() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, true).await;

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "reply with BANANA")).await;
        (p, outcome)
    });
    let request = server.next_request().await;
    assert_eq!(request["method"], "turn/steer");

    // Long past the bound, and the server has still not answered.
    tokio::time::advance(MID_TURN_DELIVERY_TIMEOUT * 4).await;
    server
        .write(json!({ "id": request["id"], "result": { "turnId": "turn-1" } }))
        .await;

    let (mut p, outcome) = steering.await.expect("the steering task joins");
    outcome.expect("a steer the server took is taken, however long it took to answer");
    assert_eq!(
        drain(&mut p),
        vec![ProcessEvent::InputAccepted {
            input: InputId("in-2".to_string()),
        }]
    );
}

/// Usage is the growth of the Session's cumulative total while the Turn ran, so
/// a Turn that a message was steered into bills the model requests the steer
/// caused as well as its own. The baseline is the reading replayed before
/// `turn/started`.
#[tokio::test]
async fn a_steered_turn_bills_every_model_request_it_made() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    p.submit(Input::text("in-1", "one")).await.expect("submit");
    p.absorb(thread_started("thread-A"));
    // A resumed thread replays what the Session had already spent.
    p.absorb(token_usage(1000, 0, 100, 0));
    p.absorb(turn_started("turn-1"));
    // The Turn's own first model request.
    p.absorb(token_usage(1400, 40, 140, 20));
    drain(&mut p);

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "and BANANA")).await;
        (p, outcome)
    });
    server.answer_next(json!({ "turnId": "turn-1" })).await;
    let (mut p, outcome) = steering.await.expect("the steering task joins");
    outcome.expect("steered");

    // The request the steered message caused, then the terminal.
    p.absorb(token_usage(1900, 80, 260, 60));
    p.absorb(turn_completed("completed", None));
    let events = drain(&mut p);

    let usage = sources(&events)
        .last()
        .expect("the turn-result block")
        .usage
        .clone()
        .expect("usage on the turn-result block");
    assert_eq!(usage["input_tokens"], 900, "1900 - 1000");
    assert_eq!(usage["cache_read_input_tokens"], 80, "80 - 0");
    assert_eq!(usage["output_tokens"], 160, "260 - 100");
    assert_eq!(usage["reasoning_output_tokens"], 60, "60 - 0");
}
