use super::*;

fn v1(command: Value) -> Value {
    match translate(&command) {
        Ok(Translated::V1(v1)) => v1,
        other => panic!("expected a V1 command, got {other:?}"),
    }
}

fn refusal(command: Value) -> CommandRefusal {
    translate(&command).expect_err("expected a refusal")
}

#[test]
fn renamed_commands_keep_their_fields_under_the_v1_type() {
    let command = v1(json!({ "type": "run.interrupt", "threadId": "task-1", "commandId": "c1" }));
    assert_eq!(command["type"], json!("thread.turn.interrupt"));
    assert_eq!(command["threadId"], json!("task-1"));
    assert_eq!(
        v1(json!({ "type": "thread.model-selection.set", "threadId": "task-1" }))["type"],
        json!("thread.meta.update")
    );
    assert_eq!(
        v1(json!({ "type": "thread.archive", "threadId": "task-1" }))["type"],
        json!("thread.archive")
    );
}

#[test]
fn message_dispatch_becomes_a_turn_start_and_queues_after_active() {
    let started = v1(json!({
        "type": "message.dispatch",
        "threadId": "task-1",
        "messageId": "m1",
        "text": "hello",
        "dispatchMode": { "type": "start_immediately" }
    }));
    assert_eq!(started["type"], json!("thread.turn.start"));
    assert_eq!(started["message"]["messageId"], json!("m1"));
    assert_eq!(started["message"]["text"], json!("hello"));
    assert!(started.get("dispatchMode").is_none());

    let queued = v1(json!({
        "type": "message.dispatch",
        "threadId": "task-1",
        "messageId": "m2",
        "dispatchMode": { "type": "queue_after_active" }
    }));
    assert_eq!(queued["dispatchMode"], json!("queue"));
}

#[test]
fn message_dispatch_refuses_restart_and_deferred_start() {
    for mode in ["restart_active", "defer_start"] {
        let refused = refusal(json!({
            "type": "message.dispatch",
            "messageId": "m1",
            "dispatchMode": { "type": mode }
        }));
        assert!(
            matches!(refused, CommandRefusal::Unsupported { .. }),
            "{mode}"
        );
    }
    let restart = refusal(json!({
        "type": "message.dispatch",
        "messageId": "m1",
        "deliveryIntent": "restart"
    }));
    assert!(matches!(restart, CommandRefusal::Unsupported { .. }));
}

#[test]
fn promote_to_steer_names_the_queued_message() {
    let command = v1(json!({
        "type": "queued-message.promote-to-steer",
        "threadId": "task-1",
        "queuedRunId": "task-1:queued:m7"
    }));
    assert_eq!(command["type"], json!("thread.queuedTurn.steer"));
    assert_eq!(command["messageId"], json!("m7"));
    assert!(matches!(
        refusal(json!({ "type": "queued-message.promote-to-steer", "queuedRunId": "task-1:2" })),
        CommandRefusal::Rejected { .. }
    ));
}

#[test]
fn fork_maps_the_target_thread_and_the_run_it_was_cut_at() {
    let command = v1(json!({
        "type": "thread.fork",
        "sourceThreadId": "task-1",
        "targetThreadId": "task-2",
        "sourcePoint": { "type": "run", "runId": "task-1:3" },
        "sameSandbox": true
    }));
    assert_eq!(command["threadId"], json!("task-2"));
    assert_eq!(command["sourceThreadId"], json!("task-1"));
    assert_eq!(command["atTurn"], json!(3));
    assert_eq!(command["sameSandbox"], json!(true));

    let latest = v1(json!({
        "type": "thread.fork",
        "targetThreadId": "task-2",
        "sourcePoint": { "type": "latest_stable" }
    }));
    assert_eq!(latest["atTurn"], Value::Null);
    assert!(matches!(
        refusal(json!({
            "type": "thread.fork",
            "targetThreadId": "task-2",
            "sourcePoint": { "type": "checkpoint", "checkpointId": "x" }
        })),
        CommandRefusal::Unsupported { .. }
    ));
}

#[test]
fn approvals_are_refused_and_answers_become_user_input_responses() {
    assert!(matches!(
        refusal(json!({ "type": "runtime-request.respond", "decision": "accept" })),
        CommandRefusal::Unsupported { .. }
    ));
    let answer = v1(json!({
        "type": "runtime-request.respond",
        "threadId": "task-1",
        "requestId": "req-1",
        "answers": { "q1": "yes" }
    }));
    assert_eq!(answer["type"], json!("thread.user-input.respond"));
    assert_eq!(answer["requestId"], json!("req-1"));
}

#[test]
fn client_local_state_is_discarded_and_unknown_commands_refused() {
    assert_eq!(
        translate(&json!({ "type": "thread.visit", "threadId": "task-1" })).unwrap(),
        Translated::Discarded
    );
    assert!(matches!(
        refusal(json!({ "type": "thread.merge_back" })),
        CommandRefusal::Unsupported { .. }
    ));
    assert!(matches!(
        refusal(json!({ "type": "checkpoint.restore" })),
        CommandRefusal::Unsupported { .. }
    ));
}
