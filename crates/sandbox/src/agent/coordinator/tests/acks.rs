//! When an input is acknowledged, and when the ack is dropped.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn ack_is_withheld_until_input_accepted() {
    let (events, _received, _cancel) = recording_publisher();
    // No `AcceptInput` step, so the harness never reports the input accepted.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::Block(block("msg-1", text_block("hello"), Role::Assistant)),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let (ack_tx, mut ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;

    // The Run ran to completion, but with no InputAccepted the batch stays
    // unacknowledged — the ack channel is still open and empty.
    assert!(matches!(
        ack_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn injected_input_ack_waits_for_following_source() {
    let (events, _received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(Harness::scripted(Script::new(vec![])), events, "task-1");

    let (first_ack, first_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "initial"), first_ack)
        .await
        .expect("submit first");
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-1".to_string()),
            cause: Cause::Input(InputId("in-1".to_string())),
        })
        .await;
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-1".to_string()),
        })
        .await;
    let first = first_rx.await.expect("opening input acked");

    let (second_ack, mut second_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-2", "merge conflict notice"), second_ack)
        .await
        .expect("submit second");
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-2".to_string()),
        })
        .await;

    assert!(
        matches!(
            second_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ),
        "an injected input is not acked on transport acceptance alone"
    );

    coord
        .handle_signal(Signal::Source(block(
            "msg-1",
            text_block("still working"),
            Role::Assistant,
        )))
        .await;
    let second = second_rx.await.expect("injected input acked after source");

    assert_eq!(second.session_id, "sess-1");
    assert_eq!(second.run_id, first.run_id);
}

#[tokio::test]
async fn injected_input_ack_resolves_on_following_activity() {
    let (events, _received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(Harness::scripted(Script::new(vec![])), events, "task-1");

    let (first_ack, first_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "initial"), first_ack)
        .await
        .expect("submit first");
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-1".to_string()),
            cause: Cause::Input(InputId("in-1".to_string())),
        })
        .await;
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-1".to_string()),
        })
        .await;
    let first = first_rx.await.expect("opening input acked");

    let (second_ack, mut second_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-2", "merge conflict notice"), second_ack)
        .await
        .expect("submit second");
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-2".to_string()),
        })
        .await;
    assert!(
        matches!(
            second_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ),
        "activity must be after acceptance to resolve the ack"
    );

    coord
        .handle_signal(Signal::Activity(Activity {
            session: SessionId("sess-1".to_string()),
            detail: "Bash".to_string(),
        }))
        .await;
    let second = second_rx
        .await
        .expect("injected input acked after activity");

    assert_eq!(second.session_id, "sess-1");
    assert_eq!(second.run_id, first.run_id);
}

#[tokio::test]
async fn multiple_injected_input_acks_resolve_on_one_following_source() {
    let (events, _received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(Harness::scripted(Script::new(vec![])), events, "task-1");

    let (first_ack, first_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "initial"), first_ack)
        .await
        .expect("submit first");
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-1".to_string()),
            cause: Cause::Input(InputId("in-1".to_string())),
        })
        .await;
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-1".to_string()),
        })
        .await;
    let first = first_rx.await.expect("opening input acked");

    let (second_ack, mut second_rx) = oneshot::channel();
    let (third_ack, mut third_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-2", "first late notice"), second_ack)
        .await
        .expect("submit second");
    coord
        .submit(Input::text("in-3", "second late notice"), third_ack)
        .await
        .expect("submit third");
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-2".to_string()),
        })
        .await;
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-3".to_string()),
        })
        .await;
    assert!(matches!(
        second_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        third_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    coord
        .handle_signal(Signal::Source(block(
            "msg-1",
            text_block("still working"),
            Role::Assistant,
        )))
        .await;
    let second = second_rx.await.expect("second input acked");
    let third = third_rx.await.expect("third input acked");

    assert_eq!(second.run_id, first.run_id);
    assert_eq!(third.run_id, first.run_id);
}

#[tokio::test]
async fn injected_input_ack_is_dropped_when_terminal_arrives_first() {
    let (events, _received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(Harness::scripted(Script::new(vec![])), events, "task-1");

    let (first_ack, first_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "initial"), first_ack)
        .await
        .expect("submit first");
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-1".to_string()),
            cause: Cause::Input(InputId("in-1".to_string())),
        })
        .await;
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-1".to_string()),
        })
        .await;
    let _ = first_rx.await.expect("opening input acked");

    let (second_ack, mut second_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-2", "merge conflict notice"), second_ack)
        .await
        .expect("submit second");
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-2".to_string()),
        })
        .await;
    assert!(
        matches!(
            second_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ),
        "accepted injected input is waiting for post-acceptance output"
    );

    coord
        .handle_signal(Signal::ExecutionFinished {
            outcome: Outcome::Completed { totals: None },
        })
        .await;

    assert!(
        matches!(second_rx.await, Err(oneshot::error::RecvError { .. })),
        "terminal-before-output drops the ack so the backend delivery is not acknowledged"
    );
}

#[tokio::test]
async fn injected_input_ack_is_dropped_when_cancelled_terminal_arrives_first() {
    let (events, _received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(Harness::scripted(Script::new(vec![])), events, "task-1");

    let (first_ack, first_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "initial"), first_ack)
        .await
        .expect("submit first");
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-1".to_string()),
            cause: Cause::Input(InputId("in-1".to_string())),
        })
        .await;
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-1".to_string()),
        })
        .await;
    let _ = first_rx.await.expect("opening input acked");

    let (second_ack, second_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-2", "merge conflict notice"), second_ack)
        .await
        .expect("submit second");
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-2".to_string()),
        })
        .await;
    coord
        .handle_signal(Signal::ExecutionFinished {
            outcome: Outcome::Cancelled,
        })
        .await;

    assert!(
        matches!(second_rx.await, Err(oneshot::error::RecvError { .. })),
        "cancelled terminal-before-output drops the ack"
    );
}

#[tokio::test]
async fn injected_input_ack_is_dropped_at_next_execution_start() {
    let (events, _received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(Harness::scripted(Script::new(vec![])), events, "task-1");

    let (first_ack, first_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "initial"), first_ack)
        .await
        .expect("submit first");
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-1".to_string()),
            cause: Cause::Input(InputId("in-1".to_string())),
        })
        .await;
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-1".to_string()),
        })
        .await;
    let _ = first_rx.await.expect("opening input acked");

    let (second_ack, second_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-2", "merge conflict notice"), second_ack)
        .await
        .expect("submit second");
    coord
        .handle_signal(Signal::InputAccepted {
            input: InputId("in-2".to_string()),
        })
        .await;
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-2".to_string()),
            cause: Cause::Internal,
        })
        .await;

    assert!(
        matches!(second_rx.await, Err(oneshot::error::RecvError { .. })),
        "a new run boundary drops unproven injected acks before run_id changes"
    );
}

#[tokio::test]
async fn rejected_input_opens_no_run_and_drops_the_ack() {
    let (events, received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(
        Harness::scripted(Script::rejecting(failure())),
        events.clone(),
        "task-1",
    );
    let (ack_tx, ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    // No ExecutionStarted preceded the rejection, so no Run was opened: no turn
    // start is published. A classified failure is surfaced.
    let types = sub_types(&received);
    assert!(!types.contains(&"started".to_string()), "got {types:?}");
    assert!(!types.contains(&"active".to_string()), "got {types:?}");
    assert!(types.contains(&"error".to_string()), "got {types:?}");

    // The ack is dropped: the dispatcher observes a closed channel and leaves
    // the batch unacknowledged.
    assert!(matches!(
        ack_rx.await,
        Err(oneshot::error::RecvError { .. })
    ));
}
