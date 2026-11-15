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
    let stop = v1(json!({
        "type": "thread.stop",
        "threadId": "task-1",
        "commandId": "c2",
        "stopDelegatedTasks": true
    }));
    assert_eq!(stop["type"], json!("thread.turn.interrupt"));
    assert_eq!(stop["threadId"], json!("task-1"));
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

/// `commands` folds `context` into the text; dropping it here loses every chip.
#[test]
fn message_dispatch_carries_its_context_onto_the_message() {
    let context = json!({ "version": 1, "records": [] });
    let started = v1(json!({
        "type": "message.dispatch",
        "threadId": "task-1",
        "messageId": "m1",
        "text": "hello",
        "context": context
    }));
    assert_eq!(started["message"]["context"], context);
}

#[test]
fn message_dispatch_refuses_a_deferred_start() {
    let refused = refusal(json!({
        "type": "message.dispatch",
        "messageId": "m1",
        "dispatchMode": { "type": "defer_start" }
    }));
    assert!(matches!(refused, CommandRefusal::Unsupported { .. }));
}

#[test]
fn a_restart_interrupts_the_turn_then_queues_the_message() {
    for command in [
        json!({
            "type": "message.dispatch",
            "threadId": "task-1",
            "messageId": "m1",
            "dispatchMode": { "type": "restart_active", "targetRunId": "task-1:3" }
        }),
        json!({
            "type": "message.dispatch",
            "threadId": "task-1",
            "messageId": "m1",
            "deliveryIntent": "restart",
            "dispatchMode": { "type": "start_immediately" }
        }),
    ] {
        let Ok(Translated::Sequence(steps)) = translate(&command) else {
            panic!("expected a sequence for {command}");
        };
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0]["type"], json!("thread.turn.interrupt"));
        assert_eq!(steps[0]["threadId"], json!("task-1"));
        assert_eq!(steps[1]["type"], json!("thread.turn.start"));
        assert_eq!(steps[1]["dispatchMode"], json!("queue"));
        assert_eq!(steps[1]["message"]["messageId"], json!("m1"));
    }
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
        "sourcePoint": { "type": "latest_stable" },
        "sideChat": true,
        "title": "Side chat",
        "interactionMode": "plan"
    }));
    assert_eq!(latest["atTurn"], Value::Null);
    assert_eq!(latest["sideChat"], json!(true));
    assert_eq!(latest["title"], json!("Side chat"));
    assert_eq!(latest["interactionMode"], json!("plan"));
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
        translate(&json!({ "type": "thread.runtime-mode.set", "threadId": "task-1" })).unwrap(),
        Translated::Discarded
    );
    assert!(matches!(
        refusal(json!({ "type": "checkpoint.restore" })),
        CommandRefusal::Unsupported { .. }
    ));
}

#[test]
fn checkpoint_rollback_reverts_the_conversation_unless_files_are_asked_for() {
    let rollback = |restore: bool| {
        v1(json!({
            "type": "checkpoint.rollback",
            "threadId": "task-1",
            "scopeId": "task-1:checkpoints",
            "checkpointId": "task-1:checkpoint:2",
            "restoreFiles": restore
        }))
    };
    let conversation = rollback(false);
    assert_eq!(conversation["type"], json!("thread.conversation.revert"));
    assert_eq!(conversation["turnCount"], json!(2));
    assert_eq!(rollback(true)["type"], json!("thread.checkpoint.revert"));
    assert!(matches!(
        refusal(
            json!({ "type": "checkpoint.rollback", "threadId": "task-1", "checkpointId": "nope" })
        ),
        CommandRefusal::Rejected { .. }
    ));
}

#[test]
fn a_project_update_is_a_meta_update_and_auto_pull_is_refused() {
    let update = json!({
        "type": "project.update",
        "commandId": "c1",
        "projectId": "ws-1",
        "scripts": [],
    });
    let v1 = project_mutation(&update).expect("an update is carried out");
    assert_eq!(v1["type"], json!("project.meta.update"));
    assert_eq!(v1["projectId"], json!("ws-1"));
    assert_eq!(v1["scripts"], json!([]));

    let mut auto_pull = update.clone();
    auto_pull["autoPull"] = json!(true);
    assert!(matches!(
        project_mutation(&auto_pull),
        Err(CommandRefusal::Unsupported { .. })
    ));

    let delete = json!({ "type": "project.delete", "commandId": "c1", "projectId": "ws-1" });
    assert_eq!(project_mutation(&delete).expect("passed on"), delete);
}

#[test]
fn queued_run_commands_name_the_message_their_run_waits_to_deliver() {
    let cancel = v1(json!({
        "type": "queued-run.cancel",
        "threadId": "task-1",
        "runId": "task-1:queued:msg-2"
    }));
    assert_eq!(cancel["type"], json!("thread.queuedTurn.cancel"));
    assert_eq!(cancel["messageId"], json!("msg-2"));

    let edit = v1(json!({
        "type": "queued-run.edit",
        "threadId": "task-1",
        "runId": "task-1:queued:msg-2",
        "text": "again",
        "context": { "version": 1, "records": [] },
        "attachments": []
    }));
    assert_eq!(edit["type"], json!("thread.queuedTurn.edit"));
    assert_eq!(edit["context"], json!({ "version": 1, "records": [] }));
    assert_eq!(edit["text"], json!("again"));
    assert_eq!(edit["attachments"], json!([]));

    let reorder = v1(json!({
        "type": "queued-run.reorder",
        "threadId": "task-1",
        "runId": "task-1:queued:msg-2",
        "beforeRunId": "task-1:queued:msg-1"
    }));
    assert_eq!(reorder["type"], json!("thread.queuedTurn.reorder"));
    assert_eq!(reorder["beforeMessageId"], json!("msg-1"));
    let to_end = v1(json!({
        "type": "queued-run.reorder",
        "threadId": "task-1",
        "runId": "task-1:queued:msg-2",
        "beforeRunId": null
    }));
    assert_eq!(to_end["beforeMessageId"], Value::Null);

    let start = v1(json!({
        "type": "queued-run.start",
        "threadId": "task-1",
        "commandId": "c1",
        "runId": "task-1:queued:msg-2"
    }));
    assert_eq!(start["type"], json!("thread.queuedTurn.start"));
    assert_eq!(start["messageId"], json!("msg-2"));
    assert_eq!(start["threadId"], json!("task-1"));

    assert!(matches!(
        refusal(json!({ "type": "queued-run.cancel", "runId": "task-1:3" })),
        CommandRefusal::Rejected { .. }
    ));
}

#[test]
fn resume_dismiss_and_read_marks_translate_to_their_v1_commands() {
    assert_eq!(
        v1(json!({ "type": "queue.resume", "threadId": "task-1" }))["type"],
        json!("thread.queue.resume")
    );
    let dismiss = v1(json!({
        "type": "thread.user-input.dismiss",
        "threadId": "task-1",
        "requestId": "req-1"
    }));
    assert_eq!(dismiss["type"], json!("thread.user-input.respond"));
    assert_eq!(dismiss["requestId"], json!("req-1"));
    assert_eq!(dismiss["dismissed"], json!(true));
    for tag in ["thread.visit", "thread.mark-unread", "thread.delete"] {
        assert_eq!(
            v1(json!({ "type": tag, "threadId": "task-1" }))["type"],
            json!(tag)
        );
    }
}

#[test]
fn pin_and_reorder_commands_keep_their_names_and_order_keys() {
    for tag in [
        "thread.pin",
        "thread.unpin",
        "thread.pin.reorder",
        "thread.active.reorder",
    ] {
        let command = v1(json!({ "type": tag, "threadId": "task-1", "orderKey": "a0" }));
        assert_eq!(command["type"], json!(tag));
        assert_eq!(command["orderKey"], json!("a0"));
    }
}

#[test]
fn auto_settle_set_keeps_its_name_and_enabled() {
    let command = v1(json!({
        "type": "thread.auto-settle.set",
        "threadId": "task-1",
        "enabled": false,
    }));
    assert_eq!(command["type"], json!("thread.auto-settle.set"));
    assert_eq!(command["enabled"], json!(false));
}

/// Starting a watch on a pull request the thread has not linked links it
/// first. Stopping one never links, whatever `link` carries.
#[test]
fn a_watch_that_asks_to_link_links_first() {
    let watch = |watching: bool| {
        json!({
            "type": "thread.pull-request.watch",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "host": "github.com",
            "repository": "acme/web",
            "number": 42,
            "watching": watching,
            "link": { "url": "https://github.com/acme/web/pull/42", "source": "manual" }
        })
    };
    let Ok(Translated::Sequence(steps)) = translate(&watch(true)) else {
        panic!("expected a link and then the watch");
    };
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["type"], json!("thread.pull-request.link"));
    assert_eq!(steps[0]["repository"], json!("acme/web"));
    assert_eq!(steps[0]["number"], json!(42));
    assert_eq!(
        steps[0]["url"],
        json!("https://github.com/acme/web/pull/42")
    );
    assert_eq!(steps[0]["source"], json!("manual"));
    assert_eq!(steps[1]["type"], json!("thread.pull-request.watch"));

    let stop = v1(watch(false));
    assert_eq!(stop["type"], json!("thread.pull-request.watch"));
    assert_eq!(stop["watching"], json!(false));
}

#[test]
fn completing_a_title_regeneration_is_refused() {
    assert!(matches!(
        refusal(json!({
            "type": "thread.title.regeneration.complete",
            "threadId": "task-1",
            "requestId": "cmd-1"
        })),
        CommandRefusal::Unsupported { .. }
    ));
}

#[test]
fn merge_back_maps_a_run_of_the_source_and_refuses_foreign_or_queued_runs() {
    let mut command = json!({"type": "thread.merge_back", "sourceThreadId": "side", "targetThreadId": "parent", "sourcePoint": {"type": "run", "runId": "side:2"}});
    let mapped = v1(command.clone());
    assert_eq!(mapped["atTurn"], json!(2));
    assert_eq!(mapped["sourceThreadId"], json!("side"));
    assert_eq!(mapped["targetThreadId"], json!("parent"));
    for run in ["parent:2", "side:0", "side:queued:x", "side:no"] {
        command["sourcePoint"]["runId"] = json!(run);
        assert!(matches!(
            refusal(command.clone()),
            CommandRefusal::Rejected { .. }
        ));
    }
}

#[test]
fn a_tag_write_is_refused_and_names_where_tags_are_set() {
    let CommandRefusal::Unsupported { reason, .. } = refusal(json!({
        "type": "thread.tag.set",
        "threadId": "task-1",
        "tag": "private",
        "tagged": true,
    })) else {
        panic!("expected an unsupported refusal");
    };
    assert!(reason.contains("moat set-tags"), "{reason}");
}
