//! The plan, question and context-window panels, read off the transcript.

use super::fixtures::*;
use super::*;

// ── the panels ──────────────────────────────────────────────────

/// The card is read off the transcript: a Moatless proposed plan is an
/// `ExitPlanMode` call, and an unanswered one is a call still in progress.
#[test]
fn an_unanswered_plan_is_an_active_proposed_plan() {
    let mut proposed = call(
        "msg-2",
        2,
        0,
        "call-1",
        moatless_types::activity::PLAN_TOOL,
        json!({ "plan": "1. Do the thing" }),
    );
    proposed.status = ActivityStatus::InProgress;

    let projection = detail(&transcript(Vec::new(), vec![proposed]));
    let plans = projection["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["id"], "msg-2:0");
    assert_eq!(plans[0]["kind"], "proposed_plan");
    assert_eq!(plans[0]["markdown"], "1. Do the thing");
    assert_eq!(plans[0]["runId"], "task-a:3");
    assert_eq!(plans[0]["status"], "active");

    let items = projection["turnItems"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "proposed_plan");
    assert_eq!(items[0]["planId"], "msg-2:0");
    assert_eq!(items[0]["markdown"], "1. Do the thing");
    assert_eq!(items[0]["status"], "running");
    assert_eq!(items[0]["completedAt"], Value::Null);
}

/// An answered plan keeps its card, settled.
#[test]
fn an_answered_plan_is_still_a_proposed_plan_and_is_completed() {
    let mut answered = call(
        "msg-2",
        2,
        0,
        "call-1",
        moatless_types::activity::PLAN_TOOL,
        json!({ "plan": "1. Do the thing" }),
    );
    answered.payload["result"] = json!({ "approved": true });
    let mut resolved = activity("msg-3", 3, 0, ActivityKind::UserInputResolved, "Answered");
    resolved.payload = json!({ "requestId": "call-1", "answers": [{ "answer": "Approve" }] });

    let projection = detail(&transcript(Vec::new(), vec![answered, resolved]));
    let plans = projection["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["id"], "msg-2:0");
    assert_eq!(plans[0]["markdown"], "1. Do the thing");
    assert_eq!(plans[0]["status"], "completed");

    let items = projection["turnItems"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "proposed_plan");
    assert_eq!(items[0]["status"], "completed");
    assert_eq!(items[0]["completedAt"], instant(at(102)));
}

/// A card with no plan in it is a card rendering nothing.
#[test]
fn a_plan_call_carrying_no_plan_proposes_none() {
    for input in [json!({}), json!({ "plan": "   " })] {
        let mut proposed = call(
            "msg-2",
            2,
            0,
            "call-1",
            moatless_types::activity::PLAN_TOOL,
            input,
        );
        proposed.status = ActivityStatus::InProgress;
        let projection = detail(&transcript(Vec::new(), vec![proposed]));
        assert_eq!(projection["plans"], json!([]));
        assert_eq!(projection["turnItems"], json!([]));
    }
}

/// The meter reads the newest reading only, so a projection carries that one,
/// on the provider thread and on its turn, rather than the thousands a long
/// thread accumulates.
#[test]
fn a_projection_carries_only_the_newest_context_window_reading() {
    let mut readings: Vec<Activity> = (0..3)
        .map(|ordinal| {
            let mut reading = activity(
                "msg-2",
                2,
                ordinal,
                ActivityKind::ContextWindowUpdated,
                "Context window updated",
            );
            reading.message_uuid = None;
            reading.payload = json!({ "usedTokens": 1_000 * (i64::from(ordinal) + 1) });
            reading
        })
        .collect();
    readings.push(activity("msg-2", 2, 9, ActivityKind::Thinking, "Thinking"));

    let projection = detail(&transcript(Vec::new(), readings));
    assert_eq!(
        projection["providerThreads"][0]["contextUsage"],
        json!({ "usedTokens": 3_000 })
    );
    let turn = &projection["providerTurns"][0];
    assert_eq!(turn["id"], "task-a:3:provider-turn");
    assert_eq!(turn["tokenUsage"]["usedTokens"], 3_000);
    assert_eq!(
        projection["turnItems"],
        json!([]),
        "a reading is no item of the timeline"
    );
}

/// One case per kind the client derives a panel from, pinned against what that
/// client's own parser requires. A payload it cannot read is not a degraded
/// panel — it is no panel at all, silently.
#[test]
fn each_panel_reaches_the_client_in_the_shape_its_parser_requires() {
    let mut asked = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
    asked.payload = json!({
        "requestId": "call-1",
        "questions": [{
            "id": "Which store?",
            "header": "Storage",
            "question": "Which store?",
            "multiSelect": false,
            "options": [{ "label": "Postgres", "description": "Already deployed." }]
        }]
    });

    let mut resolved = activity("msg-3", 3, 0, ActivityKind::UserInputResolved, "Answered");
    resolved.payload = json!({
        "requestId": "call-1",
        "answers": { "Which store?": "Postgres" }
    });

    let mut plan = activity("msg-2", 2, 1, ActivityKind::PlanUpdated, "Plan updated");
    plan.payload = json!({
        "plan": [{ "step": "first", "status": "inProgress" }]
    });

    let mut meter = activity(
        "msg-2",
        2,
        2,
        ActivityKind::ContextWindowUpdated,
        "Context window updated",
    );
    meter.message_uuid = None;
    meter.payload = json!({ "usedTokens": 1_000, "maxTokens": 200_000 });

    let projection = detail(&transcript(Vec::new(), vec![asked, plan, meter, resolved]));
    let items = projection["turnItems"].as_array().unwrap();
    let by_type = |kind: &str| {
        items
            .iter()
            .find(|item| item["type"] == kind)
            .unwrap_or_else(|| panic!("no {kind} item in {items:#?}"))
            .clone()
    };

    // The panel opens on a question whose id, header, question and options
    // are all present — the parser drops one missing any of them, and every
    // option needs a description of its own.
    let asked = by_type("user_input_request");
    assert_eq!(asked["requestId"], "call-1");
    let question = &asked["questions"][0];
    for key in ["id", "header", "question"] {
        assert!(question[key].is_string(), "questions[0].{key}");
    }
    assert!(question["options"][0]["label"].is_string());
    assert!(question["options"][0]["description"].is_string());

    // And closes on the runtime request of the same id, carrying the answers.
    assert_eq!(asked["status"], "completed");
    let request = &projection["runtimeRequests"][0];
    assert_eq!(request["id"], "call-1");
    assert_eq!(request["kind"], "user_input");
    assert_eq!(request["status"], "resolved");
    assert_eq!(request["answers"], json!({ "Which store?": "Postgres" }));
    assert_eq!(request["resolvedAt"], instant(at(103)));

    // The todo list reads its steps, each a text and its state.
    let todo = by_type("todo_list");
    assert_eq!(todo["steps"][0]["text"], "first");
    assert_eq!(todo["steps"][0]["status"], "running");
    let plan = &projection["plans"][0];
    assert_eq!(plan["kind"], "todo_list");
    assert_eq!(plan["status"], "active");
    assert_eq!(plan["steps"], todo["steps"]);

    // The meter reads a used count and, where one is known, a maximum.
    let usage = &projection["providerThreads"][0]["contextUsage"];
    assert_eq!(usage["usedTokens"], 1_000);
    assert_eq!(usage["maxTokens"], 200_000);
}

/// A question the run has stopped waiting on reaches the client settled, with
/// no resolution row of its own.
///
/// The call's own status is the fact, settled whoever settled it — covers an
/// answer from another surface, and one a resuming session dropped.
#[test]
fn a_question_whose_call_came_back_is_projected_settled() {
    let mut asked = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
    asked.payload = json!({
        "requestId": "call-1",
        "questions": [{
            "id": "Which store?",
            "header": "Storage",
            "question": "Which store?",
            "multiSelect": false,
            "options": [{ "label": "Postgres", "description": "Already deployed." }]
        }]
    });

    let mut call = activity("msg-2", 2, 1, ActivityKind::ToolCall, "AskUserQuestion");
    call.tool_use_id = Some("call-1".to_string());
    call.status = ActivityStatus::InProgress;

    let state_of = |rows: Vec<Activity>| {
        let projection = detail(&transcript(Vec::new(), rows));
        let item = projection["turnItems"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["requestId"] == "call-1")
            .expect("the panel item")
            .clone();
        (
            item["status"].clone(),
            projection["runtimeRequests"][0]["status"].clone(),
        )
    };

    // Still waiting on a person: the panel opens.
    assert_eq!(
        state_of(vec![asked.clone(), call.clone()]),
        (json!("waiting"), json!("pending"))
    );

    // The call came back unanswered. The panel item is the same item, and
    // nobody is waiting on it any more.
    call.status = ActivityStatus::Completed;
    assert_eq!(
        state_of(vec![asked, call]),
        (json!("cancelled"), json!("expired"))
    );
}

/// The transcript is ordered by `timestamp` and the client compares these as
/// strings, so a row in another format would sort away from its neighbours. A
/// row whose timestamp will not parse still needs one.
#[test]
fn every_message_reports_a_comparable_instant() {
    let mut conversation = a_conversation();
    conversation.messages[0].timestamp = "not a timestamp".to_string();
    let messages = detail(&conversation)["messages"]
        .as_array()
        .unwrap()
        .clone();
    for message in &messages {
        let at = message["createdAt"].as_str().unwrap();
        assert!(
            DateTime::parse_from_rfc3339(at).is_ok(),
            "{at} is not an instant the client can compare"
        );
        assert_eq!(message["createdAt"], message["updatedAt"]);
    }
}

/// Nothing here is answered from a Turn that is not this Message's own. An
/// activity whose turn wrote no Message borrows the one before it.
#[test]
fn a_message_outside_any_turn_reports_no_run() {
    let mut conversation = a_conversation();
    conversation.messages[1].turn_number = None;
    let detail = detail(&conversation);
    assert_eq!(detail["messages"][0]["runId"], "task-a:3");
    assert_eq!(detail["messages"][1]["runId"], Value::Null);
    let items = detail["turnItems"].as_array().unwrap();
    let ran = items
        .iter()
        .find(|item| item["type"] == "command_execution")
        .expect("the command item");
    assert_eq!(ran["runId"], "task-a:3");
    let answer = items
        .iter()
        .find(|item| item["messageId"] == "msg-4")
        .expect("the answer item");
    assert_eq!(answer["runId"], Value::Null);
}

/// An assistant turn that only called tools contributes items and no
/// bubble — an empty one would render as a blank reply, and the writer stores
/// no Message row for it at all.
#[test]
fn a_turn_that_only_called_tools_says_nothing() {
    let conversation = transcript(
        Vec::new(),
        vec![call("msg-2", 1, 0, "call-1", "Grep", json!({}))],
    );
    let detail = detail(&conversation);
    assert!(detail["messages"].as_array().unwrap().is_empty());
    let items = detail["turnItems"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "dynamic_tool");
    assert_eq!(items[0]["toolName"], "Grep");
}
