//! What was said becomes messages and what was done becomes turn items, each
//! placed in transcript order.

use super::fixtures::*;
use super::*;
use std::collections::HashSet;

/// The turn items of `projection` that stand for an activity rather than a
/// Message.
fn work(projection: &Value) -> Vec<Value> {
    projection["turnItems"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["id"].as_str().unwrap().starts_with("item:activity:"))
        .cloned()
        .collect()
}

#[test]
fn what_was_said_becomes_messages_and_what_was_done_becomes_items() {
    let projection = detail(&a_conversation());

    let spoken = projection["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| (message["role"].clone(), message["text"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        spoken,
        vec![
            (json!("user"), json!("Fix the build")),
            (json!("assistant"), json!("Done."))
        ],
        "the turn that only called tools is not a bubble of its own"
    );

    let types = work(&projection)
        .iter()
        .map(|item| item["type"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        types,
        vec![json!("command_execution"), json!("file_change")],
        "a thinking row with no text renders nothing"
    );
}

/// Every activity of one Message shares that Message's single timestamp, so
/// the timeline has to keep a turn's tool calls in the order the transcript
/// had them, numbered without gaps.
#[test]
fn items_are_numbered_in_transcript_order() {
    let projection = detail(&a_conversation());
    let items = projection["turnItems"].as_array().unwrap();
    let ordinals = items
        .iter()
        .map(|item| item["ordinal"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ordinals, (0..items.len() as u64).collect::<Vec<_>>());
    let ids = work(&projection)
        .iter()
        .map(|item| item["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            json!("item:activity:msg-2:1"),
            json!("item:activity:msg-2:2")
        ]
    );
}

/// An activity delivered on its own has to land on the item a timeline built
/// without it would hold, which is only true while the id is the row's own
/// rather than a count taken while walking the transcript.
#[test]
fn an_items_id_does_not_depend_on_the_transcript_it_was_read_with() {
    let whole: HashSet<String> = work(&detail(&a_conversation()))
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect();

    // The same activities read with none of the conversation around them,
    // as a change re-reading one turn would see them.
    let alone = transcript(Vec::new(), a_conversation().activities);
    let alone: HashSet<String> = work(&detail(&alone))
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect();
    assert!(!alone.is_empty());
    assert_eq!(alone, whole);
}

#[test]
fn a_tool_call_carries_its_outcome() {
    let work = work(&detail(&a_conversation()));

    let ran = &work[0];
    assert_eq!(ran["type"], "command_execution");
    assert_eq!(ran["status"], "completed");
    assert_eq!(ran["input"], "cargo build");
    assert_eq!(ran["output"], "Compiling");
    assert_eq!(ran["exitCode"], 0);
    assert_eq!(ran["outputIndicatesFailure"], false);

    let failed = &work[1];
    assert_eq!(failed["type"], "file_change");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["fileName"], "src/main.rs");
}

/// A subagent reaches the client as one item it watches run.
///
/// The subagent's own work is not in the Task's stream; reported as running
/// while the call is open, with the completion landing on the same id so the
/// client replaces rather than adds an item.
#[test]
fn a_subagent_reaches_the_client_as_one_item_it_watches_run() {
    let mut conversation = a_conversation();
    let mut spawn = call(
        "msg-2",
        2,
        3,
        "call-3",
        "Task",
        json!({ "subagent_type": "code-reviewer", "description": "Audit the SQL" }),
    );
    spawn.status = ActivityStatus::InProgress;
    spawn.payload["status"] = json!("inProgress");
    conversation.activities.push(spawn);

    let subagent_of = |conversation: &Transcript| {
        work(&detail(conversation))
            .into_iter()
            .find(|item| item["type"] == "subagent")
            .expect("the spawning call is an item of the thread")
    };
    let running = subagent_of(&conversation);
    assert_eq!(running["status"], "running");
    assert_eq!(running["completedAt"], Value::Null);
    assert_eq!(running["title"], "Audit the SQL");
    assert_eq!(running["prompt"], "Audit the SQL");
    assert_eq!(running["origin"], "provider_native");

    // The call comes back. The item is the same item.
    let spawn = conversation
        .activities
        .iter_mut()
        .find(|activity| activity.tool_use_id.as_deref() == Some("call-3"))
        .unwrap();
    spawn.status = ActivityStatus::Completed;
    spawn.payload["status"] = json!("completed");

    let completed = subagent_of(&conversation);
    assert_eq!(completed["status"], "completed");
    assert_eq!(
        completed["id"], running["id"],
        "the completion lands on the item the progress was reported on"
    );
}

#[test]
fn a_codex_shell_call_reaches_the_client_as_a_command_execution() {
    let mut conversation = a_conversation();
    let call = &mut conversation.activities[1];
    call.summary = "codex.shell".to_string();
    call.payload["title"] = json!("codex.shell");

    let ran = &work(&detail(&conversation))[0];
    assert_eq!(ran["type"], "command_execution");
    assert_eq!(ran["input"], "cargo build");
}

/// A call still running is reported running, not completed.
///
/// The client replaces an item with the next one of the same id, so reporting
/// complete too early shows an outcome before there is one.
#[test]
fn a_tool_call_with_no_result_yet_is_still_in_progress() {
    let mut waiting = a_conversation();
    waiting.activities[1].status = ActivityStatus::InProgress;
    waiting.activities[1].payload["status"] = json!("inProgress");
    waiting.activities[1]
        .payload
        .as_object_mut()
        .unwrap()
        .remove("result");
    waiting.activities[1].detail = None;

    let ran = &work(&detail(&waiting))[0];
    assert_eq!(ran["status"], "running");
    assert_eq!(ran["completedAt"], Value::Null);
    assert_eq!(ran["output"], Value::Null);
    assert_eq!(ran["exitCode"], Value::Null);
    assert_eq!(ran["input"], "cargo build");
}

/// T3 presents what the tool was asked to do. A stored result preview is
/// Moatless UI data and must not replace that provider-derived request.
#[test]
fn a_stored_result_preview_does_not_replace_the_tool_request() {
    let mut conversation = a_conversation();
    conversation.activities[1].payload["result"] = json!("x".repeat(10_000));
    conversation.activities[1].detail = Some("xxx\n… truncated".to_string());
    assert_eq!(work(&detail(&conversation))[0]["input"], "cargo build");
}

/// The run's own closing line is neither the person's nor the agent's.
#[test]
fn a_result_row_is_spoken_by_the_system() {
    let conversation = transcript(
        vec![message(1, MessageType::Result, "Stopped after an error")],
        Vec::new(),
    );
    let messages = detail(&conversation)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["text"], "Stopped after an error");
}

/// The sandbox waking its own agent is nobody speaking; the reply it prompted
/// still shows.
#[test]
fn a_sandbox_wake_is_not_a_bubble() {
    let conversation = transcript(
        vec![
            {
                let mut wake = message(1, MessageType::User, "A background command has finished:");
                wake.sandbox_wake = true;
                wake
            },
            message(2, MessageType::Assistant, "The build passed."),
        ],
        Vec::new(),
    );
    let spoken = detail(&conversation)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| (message["role"].clone(), message["text"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        spoken,
        vec![(json!("assistant"), json!("The build passed."))]
    );
}
