//! The four panels, read off the transcript.

use super::fixtures::*;
use super::*;

// ── the four panels ─────────────────────────────────────────────

/// The card is read off the transcript: a Moatless proposed plan is an
/// `ExitPlanMode` call, and an unanswered one is a call still in progress.
#[test]
fn an_unanswered_plan_is_a_proposed_plan_carrying_no_answer() {
    let mut proposed = call(
        "msg-2",
        2,
        0,
        "call-1",
        moatless_types::activity::PLAN_TOOL,
        json!({ "plan": "1. Do the thing" }),
    );
    proposed.status = ActivityStatus::InProgress;

    let plans = detail(&transcript(Vec::new(), vec![proposed]))["proposedPlans"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["id"], "msg-2:0");
    assert_eq!(plans[0]["planMarkdown"], "1. Do the thing");
    assert_eq!(plans[0]["turnId"], "task-a:3");
    // Nothing behind these, so neither is claimed.
    assert_eq!(plans[0]["implementedAt"], Value::Null);
    assert_eq!(plans[0]["implementationThreadId"], Value::Null);
}

/// An answered plan keeps its card, and reports when the answer landed.
#[test]
fn an_answered_plan_is_still_a_proposed_plan_and_reports_when_it_was_answered() {
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

    let plans = detail(&transcript(Vec::new(), vec![answered, resolved]))["proposedPlans"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["id"], "msg-2:0");
    assert_eq!(plans[0]["planMarkdown"], "1. Do the thing");
    assert_eq!(plans[0]["implementedAt"], instant(at(103)));
    assert_eq!(plans[0]["updatedAt"], instant(at(103)));
}

/// The window holds a fixed span, so it can reach the call without the row that
/// answered it. A settled call reports an answer time either way.
#[test]
fn an_answered_plan_whose_answer_is_out_of_window_falls_back_to_its_own_instant() {
    let mut answered = call(
        "msg-2",
        2,
        0,
        "call-1",
        moatless_types::activity::PLAN_TOOL,
        json!({ "plan": "1. Do the thing" }),
    );
    answered.payload["result"] = json!({ "approved": true });

    let plans = detail(&transcript(Vec::new(), vec![answered]))["proposedPlans"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["implementedAt"], instant(at(102)));
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
        assert_eq!(
            detail(&transcript(Vec::new(), vec![proposed]))["proposedPlans"],
            json!([])
        );
    }
}

/// The meter reads the newest reading it holds and every earlier one in the
/// same turn renders nothing, so a snapshot carries one per turn rather than
/// the thousands a long thread accumulates.
#[test]
fn a_snapshot_carries_one_context_window_reading_per_turn() {
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

    let work = detail(&transcript(Vec::new(), readings))["activities"]
        .as_array()
        .unwrap()
        .clone();
    let meters: Vec<&Value> = work
        .iter()
        .filter(|row| row["kind"] == "context-window.updated")
        .collect();
    assert_eq!(meters.len(), 1, "only the newest survives: {work:#?}");
    assert_eq!(meters[0]["payload"]["usedTokens"], 3_000);
    assert_eq!(work.len(), 2, "everything else is untouched");
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

    let work = detail(&transcript(Vec::new(), vec![asked, plan, meter, resolved]))["activities"]
        .as_array()
        .unwrap()
        .clone();
    let by_kind = |kind: &str| {
        work.iter()
            .find(|row| row["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} row in {work:#?}"))
            .clone()
    };

    // The panel opens on a question whose id, header, question and options
    // are all present — the parser drops one missing any of them, and every
    // option needs a description of its own.
    let asked = by_kind("user-input.requested");
    assert_eq!(asked["tone"], "info");
    assert_eq!(asked["payload"]["requestId"], "call-1");
    let question = &asked["payload"]["questions"][0];
    for key in ["id", "header", "question"] {
        assert!(question[key].is_string(), "questions[0].{key}");
    }
    assert!(question["options"][0]["label"].is_string());
    assert!(question["options"][0]["description"].is_string());

    // And closes on a row naming the same request.
    let resolved = by_kind("user-input.resolved");
    assert_eq!(resolved["payload"]["requestId"], "call-1");

    // The plan panel reads `payload.plan`, each entry a step and its state.
    let plan = by_kind("turn.plan.updated");
    assert_eq!(plan["payload"]["plan"][0]["step"], "first");
    assert_eq!(plan["payload"]["plan"][0]["status"], "inProgress");

    // The meter reads a used count and, where one is known, a maximum.
    let meter = by_kind("context-window.updated");
    assert_eq!(meter["payload"]["usedTokens"], 1_000);
    assert_eq!(meter["payload"]["maxTokens"], 200_000);
}

/// A question the run has stopped waiting on reaches the client resolved, with
/// no resolution row of its own.
///
/// The call's own status is the fact, settled whoever settled it — covers an
/// answer from another surface, and one a resuming session dropped.
#[test]
fn a_question_whose_call_came_back_is_projected_resolved() {
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

    let kind_of = |rows: Vec<Activity>| {
        detail(&transcript(Vec::new(), rows))["activities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["payload"]["requestId"] == "call-1")
            .expect("the panel row")["kind"]
            .clone()
    };

    // Still waiting on a person: the panel opens.
    assert_eq!(
        kind_of(vec![asked.clone(), call.clone()]),
        "user-input.requested"
    );

    // The call came back. The panel row is the same row, and says so.
    call.status = ActivityStatus::Completed;
    assert_eq!(kind_of(vec![asked, call]), "user-input.resolved");
}

/// A Sandbox activity belongs to no Message and no turn, and its kind ends in
/// neither `.started` nor `.completed`, so it claims no position in the
/// client's own lifecycle sort.
#[test]
fn a_sandbox_activity_claims_no_lifecycle_position() {
    let mut stopped = activity("", 9, 0, ActivityKind::SandboxStatus, "stopped");
    stopped.message_uuid = None;
    let mut failed = activity("", 10, 0, ActivityKind::SandboxError, "error");
    failed.message_uuid = None;
    failed.is_error = true;

    let work = detail(&transcript(Vec::new(), vec![stopped, failed]))["activities"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(work[0]["kind"], "task.progress");
    assert_eq!(work[0]["tone"], "info");
    assert_eq!(work[1]["kind"], "runtime.error");
    assert_eq!(work[1]["tone"], "error");
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
fn a_message_outside_any_turn_reports_no_turn() {
    let mut conversation = a_conversation();
    conversation.messages[1].turn_number = None;
    let detail = detail(&conversation);
    assert_eq!(detail["messages"][0]["turnId"], "task-a:3");
    assert_eq!(detail["messages"][1]["turnId"], Value::Null);
    assert_eq!(detail["activities"][0]["turnId"], "task-a:3");
}

/// An assistant turn that only called tools contributes activities and no
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
    assert_eq!(detail["activities"].as_array().unwrap().len(), 1);
}
