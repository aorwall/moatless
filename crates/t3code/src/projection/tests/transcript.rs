//! What was said becomes messages and what was done becomes activities, each
//! numbered in transcript order.

use super::contract::*;
use super::fixtures::*;
use super::*;

#[test]
fn what_was_said_becomes_messages_and_what_was_done_becomes_activities() {
    let thread = detail(&a_conversation());

    let spoken = thread["messages"]
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

    let work = thread["activities"].as_array().unwrap();
    assert_eq!(
        work.iter()
            .map(|a| a["summary"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("Thinking"),
            json!("Command run"),
            json!("File change"),
            json!("Thinking")
        ]
    );
}

/// The client sorts a work log by `sequence`, falling back to `createdAt` as
/// strings.
///
/// Every activity of one Message shares that Message's single timestamp, so
/// without a sequence a turn's tool calls would arrive in an order the
/// transcript never had.
#[test]
fn activities_are_numbered_in_transcript_order() {
    let thread = detail(&a_conversation());
    let sequences = thread["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|activity| activity["sequence"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "activities ascend in transcript order: {sequences:?}"
    );
}

/// An activity delivered on its own has to sort into a work log built without
/// it, which is only true while the number is the one the row was written with
/// rather than a count taken while walking the transcript.
#[test]
fn an_activitys_number_does_not_depend_on_the_transcript_it_was_read_with() {
    let whole = detail(&a_conversation());
    let numbered: HashMap<String, i64> = whole["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|activity| {
            (
                activity["id"].as_str().unwrap().to_string(),
                activity["sequence"].as_i64().unwrap(),
            )
        })
        .collect();

    // The same activities read with none of the conversation around them,
    // as a change re-reading one turn would see them.
    let alone = transcript(Vec::new(), a_conversation().activities);
    for activity in detail(&alone)["activities"].as_array().unwrap() {
        let id = activity["id"].as_str().unwrap();
        assert_eq!(
            activity["sequence"].as_i64().unwrap(),
            numbered[id],
            "activity {id} keeps its number"
        );
    }
}

#[test]
fn a_tool_call_carries_its_outcome() {
    let thread = detail(&a_conversation());
    let work = thread["activities"].as_array().unwrap();

    let ran = &work[1];
    assert_eq!(ran["kind"], "tool.completed");
    assert_eq!(ran["tone"], "tool");
    assert_eq!(ran["payload"]["status"], "completed");
    assert_eq!(ran["payload"]["itemType"], "command_execution");
    assert!(ran["payload"]["data"].get("command").is_none());
    assert_eq!(ran["payload"]["detail"], "Bash: cargo build");
    assert_eq!(ran["payload"]["data"]["toolCallId"], "call-1");

    let failed = &work[2];
    assert_eq!(failed["tone"], "error");
    assert_eq!(failed["payload"]["status"], "failed");
    assert_eq!(failed["payload"]["itemType"], "file_change");
    assert!(failed["payload"]["data"].get("files").is_none());
    assert_eq!(
        failed["payload"]["detail"],
        r#"Edit: {"file_path":"src/main.rs"}"#
    );
}

/// A subagent reaches the client as one task it watches run.
///
/// The subagent's own work is not in the Task's stream; reported as a running
/// task while the call is open, with the completion landing on the same id so
/// the client replaces rather than adds a row.
#[test]
fn a_subagent_reaches_the_client_as_one_task_it_watches_run() {
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

    let projected = detail(&conversation);
    let running = projected["activities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["payload"]["data"]["toolCallId"] == "call-3")
        .expect("the spawning call is a row of the thread");
    assert_eq!(running["kind"], "task.progress");
    assert_eq!(running["tone"], "info");
    assert_eq!(running["payload"]["itemType"], "collab_agent_tool_call");
    // Headed by the word for what it is, with the brief beside it.
    assert_eq!(running["payload"]["summary"], "Subagent task");
    assert_eq!(running["payload"]["detail"], "code-reviewer: Audit the SQL");

    // The call comes back. The row is the same row.
    let running_id = running["id"].clone();
    let spawn = conversation
        .activities
        .iter_mut()
        .find(|activity| activity.tool_use_id.as_deref() == Some("call-3"))
        .unwrap();
    spawn.status = ActivityStatus::Completed;
    spawn.payload["status"] = json!("completed");

    let settled = detail(&conversation);
    let completed = settled["activities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["payload"]["data"]["toolCallId"] == "call-3")
        .expect("the settled call is still a row of the thread");
    assert_eq!(completed["kind"], "task.completed");
    assert_eq!(
        completed["id"], running_id,
        "the completion lands on the row the progress was reported on"
    );
}

/// What a subagent has spent is not on the row that names it: the numbers are
/// summed across Messages the transcript excludes and reach the row on the
/// transcript that read them, keyed by the call's own id.
#[test]
fn a_subagent_row_carries_what_its_agent_has_spent() {
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
    conversation.subagent_usage.insert(
        "call-3".to_string(),
        SubagentUsage {
            tool_use_id: "call-3".to_string(),
            responses: 1,
            input_tokens: 900,
            output_tokens: 100,
        },
    );

    let projected = detail(&conversation);
    let rows = projected["activities"].as_array().unwrap().clone();
    let spent = rows
        .iter()
        .find(|row| row["payload"]["taskId"] == "call-3")
        .expect("the spawning call is a row of the thread");
    assert_eq!(spent["payload"]["typedUsage"]["totalTokens"], 1000);
    assert_eq!(spent["payload"]["typedUsage"]["inputTokens"], 900);
    assert_eq!(spent["payload"]["typedUsage"]["outputTokens"], 100);

    // A row that names no subagent is left as it was.
    let ordinary = rows
        .iter()
        .find(|row| row["payload"]["data"]["toolCallId"] == "call-1")
        .expect("the conversation has an ordinary tool call");
    assert!(ordinary["payload"].get("typedUsage").is_none());
}

#[test]
fn a_codex_shell_call_reaches_the_client_as_t3s_ran_command_row() {
    let mut conversation = a_conversation();
    let call = &mut conversation.activities[1];
    call.summary = "codex.shell".to_string();
    call.payload["title"] = json!("codex.shell");

    let ran = &detail(&conversation)["activities"][1];
    assert_eq!(ran["summary"], "Ran command");
    assert_eq!(ran["payload"]["itemType"], "command_execution");
    assert_eq!(ran["payload"]["detail"], "cargo build");
    assert_eq!(ran["payload"]["data"]["command"], "cargo build");
    assert!(ran["payload"].get("title").is_none());
}

/// A call still running is reported as an update, not a completion.
///
/// The client collapses an update into the completion that follows it, so
/// reporting complete too early keeps the wrong row and drops the real outcome
/// when it arrives.
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

    let work = detail(&waiting)["activities"].as_array().unwrap().clone();
    assert_eq!(work[1]["kind"], "tool.updated");
    assert_eq!(work[1]["payload"]["status"], "inProgress");
    assert_eq!(work[1]["payload"]["detail"], "Bash: cargo build");
}

/// T3 presents what the tool was asked to do. A stored result preview is
/// Moatless UI data and must not replace that provider-derived request.
#[test]
fn a_stored_result_preview_does_not_replace_the_tool_request() {
    let mut conversation = a_conversation();
    conversation.activities[1].payload["result"] = json!("x".repeat(10_000));
    conversation.activities[1].detail = Some("xxx\n… truncated".to_string());
    assert_eq!(
        detail(&conversation)["activities"][1]["payload"]["detail"],
        "Bash: cargo build"
    );
}

/// The result is stored whole beside its preview, so the raw values never reach
/// this contract — which re-sends its whole snapshot every time the
/// conversation moves.
#[test]
fn an_activity_carries_no_raw_call_or_result_payload() {
    let payload = detail(&a_conversation())["activities"][1]["payload"].clone();
    for key in ["input", "result", "typedResult"] {
        assert!(
            payload.get(key).is_none(),
            "the T3 payload must not carry `{key}`"
        );
    }
}

/// A capped read that simply began in the middle would look like a conversation
/// that began there.
#[test]
fn a_capped_transcript_says_what_it_left_out() {
    let mut conversation = a_conversation();
    conversation.omitted = 12;
    let messages = detail(&conversation)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(messages[0]["role"], "system");
    assert!(
        messages[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("12 earlier messages")
    );
    assert_eq!(messages[0]["turnId"], Value::Null);
    assert_keys(&messages[0], "message", MESSAGE_REQUIRED, MESSAGE_OPTIONAL);
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
            TranscriptMessage {
                sandbox_wake: true,
                ..message(1, MessageType::User, "A background command has finished:")
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
