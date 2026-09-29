//! The events one change emits, their sequence, and how each agrees with the
//! snapshot row it updates.

use super::contract::*;
use super::fixtures::*;
use super::*;

/// The change that names activity `activity_id`, at `position`.
fn activity_change(position: i64, activity_id: &str) -> ConversationChange {
    ConversationChange {
        position,
        message_uuid: None,
        activity_id: Some(activity_id.to_string()),
        reverted_to_turn: None,
    }
}

/// The rows a rewind dropped are gone, so the change that carries it names none
/// of them and this is read off the change itself.
#[test]
fn a_rewind_emits_the_cut_it_made() {
    let cut = ConversationChange {
        position: 7,
        message_uuid: None,
        activity_id: None,
        reverted_to_turn: Some(2),
    };
    let emitted = events(&cut, &transcript(Vec::new(), Vec::new()));
    assert_eq!(emitted.len(), 1);
    assert_keys(&emitted[0], "event", EVENT_REQUIRED, &[]);
    assert_eq!(emitted[0]["type"], "thread.reverted");
    assert_eq!(emitted[0]["payload"]["threadId"], "task-a");
    assert_eq!(emitted[0]["payload"]["turnCount"], json!(2));
}

/// A rewind that kept nothing is `0`, not an absence: the client empties the
/// thread rather than leaving what it holds in place.
#[test]
fn a_rewind_to_nothing_says_zero() {
    let cut = ConversationChange {
        position: 7,
        message_uuid: None,
        activity_id: None,
        reverted_to_turn: Some(0),
    };
    let emitted = events(&cut, &transcript(Vec::new(), Vec::new()));
    assert_eq!(emitted[0]["type"], "thread.reverted");
    assert_eq!(emitted[0]["payload"]["turnCount"], json!(0));
}

/// A call and the panel row derived from it are two rows in one change, and
/// each needs a sequence of its own.
///
/// The feed delivers an item only when its sequence is past the last delivered,
/// so two rows sharing one costs the second — always the panel, since it sorts
/// after its call.
#[test]
fn a_panel_row_and_its_call_do_not_share_one_sequence() {
    let mut call = activity("msg-2", 2, 0, ActivityKind::ToolCall, "AskUserQuestion");
    call.tool_use_id = Some("call-1".to_string());
    call.status = ActivityStatus::Completed;

    let mut panel = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
    panel.activity_id = "msg-2:0:user-input".to_string();
    panel.payload = json!({
        "requestId": "call-1",
        "questions": [{
            "id": "Which store?",
            "header": "Storage",
            "question": "Which store?",
            "multiSelect": false,
            "options": [{ "label": "Postgres", "description": "Already deployed." }]
        }]
    });

    let window = transcript(Vec::new(), vec![call, panel]);
    let emitted = events(&change(2, Some("msg-2")), &window);

    let sequences: Vec<i64> = emitted
        .iter()
        .map(|event| event["sequence"].as_i64().expect("a sequence"))
        .collect();
    let unique: HashSet<i64> = sequences.iter().copied().collect();
    assert_eq!(
        unique.len(),
        sequences.len(),
        "each row needs its own sequence, got {sequences:?}"
    );
}

/// A panel row reaches the client resolved even when the call it names is not
/// among the rows being projected.
///
/// A resuming session re-presents the call under a new Message; the call is
/// stored once (`tool_use_id` is unique) but the panel row carries none and is
/// written again, so the two end up on different Messages.
#[test]
fn a_panel_row_is_resolved_by_a_call_outside_the_rows_being_read() {
    let mut panel = activity("msg-9", 9, 0, ActivityKind::UserInputRequested, "Question");
    panel.activity_id = "msg-9:0:user-input".to_string();
    panel.payload = json!({
        "requestId": "call-1",
        "questions": [{
            "id": "Which store?",
            "header": "Storage",
            "question": "Which store?",
            "multiSelect": false,
            "options": [{ "label": "Postgres", "description": "Already deployed." }]
        }]
    });

    // The call came back long ago, on the Message it was first presented
    // on, which this change does not name.
    let mut window = transcript(Vec::new(), vec![panel]);
    window.settled_requests = HashSet::from(["call-1".to_string()]);
    let emitted = events(&change(9, Some("msg-9")), &window);

    let panel_event = emitted
        .iter()
        .find(|event| event["payload"]["activity"]["payload"]["requestId"] == "call-1")
        .expect("the panel row");
    assert_eq!(
        panel_event["payload"]["activity"]["kind"], "user-input.resolved",
        "a settled call resolves its panel wherever the call is stored"
    );
}

/// A change's items climb, so a reader that delivers each sequence once keeps
/// all of them.
///
/// A derived row's slot sits above the blocks, landing between two in row
/// order; comparing against the last delivered sequence would otherwise drop
/// every block after it.
#[test]
fn a_changes_items_climb_so_a_later_block_survives_a_derived_row() {
    let mut call = activity("msg-2", 2, 0, ActivityKind::ToolCall, "AskUserQuestion");
    call.tool_use_id = Some("call-1".to_string());

    let mut panel = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
    panel.activity_id = "msg-2:0:user-input".to_string();
    panel.payload = json!({
        "requestId": "call-1",
        "questions": [{
            "id": "Which store?",
            "header": "Storage",
            "question": "Which store?",
            "multiSelect": false,
            "options": [{ "label": "Postgres", "description": "Already deployed." }]
        }]
    });

    // The block after the one the question was asked on.
    let mut later = activity("msg-2", 2, 1, ActivityKind::ToolCall, "Bash");
    later.tool_use_id = Some("call-2".to_string());

    let window = transcript(Vec::new(), vec![call, panel, later]);
    let emitted = events(&change(2, Some("msg-2")), &window);

    let sequences: Vec<i64> = emitted
        .iter()
        .map(|event| event["sequence"].as_i64().expect("a sequence"))
        .collect();
    let mut climbing = sequences.clone();
    climbing.sort_unstable();
    assert_eq!(
        sequences, climbing,
        "items must be emitted in sequence order"
    );
    assert_eq!(sequences.len(), 3, "every row reaches the reader");
}

/// A person's Message is one bubble and no work-log rows.
#[test]
fn a_change_to_a_user_message_carries_the_bubble_it_wrote() {
    let window = a_conversation();
    let sent = events(&change(1, Some("msg-1")), &window);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["type"], "thread.message-sent");
    assert_eq!(sent[0]["payload"]["threadId"], "task-a");
    assert_eq!(sent[0]["payload"]["messageId"], "msg-1");
    assert_eq!(sent[0]["payload"]["role"], "user");
    assert_eq!(sent[0]["payload"]["text"], "Fix the build");
    assert_eq!(sent[0]["payload"]["streaming"], false);
    assert_eq!(sent[0]["payload"]["turnId"], "task-a:3");
}

/// An assistant Message that only called tools has no bubble, so its change
/// says only what the work log shows.
#[test]
fn a_change_to_a_tool_calling_message_carries_only_its_work_log_rows() {
    let window = a_conversation();
    let sent = events(&change(2, Some("msg-2")), &window);
    assert!(
        sent.iter()
            .all(|event| event["type"] == "thread.activity-appended"),
        "no bubble for a turn that said nothing: {sent:#?}"
    );
    let summaries: Vec<&str> = sent
        .iter()
        .map(|event| event["payload"]["activity"]["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, vec!["Thinking", "Command run", "File change"]);
    assert_eq!(sent[0]["payload"]["threadId"], "task-a");
}

/// A change naming an activity rather than a Message — a Sandbox lifecycle row,
/// a context-window reading — carries that one work-log row alone: no bubble,
/// no `thread.session-set`.
///
/// Numbered `ordinal + 1`, the same value its snapshot row carries.
#[test]
fn a_change_naming_an_activity_carries_that_row_and_no_session() {
    let mut ready = activity("", 6, 0, ActivityKind::SandboxStatus, "Ready");
    ready.activity_id = "evt-ready".to_string();
    ready.message_uuid = None;
    let window = transcript(Vec::new(), vec![ready]);

    let sent = events(&activity_change(6, "evt-ready"), &window);
    assert_eq!(sent.len(), 1, "one work-log row and no session: {sent:#?}");
    assert_eq!(sent[0]["type"], "thread.activity-appended");
    assert_eq!(sent[0]["payload"]["activity"]["id"], "evt-ready");
    assert_eq!(sent[0]["payload"]["activity"]["summary"], "Ready");
    assert_eq!(sent[0]["sequence"], 6 * EVENTS_PER_CHANGE + 1);
}

/// A change naming an activity the window does not hold says nothing — a
/// complete answer, the same one a change naming an absent Message gives.
#[test]
fn a_change_naming_an_activity_not_in_the_window_says_nothing() {
    assert!(events(&activity_change(6, "evt-gone"), &a_conversation()).is_empty());
}

/// A tool result settles the row its call already wrote; the change records the
/// *calling* Message.
///
/// The outcome reaches the client as that same row re-sent under the same id,
/// which the client upserts — no separate result row to search for.
#[test]
fn a_settled_call_is_re_sent_under_the_id_the_snapshot_gave_it() {
    let window = a_conversation();
    let sent = events(&change(2, Some("msg-2")), &window);
    let resolved: Vec<(&str, &str)> = sent
        .iter()
        .filter(|event| event["payload"]["activity"]["kind"] != "task.progress")
        .map(|event| {
            (
                event["payload"]["activity"]["summary"].as_str().unwrap(),
                event["payload"]["activity"]["payload"]["status"]
                    .as_str()
                    .unwrap(),
            )
        })
        .collect();
    assert_eq!(
        resolved,
        vec![("Command run", "completed"), ("File change", "failed")]
    );
    assert_eq!(sent[1]["payload"]["activity"]["id"], "msg-2:1");
}

/// The client drops an event whose sequence it has passed, so two may never
/// share one — including across the boundary between two changes.
#[test]
fn every_event_of_a_change_has_its_own_ascending_sequence() {
    let window = a_conversation();
    let first: Vec<i64> = events(&change(2, Some("msg-2")), &window)
        .iter()
        .map(|event| event["sequence"].as_i64().unwrap())
        .collect();
    let second: Vec<i64> = events(&change(4, Some("msg-4")), &window)
        .iter()
        .map(|event| event["sequence"].as_i64().unwrap())
        .collect();
    assert!(first.windows(2).all(|pair| pair[0] < pair[1]), "{first:?}");
    assert!(
        first.last() < second.first(),
        "a later change is numbered above an earlier one: {first:?} then {second:?}"
    );
}

/// The invariant the whole numbering exists for: a row a client received as an
/// event and the same row read out of a snapshot have to carry one number, or
/// sorting one against the other means nothing.
#[test]
fn an_activitys_event_is_numbered_the_same_as_its_snapshot_row() {
    let window = a_conversation();
    let snapshot: HashMap<String, i64> = detail(&window)["activities"]
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

    for change in [change(2, Some("msg-2")), change(4, Some("msg-4"))] {
        for event in events(&change, &window) {
            let Some(activity) = event["payload"].get("activity") else {
                continue;
            };
            let id = activity["id"].as_str().unwrap();
            assert_eq!(
                event["sequence"].as_i64().unwrap(),
                snapshot[id],
                "activity {id} is numbered the same either way"
            );
        }
    }
}

/// Slot 0 belongs to the bubble whether or not the Message wrote one, so a
/// turn's first activity is numbered the same in a change that carries a bubble
/// and one that does not.
#[test]
fn the_bubbles_slot_is_reserved_even_when_no_bubble_is_sent() {
    let window = a_conversation();
    let silent = events(&change(2, Some("msg-2")), &window);
    assert_eq!(silent[0]["payload"]["activity"]["summary"], "Thinking");
    assert_eq!(silent[0]["sequence"], 2 * EVENTS_PER_CHANGE + 1);

    let spoken = events(&change(4, Some("msg-4")), &window);
    assert_eq!(spoken[0]["type"], "thread.message-sent");
    assert_eq!(spoken[0]["sequence"], 4 * EVENTS_PER_CHANGE);
    assert_eq!(spoken[1]["payload"]["activity"]["summary"], "Thinking");
    assert_eq!(spoken[1]["sequence"], 4 * EVENTS_PER_CHANGE + 1);
}

/// A turn of ten or more blocks: the client falls back to comparing ids as
/// strings, where `…:10` sorts before `…:2`. The number is what keeps the tenth
/// block after the second.
#[test]
fn a_turn_of_ten_blocks_is_numbered_in_block_order() {
    let activities: Vec<Activity> = (0..12)
        .map(|ordinal| activity("msg-2", 2, ordinal, ActivityKind::Thinking, "Thinking"))
        .collect();
    let ids: Vec<String> = activities
        .iter()
        .map(|activity| activity.activity_id.clone())
        .collect();
    let mut sorted_by_id = ids.clone();
    sorted_by_id.sort();
    assert_ne!(
        ids, sorted_by_id,
        "the fixture has to be one the id order gets wrong"
    );

    let sequences: Vec<i64> = detail(&transcript(Vec::new(), activities))["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|activity| activity["sequence"].as_i64().unwrap())
        .collect();
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "{sequences:?}"
    );
}

/// A change naming a uuid the window holds nothing for — no Message and no
/// activity, so deleted, or one of the subagent rows no transcript reads — is a
/// complete answer of nothing.
#[test]
fn a_change_naming_a_row_that_is_not_there_says_nothing() {
    assert!(events(&change(4, Some("msg-404")), &a_conversation()).is_empty());
}

/// A change naming no Message is the conversation's own, and the client derives
/// the turn it renders from the session it is sent.
#[test]
fn a_change_to_the_conversation_itself_carries_its_session() {
    let sent = events(&change(4, None), &transcript(Vec::new(), Vec::new()));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["type"], "thread.session-set");
    assert_eq!(sent[0]["payload"]["threadId"], "task-a");
    assert_eq!(sent[0]["payload"]["session"]["status"], "running");
    assert_eq!(sent[0]["payload"]["session"]["activeTurnId"], "task-a:3");
}

/// The contract's session payload has no empty value, so a conversation that
/// has never had a Sandbox has nothing to say rather than a null to send.
#[test]
fn a_conversation_with_no_sandbox_sends_no_session_event() {
    let without = Conversation {
        sandbox_desired_state_raw: None,
        ..running_conversation("task-a", "ws-a")
    };
    assert!(
        change_events(
            &without,
            &change(4, None),
            &transcript(Vec::new(), Vec::new()),
            &OriginContext {
                frontend_url: "http://localhost:3000",
                source_task_names: &HashMap::new(),
            },
        )
        .is_empty()
    );
}

/// A message event carries the same values the snapshot's own bubble does,
/// under the contract's other set of names for them.
#[test]
fn a_message_event_and_a_snapshot_bubble_agree() {
    let window = a_conversation();
    let listed = detail(&window);
    let bubble = &listed["messages"].as_array().unwrap()[0];
    let sent = events(&change(1, Some("msg-1")), &window);
    let payload = &sent[0]["payload"];
    assert_eq!(payload["messageId"], bubble["id"]);
    assert_eq!(payload["role"], bubble["role"]);
    assert_eq!(payload["text"], bubble["text"]);
    assert_eq!(payload["turnId"], bubble["turnId"]);
    assert_eq!(payload["streaming"], bubble["streaming"]);
    assert_eq!(payload["createdAt"], bubble["createdAt"]);
    assert_eq!(payload["updatedAt"], bubble["updatedAt"]);
}

/// An activity event carries the activity a snapshot would have carried,
/// unchanged — including the number it sorts by, which is what lets it arrive
/// long after the snapshot it has to sort into.
#[test]
fn an_activity_event_and_a_snapshot_activity_agree() {
    let window = a_conversation();
    let listed = detail(&window)["activities"].as_array().unwrap().clone();
    let sent = events(&change(2, Some("msg-2")), &window);
    for (event, activity) in sent.iter().zip(listed.iter()) {
        assert_eq!(event["payload"]["activity"], *activity);
    }
}

// ── the plan's card ─────────────────────────────────────────────────

/// A page open while the agent proposes is sent the plan, not only the panel
/// asking about it: `proposedPlans` rides no other event, so without this the
/// client is asked to approve a plan it has never been sent.
#[test]
fn a_change_holding_a_plan_call_carries_the_plan_itself() {
    let mut proposed = call(
        "msg-2",
        2,
        0,
        "call-1",
        moatless_types::activity::PLAN_TOOL,
        json!({ "plan": "1. Do the thing" }),
    );
    proposed.status = ActivityStatus::InProgress;

    let mut panel = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
    panel.activity_id = "msg-2:0:user-input".to_string();
    panel.payload = json!({
        "requestId": "call-1",
        "questions": [{
            "id": moatless_types::activity::PLAN_QUESTION,
            "header": "Plan",
            "question": moatless_types::activity::PLAN_QUESTION,
            "multiSelect": false,
            "options": [{ "label": moatless_types::activity::PLAN_APPROVE }]
        }]
    });

    let window = transcript(Vec::new(), vec![proposed, panel]);
    let emitted = events(&change(2, Some("msg-2")), &window);

    let card = emitted
        .iter()
        .find(|event| event["type"] == "thread.proposed-plan-upserted")
        .expect("the plan's card");
    assert_eq!(card["payload"]["threadId"], "task-a");
    assert_eq!(
        card["payload"]["proposedPlan"]["planMarkdown"],
        "1. Do the thing"
    );
    assert_eq!(card["payload"]["proposedPlan"]["id"], "msg-2:0");
    assert_eq!(
        card["payload"]["proposedPlan"]["implementedAt"],
        Value::Null
    );
    assert_eq!(card["sequence"], 2 * EVENTS_PER_CHANGE + 768 + 1);

    // The call, its panel and its card are three events off one row, and the
    // feed drops an item whose sequence it has already passed.
    let sequences: Vec<i64> = emitted
        .iter()
        .map(|event| event["sequence"].as_i64().expect("a sequence"))
        .collect();
    let mut climbing = sequences.clone();
    climbing.sort_unstable();
    assert_eq!(
        sequences, climbing,
        "items must be emitted in sequence order"
    );
    let unique: HashSet<i64> = sequences.iter().copied().collect();
    assert_eq!(
        unique.len(),
        sequences.len(),
        "each row needs its own sequence, got {sequences:?}"
    );
}

/// The answer re-sends the card, this time answered, which is what stops the
/// client offering the plan.
///
/// It is written on a Message of its own and settles the call in place on the
/// Message it was asked on, so this change holds the resolved row and not the
/// call.
#[test]
fn a_change_answering_a_plan_carries_the_card_it_answered() {
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

    let window = transcript(Vec::new(), vec![answered, resolved]);
    let emitted = events(&change(3, Some("msg-3")), &window);

    let card = emitted
        .iter()
        .find(|event| event["type"] == "thread.proposed-plan-upserted")
        .expect("the plan's card");
    // The card the client already holds, so the upsert lands on it.
    assert_eq!(card["payload"]["proposedPlan"]["id"], "msg-2:0");
    assert_eq!(
        card["payload"]["proposedPlan"]["implementedAt"],
        instant(at(103))
    );
    assert_eq!(card["sequence"], 3 * EVENTS_PER_CHANGE + 768 + 1);
}

/// The join is by `requestId`, and only an `ExitPlanMode` call has a plan to
/// render.
#[test]
fn a_change_answering_a_question_carries_no_card() {
    let mut asked = call(
        "msg-2",
        2,
        0,
        "call-1",
        moatless_types::activity::QUESTION_TOOL,
        json!({ "questions": [] }),
    );
    asked.payload["result"] = json!({ "answer": "Postgres" });
    let mut resolved = activity("msg-3", 3, 0, ActivityKind::UserInputResolved, "Answered");
    resolved.payload = json!({ "requestId": "call-1", "answers": [{ "answer": "Postgres" }] });

    let window = transcript(Vec::new(), vec![asked, resolved]);
    assert!(
        events(&change(3, Some("msg-3")), &window)
            .iter()
            .all(|event| event["type"] != "thread.proposed-plan-upserted"),
        "a question is not a plan"
    );
}

/// A card rendering nothing is not sent.
#[test]
fn a_change_holding_a_plan_call_with_no_plan_carries_no_card() {
    let mut proposed = call(
        "msg-2",
        2,
        0,
        "call-1",
        moatless_types::activity::PLAN_TOOL,
        json!({}),
    );
    proposed.status = ActivityStatus::InProgress;

    let window = transcript(Vec::new(), vec![proposed]);
    assert!(
        events(&change(2, Some("msg-2")), &window)
            .iter()
            .all(|event| event["type"] != "thread.proposed-plan-upserted"),
        "nothing to render, nothing to send"
    );
}

// ── the settled turn's checkpoint ───────────────────────────────────

/// The newest settled turn, as the conversation looks once turn 3 ended.
fn settled_turn() -> (Conversation, Transcript) {
    let mut conversation = running_conversation("task-a", "ws-a");
    conversation.turn_status = Some("completed".to_string());
    let turn = transcript(
        vec![
            message(1, MessageType::User, "do a thing"),
            message(2, MessageType::Assistant, "did it"),
        ],
        Vec::new(),
    );
    (conversation, turn)
}

/// The checkpoint a live page is sent is the one a reload would show last, so
/// the rewind control reaches a turn that ended while the page was open.
#[test]
fn a_settled_turn_is_sent_the_checkpoint_a_snapshot_ends_on() {
    let (conversation, turn) = settled_turn();
    let snapshot = detail_of(conversation.clone(), &turn)["checkpoints"][0].clone();
    let sent = turn_settled_events(&conversation, &change(4, None), &turn);
    let [(3, sent)] = sent.as_slice() else {
        panic!("one checkpoint, for turn 3: {sent:#?}");
    };
    let sent = &sent["event"];
    assert_keys(sent, "event", EVENT_REQUIRED, &[]);
    assert_eq!(sent["type"], "thread.turn-diff-completed");
    assert_eq!(sent["occurredAt"], snapshot["completedAt"]);
    let mut expected = snapshot;
    expected["threadId"] = json!("task-a");
    assert_eq!(sent["payload"], expected);
}

/// Two turns settling between polls both reach the client, oldest first, in
/// the slots above the session the same change carries.
#[test]
fn every_settled_turn_is_sent_after_the_session_event() {
    let (conversation, _) = settled_turn();
    let mut earlier = message(1, MessageType::Assistant, "first");
    earlier.turn_number = Some(3);
    let mut later = message(2, MessageType::Assistant, "second");
    later.turn_number = Some(4);
    let conversation = Conversation {
        turn_number: Some(4),
        ..conversation
    };
    let sent = turn_settled_events(
        &conversation,
        &change(4, None),
        &transcript(vec![earlier, later], Vec::new()),
    );
    let turns: Vec<(i32, i64, &str)> = sent
        .iter()
        .map(|(turn, item)| {
            (
                *turn,
                item["event"]["sequence"].as_i64().unwrap(),
                item["event"]["payload"]["assistantMessageId"]
                    .as_str()
                    .unwrap(),
            )
        })
        .collect();
    assert_eq!(
        turns,
        [
            (3, 4 * EVENTS_PER_CHANGE + 1, "msg-1"),
            (4, 4 * EVENTS_PER_CHANGE + 2, "msg-2"),
        ]
    );
}

/// A conversation with no settled turn, or only the open one, has no cut to
/// offer.
#[test]
fn no_settled_turn_sends_no_checkpoint() {
    let (conversation, _) = settled_turn();
    let none = transcript(Vec::new(), Vec::new());
    assert!(turn_settled_events(&conversation, &change(4, None), &none).is_empty());
    let (_, turn) = settled_turn();
    let running = running_conversation("task-a", "ws-a");
    assert!(turn_settled_events(&running, &change(4, None), &turn).is_empty());
}

#[test]
fn only_a_change_naming_nothing_is_a_session_change() {
    assert!(is_session_change(&change(4, None)));
    assert!(!is_session_change(&change(4, Some("msg-1"))));
    assert!(!is_session_change(&activity_change(4, "act-1")));
    let cut = ConversationChange {
        reverted_to_turn: Some(1),
        ..change(4, None)
    };
    assert!(!is_session_change(&cut));
}
