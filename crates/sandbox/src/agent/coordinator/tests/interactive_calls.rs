//! A call is published once, whichever route sees it first.

use super::fixtures::*;
use super::*;

/// An interactive call is published by the permission handler the moment it
/// parks, so a person can see the question.
///
/// The block holding it arrives here too under the same tool-use id; since the
/// conversation stores one row per id, the coordinator must let the
/// announcement stand rather than write it again.
#[tokio::test]
async fn a_call_the_permission_handler_announced_is_not_published_again() {
    let (events, received, _cancel) = recording_publisher();
    let plan = |id: &str| serde_json::json!({ "type": "tool_use", "id": id, "name": "ExitPlanMode", "input": {} });
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", plan("toolu_plan"), Role::Assistant)),
        Step::Block(block("msg-1", text_block("after"), Role::Assistant)),
        Step::Completed,
    ]);
    let agent_state = Arc::new(crate::agent::agent_state::AgentState::new());
    // The claim the handler took on its way into the wait.
    assert!(agent_state.claim_prompt_publication("toolu_plan").await);

    let mut coord = Coordinator::new(HarnessRuntime::scripted(script), events.clone(), "task-1")
        .driving(Arc::clone(&agent_state));
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

    // Only the text block is published, and it keeps the position the model gave
    // it — the skipped call spent index 0.
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["blockIndex"], 1);
    assert_eq!(messages[0]["message"]["content"][0]["text"], "after");
}

/// The claim runs the other way too: a call reaching the coordinator before the
/// handler announces it is published here, taking the claim that keeps the
/// announcement from writing it again.
///
/// Only calls that park a run are claimed; every other tool call publishes
/// unconditionally, so the claim set stays as small as the questions asked.
#[tokio::test]
async fn a_call_that_arrives_first_publishes_and_takes_the_claim() {
    let (events, received, _cancel) = recording_publisher();
    let call = |name: &str, id: &str| serde_json::json!({ "type": "tool_use", "id": id, "name": name, "input": {} });
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block(
            "msg-1",
            call("ExitPlanMode", "toolu_plan"),
            Role::Assistant,
        )),
        Step::Block(block("msg-1", call("Bash", "toolu_bash"), Role::Assistant)),
        Step::Completed,
    ]);
    let agent_state = Arc::new(crate::agent::agent_state::AgentState::new());
    let mut coord = Coordinator::new(HarnessRuntime::scripted(script), events.clone(), "task-1")
        .driving(Arc::clone(&agent_state));
    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    let ids: Vec<String> = received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| r.sub_type == "message")
        .map(|r| {
            let payload: serde_json::Value = serde_json::from_str(&r.payload_json).expect("json");
            payload["message"]["content"][0]["id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(ids, vec!["toolu_plan", "toolu_bash"]);

    // The interactive call is spoken for; the ordinary one was never claimed.
    assert!(!agent_state.claim_prompt_publication("toolu_plan").await);
    assert!(agent_state.claim_prompt_publication("toolu_bash").await);
}
