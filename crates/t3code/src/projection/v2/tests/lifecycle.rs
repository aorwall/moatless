//! Run endings, background commands, context usage, compactions, handoffs and
//! child-outcome notices in the V2 projection.

use super::*;
use crate::transcript::TurnOutcome;

fn ended(turn_number: i32, status: &str) -> TurnOutcome {
    TurnOutcome {
        turn_number,
        status: status.to_string(),
        started_at: None,
        ended_at: Some(Utc::now()),
        error: None,
        reason: None,
        resets_at: None,
    }
}

fn usage_limited(turn_number: i32) -> TurnOutcome {
    TurnOutcome {
        error: Some("Claude hit its usage limit".to_string()),
        reason: Some("usage_limit".to_string()),
        resets_at: Some("2026-07-01T14:00:00Z".to_string()),
        ..ended(turn_number, "error")
    }
}

fn project(conversation: &Conversation, transcript: &Transcript) -> Value {
    thread_projection(
        conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        transcript,
        &origins(&HashMap::new()),
        &Lineage::root(&conversation.task_id),
        &[],
    )
}

fn items_of<'a>(projection: &'a Value, kind: &str) -> Vec<&'a Value> {
    projection["turnItems"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == json!(kind))
        .collect()
}

#[test]
fn an_older_turn_keeps_its_failure_and_its_stop() {
    let conversation = open_turn(conversation("task-1"), 3, "completed");
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            message("msg-2", Some(2), "second", MessageType::User),
            message("msg-3", Some(3), "third", MessageType::User),
        ],
        turn_outcomes: vec![usage_limited(1), ended(2, "stopped"), ended(3, "completed")],
        ..Transcript::default()
    };
    let projection = project(&conversation, &transcript);
    let statuses: Vec<&Value> = projection["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| &run["status"])
        .collect();
    assert_eq!(
        statuses,
        vec![&json!("failed"), &json!("interrupted"), &json!("completed")]
    );

    let errors = items_of(&projection, "error");
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(errors[0]["runId"], json!("task-1:1"));
    assert_eq!(errors[0]["nodeId"], json!("task-1:1:root"));
    assert_eq!(errors[0]["status"], json!("failed"));
    assert_eq!(
        errors[0]["failure"],
        json!({
            "class": "usage_limit",
            "message": "Claude hit its usage limit",
            "code": "usage_limit",
            "retryable": null,
            "resetAt": "2026-07-01T14:00:00.000Z"
        })
    );

    let stops = items_of(&projection, "run_interrupt_result");
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["runId"], json!("task-1:2"));
    assert_eq!(stops[0]["status"], json!("interrupted"));
    assert_eq!(stops[0]["message"], json!("Run stopped before it finished"));
}

#[test]
fn a_failed_newest_turn_classifies_the_shell_and_the_timeline() {
    let mut conversation = open_turn(conversation("task-1"), 1, "error");
    conversation.turn_ended_at = Some(Utc::now());
    conversation.turn_error = Some("Credentials were rejected".to_string());
    conversation.turn_error_reason = Some("authentication".to_string());
    let row = shell(&conversation);
    assert_eq!(row["status"], json!("failed"));
    assert_eq!(row["lastErrorClass"], json!("permission_error"));
    assert_eq!(row["usageLimitResetAt"], Value::Null);

    let projection = project(&conversation, &Transcript::default());
    let errors = items_of(&projection, "error");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["failure"]["class"], json!("permission_error"));
    assert_eq!(
        errors[0]["failure"]["message"],
        json!("Credentials were rejected")
    );
}

#[test]
fn a_usage_limit_names_when_it_lifts_on_the_shell() {
    let mut conversation = open_turn(conversation("task-1"), 1, "error");
    conversation.turn_ended_at = Some(Utc::now());
    conversation.turn_error_reason = Some("usage_limit".to_string());
    conversation.turn_error_resets_at = Some("2026-07-01T14:00:00Z".to_string());
    let row = shell(&conversation);
    assert_eq!(row["lastErrorClass"], json!("usage_limit"));
    assert_eq!(row["usageLimitResetAt"], json!("2026-07-01T14:00:00.000Z"));

    let completed = open_turn(super::conversation("task-1"), 1, "completed");
    assert_eq!(shell(&completed)["lastErrorClass"], Value::Null);
}

#[test]
fn a_command_outliving_its_turn_is_pending_background_work() {
    let mut idle = open_turn(conversation("task-1"), 1, "completed");
    idle.turn_ended_at = Some(Utc::now());
    idle.running_commands = 2;
    // The second label has not been mirrored yet, so the count still decides.
    idle.running_command_labels = vec!["moat gh run watch 4358 -R soaplabs/soap-infra".into()];
    let tasks = shell(&idle)["pendingBackgroundTasks"].clone();
    assert_eq!(
        tasks,
        json!([
            { "taskId": "task-1:command:1", "description": "moat gh run watch 4358 -R soaplabs/soap-infra", "kind": "background_task" },
            { "taskId": "task-1:command:2", "description": "Background command", "kind": "background_task" }
        ])
    );
    let projection = project(&idle, &Transcript::default());
    assert_eq!(
        projection["providerThreads"][0]["pendingBackgroundTasks"],
        tasks
    );

    let mut working = open_turn(conversation("task-1"), 1, "active");
    working.running_commands = 2;
    assert_eq!(shell(&working)["pendingBackgroundTasks"], json!([]));
}

fn reading(id: &str, position: i64, used: i64) -> Activity {
    let mut reading = activity(
        id,
        None,
        ActivityKind::ContextWindowUpdated,
        ActivityStatus::Completed,
    );
    reading.position = position;
    reading.payload = json!({ "usedTokens": used, "maxTokens": 200_000 });
    reading
}

#[test]
fn the_newest_context_reading_fills_the_meter() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        activities: vec![reading("ctx-1", 1, 40_000), reading("ctx-2", 2, 52_000)],
        ..Transcript::default()
    };
    let projection = project(&conversation, &transcript);
    let usage = &projection["providerTurns"][0]["tokenUsage"];
    assert_eq!(usage["usedTokens"], json!(52_000));
    assert_eq!(usage["maxTokens"], json!(200_000));
    assert!(usage["updatedAt"].is_string());
    assert_eq!(
        projection["providerThreads"][0]["contextUsage"],
        json!({ "usedTokens": 52_000, "maxTokens": 200_000 })
    );
}

#[test]
fn a_compaction_is_a_compaction_item() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let mut compacted = activity(
        "cmp-1",
        None,
        ActivityKind::Compaction,
        ActivityStatus::Completed,
    );
    compacted.payload = json!({ "summary": "We fixed the checkout." });
    let transcript = Transcript {
        activities: vec![compacted],
        ..Transcript::default()
    };
    let projection = project(&conversation, &transcript);
    let items = items_of(&projection, "compaction");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["status"], json!("completed"));
    assert_eq!(items[0]["runId"], json!("task-1:1"));
    assert_eq!(items[0]["summary"], json!("We fixed the checkout."));
    assert_eq!(items[0]["driver"], json!("claudeAgent"));
}

fn switched(at: chrono::DateTime<Utc>) -> Activity {
    let mut switched = activity(
        "evt-switch",
        None,
        ActivityKind::AgentSwitched,
        ActivityStatus::Completed,
    );
    switched.summary = "Switched from Codex to Claude Code".to_string();
    switched.payload = json!({ "summary": switched.summary, "from": "codex", "to": "claude-code" });
    switched.created_at = at;
    switched
}

fn started(turn_number: i32, at: chrono::DateTime<Utc>) -> TurnOutcome {
    TurnOutcome {
        started_at: Some(at),
        ..ended(turn_number, "completed")
    }
}

/// Turns 1 and 2 started before the switch, which came while turn 2 ran;
/// turn 3 is open.
fn switched_mid_turn() -> (Conversation, Transcript) {
    let switched_at = Utc::now() - chrono::Duration::minutes(5);
    let mut conversation = open_turn(conversation("task-1"), 3, "active");
    conversation.turn_started_at = Some(switched_at + chrono::Duration::minutes(2));
    let mut reply = message("msg-2r", Some(2), "still on Codex", MessageType::Assistant);
    reply.created_at = Some(switched_at + chrono::Duration::minutes(1));
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            message("msg-2", Some(2), "second", MessageType::User),
            reply,
            message("msg-3", Some(3), "third", MessageType::User),
        ],
        activities: vec![switched(switched_at)],
        turn_outcomes: vec![
            started(1, switched_at - chrono::Duration::minutes(10)),
            started(2, switched_at - chrono::Duration::minutes(1)),
        ],
        ..Transcript::default()
    };
    (conversation, transcript)
}

#[test]
fn an_agent_switch_is_a_ready_handoff_into_the_next_run() {
    let (conversation, transcript) = switched_mid_turn();
    let projection = project(&conversation, &transcript);

    let handoffs = items_of(&projection, "handoff");
    assert_eq!(handoffs.len(), 1);
    let handoff = handoffs[0];
    assert_eq!(handoff["runId"], json!("task-1:3"));
    assert_eq!(handoff["strategy"], json!("full_thread_summary"));
    assert_eq!(handoff["fromProviderInstanceIds"], json!(["codex"]));
    assert_eq!(handoff["toProviderInstanceId"], json!("claudeAgent"));
    let id = handoff["contextHandoffId"].clone();

    let context = &projection["contextHandoffs"][0];
    assert_eq!(context["id"], id);
    assert_eq!(context["status"], json!("ready"));
    assert_eq!(context["targetRunId"], json!("task-1:3"));
    assert_eq!(context["coveredRunOrdinals"], json!({ "from": 1, "to": 2 }));
    assert_eq!(projection["runs"][2]["contextHandoffId"], id);
    assert_eq!(projection["runs"][1]["contextHandoffId"], Value::Null);
    assert_eq!(projection["providerThreads"][0]["handoffIds"], json!([id]));
}

#[test]
fn a_switch_before_the_first_run_hands_nothing_over() {
    let switched_at = Utc::now() - chrono::Duration::minutes(5);
    let mut conversation = open_turn(conversation("task-1"), 1, "active");
    conversation.turn_started_at = Some(switched_at + chrono::Duration::minutes(1));
    let transcript = Transcript {
        messages: vec![message("msg-1", Some(1), "first", MessageType::User)],
        activities: vec![switched(switched_at)],
        ..Transcript::default()
    };
    let projection = project(&conversation, &transcript);
    assert!(items_of(&projection, "handoff").is_empty());
    assert_eq!(projection["contextHandoffs"], json!([]));
}

#[test]
fn a_child_outcome_is_a_notification_and_never_a_queued_run() {
    let conversation = awake(open_turn(conversation("task-1"), 1, "active"));
    let mut notice = held(message(
        "msg-child",
        None,
        "Child task `task-2` errored.\n\nError: boom",
        MessageType::User,
    ));
    notice.child_outcome = Some("errored".to_string());
    notice.source_task_id = Some("task-2".to_string());
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            notice,
        ],
        ..Transcript::default()
    };
    let projection = project(&conversation, &transcript);
    assert!(
        projection["runs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["status"] != json!("queued")),
        "{:#?}",
        projection["runs"]
    );
    let notices = items_of(&projection, "notification");
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0]["source"],
        json!({ "kind": "delegated_task", "taskIds": [], "childThreadId": "task-2" })
    );
    assert_eq!(notices[0]["outcome"], json!("failed"));
    assert_eq!(notices[0]["summary"], json!("Child task `task-2` errored."));
    assert_eq!(notices[0]["detail"], json!("Error: boom"));
}

fn interrupt_requests(conversation: &Conversation) -> Vec<Value> {
    let names = HashMap::new();
    let transcript = Transcript {
        messages: vec![message("msg-1", Some(1), "first", MessageType::User)],
        ..Transcript::default()
    };
    let (items, _) = turn_items_and_messages(conversation, &transcript, &origins(&names));
    items
        .into_iter()
        .filter(|item| item["type"] == json!("run_interrupt_request"))
        .collect()
}

/// A Stop the agent left unconfirmed keeps its run open and carries
/// `unresponsiveAt`, which is what the client offers Force stop for.
#[test]
fn an_unconfirmed_stop_is_an_unresponsive_interrupt_request_on_the_open_run() {
    let mut conversation = awake(open_turn(conversation("task-1"), 1, "active"));
    assert!(interrupt_requests(&conversation).is_empty());

    let at = Utc::now();
    conversation.turn_stop_unresponsive_at = Some(at);
    let requests = interrupt_requests(&conversation);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["runId"], json!("task-1:1"));
    assert_eq!(requests[0]["unresponsiveAt"], json!(instant(at)));
    assert_eq!(requests[0]["startedAt"], json!(instant(at)));

    let mut stopped = open_turn(conversation.clone(), 1, "stopped");
    stopped.turn_ended_at = Some(Utc::now());
    assert!(
        interrupt_requests(&stopped).is_empty(),
        "a closed turn reports its interrupt result instead"
    );
}
