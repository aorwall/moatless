use super::*;
use crate::projection::v2_items::command_exit_code;
use crate::projection::v2_items::plans;
use chrono::Utc;
use moatless_types::agent::AgentProfile;
use moatless_types::message::MessageRole;
use moatless_types::task::Visibility;
use std::collections::HashMap;

fn conversation(task_id: &str) -> Conversation {
    Conversation {
        task_id: task_id.to_string(),
        workspace_id: Some("ws-a".to_string()),
        name: Some("Fix the checkout".to_string()),
        branch: Some("feat/checkout".to_string()),
        agent_profile: AgentProfile::ClaudeCode.into(),
        created_at: "2026-07-01T09:00:00.000Z".to_string(),
        created_at_db: Some(Utc::now()),
        updated_at: Some(Utc::now()),
        closed_at: None,
        visibility: Visibility::Private,
        owner_name: None,
        turn_number: None,
        turn_status: None,
        turn_started_at: None,
        turn_ended_at: None,
        turn_error: None,
        sandbox_desired_state_raw: None,
        sandbox_stop_reason_raw: None,
        sandbox_started_at: None,
        sandbox_updated_at: None,
        running_commands: 0,
        latest_user_message_at: None,
        latest_queued_message_at: None,
        assistant_model: None,
        requested_model: None,
        pending_question: false,
        pending_plan: false,
        settled_override: None,
        settled_at: None,
        snoozed_until: None,
        snoozed_at: None,
        pull_requests: None,
    }
}

fn open_turn(mut conversation: Conversation, turn_number: i32, status: &str) -> Conversation {
    conversation.turn_number = Some(turn_number);
    conversation.turn_status = Some(status.to_string());
    conversation.turn_started_at = Some(Utc::now());
    conversation
}

fn message(
    uuid: &str,
    turn_number: Option<i32>,
    text: &str,
    kind: MessageType,
) -> TranscriptMessage {
    TranscriptMessage {
        id: 1,
        uuid: uuid.to_string(),
        message_type: kind,
        message_role: MessageRole::User,
        message_text: Some(text.to_string()),
        user_type: "default".to_string(),
        sandbox_wake: false,
        turn_number,
        timestamp: "2026-07-01T09:05:00.000Z".to_string(),
        created_at: Some(Utc::now()),
        external_adapter: None,
        external_source_name: None,
        external_subject_key: None,
        external_event_id: None,
        external_author: None,
        source_task_id: None,
        binding_url: None,
        binding_label: None,
        sent_by_name: None,
        file_contexts: None,
    }
}

/// `turn_number` is unused by [`Activity`] itself — an activity carries no
/// turn of its own; [`activity_anchor`] resolves one from the conversation's
/// newest turn when the window holds no message, which is all these fixtures
/// need.
fn activity(
    id: &str,
    _turn_number: Option<i32>,
    kind: ActivityKind,
    status: ActivityStatus,
) -> Activity {
    Activity {
        activity_id: id.to_string(),
        task_id: "task-1".to_string(),
        position: 1,
        ordinal: 0,
        message_uuid: None,
        parent_uuid: None,
        kind,
        status,
        tool_use_id: None,
        summary: "Bash".to_string(),
        payload: json!({}),
        detail: None,
        is_error: status == ActivityStatus::Failed,
        created_at: Utc::now(),
    }
}

fn origins(names: &HashMap<String, String>) -> OriginContext<'_> {
    OriginContext {
        frontend_url: "https://example.test",
        source_task_names: names,
    }
}

#[test]
fn lineage_none_is_its_own_root_with_no_relationship() {
    let lineage = Lineage::none("task-1");
    assert_eq!(
        lineage.to_value(),
        json!({ "parentThreadId": null, "relationshipToParent": null, "rootThreadId": "task-1" })
    );
    assert_eq!(lineage.forked_from_value(), Value::Null);
}

#[test]
fn lineage_child_fork_reports_relationship_and_forked_from() {
    let lineage = Lineage::child(
        "parent-1".to_string(),
        TaskRelationType::ForkedFrom,
        "parent-1".to_string(),
        Some(3),
    );
    assert_eq!(
        lineage.to_value(),
        json!({ "parentThreadId": "parent-1", "relationshipToParent": "fork", "rootThreadId": "parent-1" })
    );
    assert_eq!(
        lineage.forked_from_value(),
        json!({ "type": "run", "threadId": "parent-1", "runId": "parent-1:3" })
    );
    assert_eq!(lineage.created_by, "user");
    assert_eq!(lineage.creation_source, "web");
}

#[test]
fn lineage_child_created_by_reports_subagent_relationship() {
    let lineage = Lineage::child(
        "parent-1".to_string(),
        TaskRelationType::CreatedBy,
        "parent-1".to_string(),
        None,
    );
    assert_eq!(lineage.relationship, Some("subagent"));
    assert_eq!(lineage.created_by, "agent");
    assert_eq!(lineage.creation_source, "mcp");
    // A delegated child was never cut from a specific run, so it carries no
    // `forkedFrom` even though it has a parent.
    assert_eq!(lineage.forked_from_value(), Value::Null);
}

#[test]
fn shell_status_is_idle_with_no_turn() {
    let conversation = conversation("task-1");
    assert_eq!(shell_status(&conversation), "idle");
}

#[test]
fn shell_status_running_turn_reports_running() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    assert_eq!(shell_status(&conversation), "running");
}

#[test]
fn shell_status_closed_error_turn_reports_failed() {
    let mut conversation = open_turn(conversation("task-1"), 1, "error");
    conversation.turn_ended_at = Some(Utc::now());
    assert_eq!(shell_status(&conversation), "failed");
}

#[test]
fn shell_status_closed_stopped_turn_reports_interrupted() {
    let mut conversation = open_turn(conversation("task-1"), 1, "stopped");
    conversation.turn_ended_at = Some(Utc::now());
    assert_eq!(shell_status(&conversation), "interrupted");
}

#[test]
fn run_ids_report_both_ids_while_open_and_only_latest_once_closed() {
    let open = open_turn(conversation("task-1"), 2, "active");
    assert_eq!(run_ids(&open), (json!("task-1:2"), json!("task-1:2")));
    let mut closed = open_turn(conversation("task-1"), 2, "completed");
    closed.turn_ended_at = Some(Utc::now());
    assert_eq!(run_ids(&closed), (json!("task-1:2"), Value::Null));
}

#[test]
fn command_exit_code_follows_settle_status() {
    assert_eq!(
        command_exit_code(&activity(
            "a1",
            None,
            ActivityKind::ToolCall,
            ActivityStatus::InProgress
        )),
        Value::Null
    );
    assert_eq!(
        command_exit_code(&activity(
            "a1",
            None,
            ActivityKind::ToolCall,
            ActivityStatus::Completed
        )),
        json!(0)
    );
    assert_eq!(
        command_exit_code(&activity(
            "a1",
            None,
            ActivityKind::ToolCall,
            ActivityStatus::Failed
        )),
        json!(1)
    );
}

#[test]
fn turn_items_render_user_and_assistant_messages() {
    let conversation = open_turn(conversation("task-1"), 1, "completed");
    let transcript = Transcript {
        messages: vec![
            message("msg-user", Some(1), "Fix the bug", MessageType::User),
            message("msg-assistant", Some(1), "Fixed it", MessageType::Assistant),
        ],
        ..Transcript::default()
    };
    let names = HashMap::new();
    let (items, messages) = turn_items_and_messages(&conversation, &transcript, &origins(&names));
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["type"], json!("user_message"));
    assert_eq!(items[0]["messageId"], json!("msg-user"));
    assert_eq!(items[0]["runId"], json!("task-1:1"));
    assert_eq!(items[1]["type"], json!("assistant_message"));
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], json!("user"));
    assert_eq!(messages[1]["role"], json!("assistant"));
}

#[test]
fn turn_items_render_reasoning_with_the_unstripped_text() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let mut thinking = activity(
        "think-1",
        Some(1),
        ActivityKind::Thinking,
        ActivityStatus::Completed,
    );
    thinking.payload = json!({ "text": "Considering the approach", "truncated": false });
    let transcript = Transcript {
        activities: vec![thinking],
        ..Transcript::default()
    };
    let names = HashMap::new();
    let (items, _) = turn_items_and_messages(&conversation, &transcript, &origins(&names));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], json!("reasoning"));
    assert_eq!(items[0]["text"], json!("Considering the approach"));
}

#[test]
fn turn_items_classify_bash_calls_as_command_execution_with_a_derived_exit_code() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let mut call = activity(
        "call-1",
        Some(1),
        ActivityKind::ToolCall,
        ActivityStatus::Completed,
    );
    call.summary = "Bash".to_string();
    call.payload = json!({ "input": { "command": "ls -la" }, "result": "total 0" });
    let transcript = Transcript {
        activities: vec![call],
        ..Transcript::default()
    };
    let names = HashMap::new();
    let (items, _) = turn_items_and_messages(&conversation, &transcript, &origins(&names));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], json!("command_execution"));
    assert_eq!(items[0]["input"], json!("ls -la"));
    assert_eq!(items[0]["output"], json!("total 0"));
    assert_eq!(items[0]["exitCode"], json!(0));
}

fn tool_call(id: &str, tool: &str, input: Value, result: Option<&str>) -> Activity {
    let mut call = activity(
        id,
        Some(1),
        ActivityKind::ToolCall,
        ActivityStatus::Completed,
    );
    call.summary = tool.to_string();
    call.payload = json!({ "input": input, "result": result });
    call
}

fn items_for(activities: Vec<Activity>) -> Vec<Value> {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        activities,
        ..Transcript::default()
    };
    let names = HashMap::new();
    turn_items_and_messages(&conversation, &transcript, &origins(&names)).0
}

#[test]
fn tool_calls_render_as_the_item_their_kind_names() {
    let items = items_for(vec![
        tool_call(
            "edit-1",
            "Edit",
            json!({ "file_path": "src/a.rs", "old_string": "a", "new_string": "b" }),
            Some("ok"),
        ),
        tool_call(
            "codex-edit-1",
            "codex.edit",
            json!({ "changes": [{ "path": "/w/b.rs", "kind": "add", "diff": "+b\n" }] }),
            None,
        ),
        tool_call(
            "agent-1",
            "Task",
            json!({ "description": "Look around", "prompt": "Find the bug" }),
            Some("Found it"),
        ),
        tool_call("search-1", "WebSearch", json!({ "query": "rust" }), None),
        tool_call(
            "read-1",
            "Read",
            json!({ "file_path": "src/a.rs" }),
            Some("text"),
        ),
        tool_call(
            "shell-1",
            "codex.shell",
            json!({ "command": "ls" }),
            Some("a"),
        ),
    ]);
    let types: Vec<&str> = items
        .iter()
        .map(|item| item["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "file_change",
            "file_change",
            "subagent",
            "web_search",
            "dynamic_tool",
            "command_execution"
        ]
    );
    assert_eq!(items[0]["fileName"], json!("src/a.rs"));
    assert_eq!(items[0]["oldStr"], json!("a"));
    assert_eq!(items[0]["newStr"], json!("b"));
    assert_eq!(items[1]["fileName"], json!("/w/b.rs"));
    assert_eq!(items[1]["changes"][0]["operation"], json!("add"));
    assert_eq!(items[1]["diffStr"], json!("+b\n"));
    assert_eq!(items[2]["prompt"], json!("Find the bug"));
    assert_eq!(items[2]["title"], json!("Look around"));
    assert_eq!(items[2]["result"], json!("Found it"));
    assert_eq!(items[3]["patterns"], json!(["rust"]));
    assert_eq!(items[4]["toolName"], json!("Read"));
    assert_eq!(items[5]["input"], json!("ls"));
}

#[test]
fn a_question_or_todo_call_renders_through_its_own_row() {
    let mut plan = activity(
        "todo-1:plan",
        Some(1),
        ActivityKind::PlanUpdated,
        ActivityStatus::Completed,
    );
    plan.payload = json!({ "plan": [
        { "step": "Read", "status": "completed" },
        { "step": "Write", "status": "inProgress" }
    ] });
    let items = items_for(vec![
        tool_call("question-1", "AskUserQuestion", json!({}), None),
        tool_call("todo-1", "TodoWrite", json!({ "todos": [] }), None),
        plan,
    ]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], json!("todo_list"));
    assert_eq!(items[0]["planId"], json!("todo-1:plan"));
    assert_eq!(
        items[0]["steps"][1],
        json!({ "id": "2", "text": "Write", "status": "running" })
    );
}

#[test]
fn a_proposed_plan_is_an_item_and_an_artifact_active_until_answered() {
    let mut call = tool_call("plan-1", "ExitPlanMode", json!({ "plan": "# Do it" }), None);
    call.status = ActivityStatus::InProgress;
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        activities: vec![call.clone()],
        ..Transcript::default()
    };
    let names = HashMap::new();
    let (items, _) = turn_items_and_messages(&conversation, &transcript, &origins(&names));
    assert_eq!(items[0]["type"], json!("proposed_plan"));
    assert_eq!(items[0]["markdown"], json!("# Do it"));
    let artifacts = plans(&conversation, &transcript);
    assert_eq!(artifacts[0]["kind"], json!("proposed_plan"));
    assert_eq!(artifacts[0]["status"], json!("active"));

    call.status = ActivityStatus::Completed;
    let answered = Transcript {
        activities: vec![call],
        ..Transcript::default()
    };
    assert_eq!(
        plans(&conversation, &answered)[0]["status"],
        json!("completed")
    );
}

#[test]
fn a_running_turn_offers_steering_through_its_session_and_provider_turn() {
    let conversation = open_turn(conversation("task-1"), 2, "active");
    let names = HashMap::new();
    let projection = thread_projection(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &Transcript::default(),
        &origins(&names),
        &Lineage::none("task-1"),
    );
    let run = projection["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["status"] == json!("running"))
        .expect("a running run");
    let attempt = run["activeAttemptId"].as_str().unwrap();
    let thread_id = projection["thread"]["activeProviderThreadId"].clone();
    assert_eq!(run["providerThreadId"], thread_id);
    let provider_thread = &projection["providerThreads"][0];
    assert_eq!(provider_thread["id"], thread_id);
    let session = &projection["providerSessions"][0];
    assert_eq!(provider_thread["providerSessionId"], session["id"]);
    assert_eq!(
        session["capabilities"]["turns"]["supportsActiveSteering"],
        json!(true)
    );
    assert!(
        projection["providerTurns"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |turn| turn["runAttemptId"] == json!(attempt) && turn["status"] == json!("running")
            )
    );
    assert!(
        projection["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == json!(attempt))
    );
}

#[test]
fn fork_turn_item_is_none_without_a_fork_relationship() {
    let conversation = conversation("task-1");
    assert!(fork_turn_item(&conversation, &Lineage::none("task-1")).is_none());
}

#[test]
fn fork_turn_item_names_the_source_run_for_a_fork() {
    let conversation = conversation("task-1");
    let lineage = Lineage::child(
        "parent-1".to_string(),
        TaskRelationType::ForkedFrom,
        "parent-1".to_string(),
        Some(2),
    );
    let item = fork_turn_item(&conversation, &lineage).expect("a fork reports a fork item");
    assert_eq!(item["type"], json!("fork"));
    assert_eq!(
        item["source"],
        json!({ "type": "run", "threadId": "parent-1", "runId": "parent-1:2" })
    );
    assert_eq!(item["targetThreadId"], json!("task-1"));
}

#[test]
fn runs_reports_one_run_per_historical_turn_plus_the_current_one() {
    let conversation = open_turn(conversation("task-1"), 2, "active");
    let transcript = Transcript {
        messages: vec![message("msg-1", Some(1), "first turn", MessageType::User)],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    let ids: Vec<&str> = runs.iter().map(|run| run["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["task-1:1", "task-1:2"]);
    assert_eq!(runs[0]["status"], json!("completed"));
    assert_eq!(runs[1]["status"], json!("running"));
}

#[test]
fn thread_projection_prepends_the_fork_item_and_renumbers_the_rest() {
    let conversation = open_turn(conversation("task-1"), 1, "completed");
    let transcript = Transcript {
        messages: vec![message("msg-1", Some(1), "hello", MessageType::User)],
        ..Transcript::default()
    };
    let lineage = Lineage::child(
        "parent-1".to_string(),
        TaskRelationType::ForkedFrom,
        "parent-1".to_string(),
        Some(1),
    );
    let projection = thread_projection(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &transcript,
        &origins(&HashMap::new()),
        &lineage,
    );
    let items = projection["turnItems"].as_array().expect("turnItems array");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["type"], json!("fork"));
    assert_eq!(items[0]["ordinal"], json!(0));
    assert_eq!(items[1]["ordinal"], json!(1));
    assert_eq!(
        projection["thread"]["lineage"]["relationshipToParent"],
        json!("fork")
    );
    let visible = projection["visibleTurnItems"]
        .as_array()
        .expect("visibleTurnItems");
    assert_eq!(visible.len(), 2);
    assert_eq!(visible[0]["sourceThreadId"], json!("task-1"));
}

fn asked(id: &str, request_id: &str) -> Activity {
    let mut activity = activity(
        id,
        None,
        ActivityKind::UserInputRequested,
        ActivityStatus::Completed,
    );
    activity.payload = json!({ "requestId": request_id, "questions": [{ "id": "q1" }] });
    activity
}

fn answered(id: &str, request_id: &str) -> Activity {
    let mut activity = activity(
        id,
        None,
        ActivityKind::UserInputResolved,
        ActivityStatus::Completed,
    );
    activity.payload = json!({ "requestId": request_id, "answers": { "q1": "yes" } });
    activity
}

#[test]
fn queued_run_ids_round_trip_and_never_parse_as_a_turn() {
    let id = queued_run_id("task-1", "msg-9");
    assert_eq!(queued_message_uuid(&id), Some("msg-9"));
    assert_eq!(turn_number_of_run(&id), None);
    assert_eq!(turn_number_of_run("task-1:3"), Some(3));
    assert_eq!(turn_number_of_run("task-1:0"), None);
    assert_eq!(queued_message_uuid("task-1:3"), None);
}

#[test]
fn runs_append_one_queued_run_per_message_waiting_for_a_turn() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            message("msg-2", None, "next", MessageType::User),
            message("msg-3", None, "after", MessageType::User),
        ],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1]["id"], json!("task-1:queued:msg-2"));
    assert_eq!(runs[1]["status"], json!("queued"));
    assert_eq!(runs[1]["queuePosition"], json!(1));
    assert_eq!(runs[1]["ordinal"], json!(2));
    assert_eq!(runs[2]["queuePosition"], json!(2));
}

#[test]
fn runtime_request_is_pending_while_its_turn_is_open() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        activities: vec![asked("act-1", "req-1")],
        ..Transcript::default()
    };
    let requests = runtime_requests(&conversation, &transcript);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["id"], json!("req-1"));
    assert_eq!(requests[0]["kind"], json!("user_input"));
    assert_eq!(requests[0]["status"], json!("pending"));
    assert_eq!(requests[0]["resolvedAt"], Value::Null);
}

#[test]
fn runtime_request_is_resolved_with_its_answers_once_answered() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        activities: vec![asked("act-1", "req-1"), answered("act-2", "req-1")],
        ..Transcript::default()
    };
    let requests = runtime_requests(&conversation, &transcript);
    assert_eq!(requests[0]["status"], json!("resolved"));
    assert_eq!(requests[0]["answers"], json!({ "q1": "yes" }));
}

#[test]
fn runtime_request_expires_when_its_turn_closed_unanswered() {
    let conversation = open_turn(conversation("task-1"), 1, "completed");
    let transcript = Transcript {
        activities: vec![asked("act-1", "req-1")],
        ..Transcript::default()
    };
    assert_eq!(
        runtime_requests(&conversation, &transcript)[0]["status"],
        json!("expired")
    );
}

#[test]
fn turn_items_render_a_pending_question_as_a_waiting_user_input_request() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        activities: vec![asked("act-1", "req-1")],
        ..Transcript::default()
    };
    let (items, _) = turn_items_and_messages(&conversation, &transcript, &origins(&HashMap::new()));
    let item = items
        .iter()
        .find(|item| item["type"] == json!("user_input_request"))
        .expect("user_input_request item");
    assert_eq!(item["requestId"], json!("req-1"));
    assert_eq!(item["status"], json!("waiting"));
}

#[test]
fn thread_updated_moves_a_closed_task_to_the_archive() {
    let mut closed = conversation("task-1");
    closed.closed_at = Some(Utc::now());
    let defaults = AgentModelDefaults::default();
    let item = thread_updated_v2(&closed, "ws-a", &defaults, 2048);
    assert_eq!(item["kind"], json!("thread.updated"));
    assert_eq!(item["sequence"], json!(2048));
    assert_eq!(item["location"], json!("archive"));
    let open = thread_updated_v2(&conversation("task-1"), "ws-a", &defaults, 2048);
    assert_eq!(open["location"], json!("active"));
    let removed = thread_removed_v2("task-1", 3072);
    assert_eq!(removed["kind"], json!("thread.removed"));
    assert_eq!(removed["threadId"], json!("task-1"));
}

#[test]
fn turn_items_interleave_messages_and_activities_by_time() {
    let conversation = open_turn(conversation("task-1"), 2, "active");
    let mut first = message("msg-1", Some(1), "first", MessageType::User);
    first.timestamp = "2026-07-01T09:00:00.000Z".to_string();
    let mut last = message("msg-2", Some(2), "second", MessageType::User);
    last.timestamp = "2026-07-01T09:10:00.000Z".to_string();
    first.created_at = Some(instant_of("2026-07-01T09:00:00Z"));
    last.created_at = Some(instant_of("2026-07-01T09:10:00Z"));
    let mut thought = activity(
        "act-1",
        None,
        ActivityKind::Thinking,
        ActivityStatus::Completed,
    );
    thought.payload = json!({ "text": "pondering" });
    thought.created_at = instant_of("2026-07-01T09:05:00Z");
    let transcript = Transcript {
        messages: vec![first, last],
        activities: vec![thought],
        ..Transcript::default()
    };
    let (items, _) = turn_items_and_messages(&conversation, &transcript, &origins(&HashMap::new()));
    let kinds: Vec<&str> = items
        .iter()
        .map(|item| item["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["user_message", "reasoning", "user_message"]);
    let ordinals: Vec<i64> = items
        .iter()
        .map(|item| item["ordinal"].as_i64().unwrap())
        .collect();
    assert_eq!(ordinals, vec![0, 1, 2]);
}

#[test]
fn a_resolved_request_reports_when_it_was_answered() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let mut answer = answered("act-2", "req-1");
    answer.created_at = instant_of("2026-07-01T09:30:00Z");
    let transcript = Transcript {
        activities: vec![asked("act-1", "req-1"), answer],
        ..Transcript::default()
    };
    assert_eq!(
        runtime_requests(&conversation, &transcript)[0]["resolvedAt"],
        json!("2026-07-01T09:30:00.000Z")
    );
}

fn instant_of(value: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn every_node_a_row_names_is_in_the_projection() {
    let conversation = open_turn(conversation("task-1"), 2, "active");
    let mut call = tool_call("plan-1", "ExitPlanMode", json!({ "plan": "# Do it" }), None);
    call.status = ActivityStatus::InProgress;
    let transcript = Transcript {
        activities: vec![call],
        ..Transcript::default()
    };
    let names = HashMap::new();
    let projection = thread_projection(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &transcript,
        &origins(&names),
        &Lineage::none("task-1"),
    );
    let rows = |key: &str| projection[key].as_array().unwrap().clone();
    let nodes: Vec<Value> = rows("nodes")
        .iter()
        .map(|node| node["id"].clone())
        .collect();
    let mut named: Vec<Value> = rows("runs")
        .iter()
        .map(|run| run["rootNodeId"].clone())
        .filter(|id| !id.is_null())
        .collect();
    named.extend(rows("attempts").iter().map(|row| row["rootNodeId"].clone()));
    named.extend(
        rows("providerTurns")
            .iter()
            .map(|row| row["nodeId"].clone()),
    );
    named.extend(rows("plans").iter().map(|row| row["nodeId"].clone()));
    named.extend(rows("nodes").iter().map(|row| row["rootNodeId"].clone()));
    assert!(!rows("plans").is_empty());
    for id in named {
        assert!(nodes.contains(&id), "{id} names no node");
    }
    let run = rows("runs")
        .into_iter()
        .find(|run| run["status"] == json!("running"))
        .expect("a running run");
    let attempt = rows("attempts")
        .into_iter()
        .find(|row| row["id"] == run["activeAttemptId"])
        .expect("the run's attempt");
    assert_eq!(attempt["rootNodeId"], run["rootNodeId"]);
}
