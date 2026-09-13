//! The `agent.*` events one turn publishes, and their order.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn fresh_turn_publishes_ordered_events_and_acks_after_acceptance() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", text_block("hello"), Role::Assistant)),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");

    let (ack_tx, ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    // ExecutionStarted → turn start (started + taskTurn active) + session init;
    // then the block; then the terminal completion — in native order.
    assert_eq!(
        sub_types(&received),
        vec!["started", "active", "session_init", "message", "completed"],
    );

    // The batch is acknowledged only after InputAccepted, against the minted Run.
    let ack = ack_rx.await.expect("ack resolved");
    assert_eq!(ack.session_id, "sess-1");
    let started = only(&received, "started");
    assert_eq!(ack.run_id, started.run_id);
    assert!(!ack.run_id.is_empty());

    // Every event of the Run carries its Run id.
    for req in received.lock().expect("lock").iter() {
        assert_eq!(req.run_id, ack.run_id, "run-scoped: {}", req.sub_type);
    }
}

/// A signal moves the state machine and the event stream as one act.
///
/// Checked after *every* signal, not just at the end: the agent reports
/// `Running` for exactly the span the events say the turn is open.
#[tokio::test]
async fn a_turn_moves_the_state_machine_and_the_event_stream_together() {
    use crate::agent::agent_state::AgentState;
    use crate::agent::types::AgentStatus;

    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", text_block("hello"), Role::Assistant)),
        Step::Completed,
    ]);
    let agent_state = Arc::new(AgentState::new());
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    coord.bind(Arc::clone(&agent_state), CancellationToken::new());

    assert_eq!(
        agent_state.status().await,
        AgentStatus::Idle,
        "before the turn"
    );

    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");

    while let Some(signal) = coord.next_signal().await {
        let terminal = matches!(signal, Signal::ExecutionFinished { .. });
        coord.handle_signal(signal).await;
        flush(&events).await;

        let published = sub_types(&received);
        let turn_is_open = published.contains(&"active".to_string())
            && !published.contains(&"completed".to_string());
        assert_eq!(
            agent_state.status().await == AgentStatus::Running,
            turn_is_open,
            "state machine disagrees with the events after {published:?}"
        );

        if terminal {
            break;
        }
    }

    assert_eq!(
        sub_types(&received),
        vec!["started", "active", "session_init", "message", "completed"],
    );
    assert_eq!(agent_state.status().await, AgentStatus::Idle);
    assert_eq!(
        agent_state.current_run_id().await.as_deref(),
        Some(coord.current_run_id()),
        "the state machine names the Run the events were stamped with"
    );
}

#[tokio::test]
async fn per_block_message_payload_matches_backend_assembly_contract() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", text_block("one"), Role::Assistant)),
        Step::Block(block(
            "msg-1",
            serde_json::json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": {} }),
            Role::Assistant,
        )),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    let messages: Vec<serde_json::Value> = received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| r.sub_type == "message")
        .map(|r| serde_json::from_str(&r.payload_json).expect("json"))
        .collect();
    assert_eq!(messages.len(), 2);

    // Both blocks share the native Message id as their grouping `uuid`, carry
    // increasing `blockIndex`, and each holds exactly its own single block.
    assert_eq!(messages[0]["uuid"], "msg-1");
    assert_eq!(messages[0]["blockIndex"], 0);
    assert_eq!(messages[0]["session_id"], "sess-1");
    assert_eq!(messages[0]["message"]["role"], "assistant");
    assert_eq!(messages[0]["message"]["content"][0]["text"], "one");
    assert_eq!(
        messages[0]["message"]["content"].as_array().unwrap().len(),
        1
    );

    assert_eq!(messages[1]["uuid"], "msg-1");
    assert_eq!(messages[1]["blockIndex"], 1);
    assert_eq!(messages[1]["message"]["content"][0]["type"], "tool_use");
    assert_eq!(messages[1]["message"]["content"][0]["name"], "Bash");
}

#[tokio::test]
async fn tool_result_block_surfaces_typed_result_at_payload_top_level() {
    let (events, received, _cancel) = recording_publisher();
    let mut result = block(
        "msg-r",
        serde_json::json!({
            "type": "tool_result",
            "tool_use_id": "t1",
            "content": "ok",
            "is_error": false,
        }),
        Role::User,
    );
    result.tool_use_result = Some(serde_json::json!({ "status": "completed", "exitCode": 0 }));
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(result),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    let message = received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| r.sub_type == "message")
        .map(|r| serde_json::from_str::<serde_json::Value>(&r.payload_json).expect("json"))
        .next()
        .expect("a message event");
    // The typed result rides at the payload top level (`toolUseResult`), where the
    // backend reads it to enrich the rendered tool output, separate from the
    // block's own summary `content`.
    assert_eq!(message["toolUseResult"]["status"], "completed");
    assert_eq!(message["toolUseResult"]["exitCode"], 0);
    assert_eq!(message["message"]["role"], "user");
    assert_eq!(message["message"]["content"][0]["type"], "tool_result");
}

#[tokio::test]
async fn failed_execution_publishes_error_and_no_completion() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Failed(failure()),
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    let types = sub_types(&received);
    assert!(types.contains(&"error".to_string()), "got {types:?}");
    assert!(!types.contains(&"completed".to_string()), "got {types:?}");

    let error: serde_json::Value =
        serde_json::from_str(&only(&received, "error").payload_json).expect("json");
    assert_eq!(error["error"], "boom");
    assert_eq!(error["reason"], "native_protocol");
}
