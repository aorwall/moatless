use super::*;
use crate::projection::v2_items::command_exit_code;
use crate::projection::v2_items::plans;
use crate::transcript::TranscriptEntry;
use chrono::Utc;
use moatless_tasks::task_listing::RunningChild;
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
        turn_error_reason: None,
        turn_error_resets_at: None,
        child_count: 0,
        running_children: Vec::new(),
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
        parent_task_id: None,
        parent_relation: None,
        fork_at_turn: None,
        settled_override: None,
        settled_at: None,
        snoozed_until: None,
        snoozed_at: None,
        last_read_at: None,
        pinned_at: None,
        pin_order_key: None,
        active_order_key: None,
        auto_settle_disabled_at: None,
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
        entry: TranscriptEntry {
            id: 1,
            uuid: uuid.to_string(),
            message_type: kind,
            message_role: MessageRole::User,
            message_text: Some(text.to_string()),
            user_type: "default".to_string(),
            sandbox_wake: false,
            held: false,
            queue_order: None,
            turn_number,
            timestamp: "2026-07-01T09:05:00.000Z".to_string(),
            created_at: Some(Utc::now()),
            external_adapter: None,
            external_source_name: None,
            external_subject_key: None,
            external_event_id: None,
            external_author: None,
            source_task_id: None,
            file_contexts: None,
            child_outcome: None,
            sent_by: None,
        },
        binding_url: None,
        binding_label: None,
        loop_name: None,
        sent_by_name: None,
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
    let lineage = Lineage::root("task-1");
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
fn shell_status_closed_completed_turn_reports_completed() {
    let mut conversation = open_turn(conversation("task-1"), 1, "completed");
    conversation.turn_ended_at = Some(Utc::now());
    assert_eq!(shell_status(&conversation), "completed");
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
        &Lineage::root("task-1"),
        &[],
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
    assert!(fork_turn_item(&conversation, &Lineage::root("task-1")).is_none());
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

fn rewound(turn_number: i32) -> RewoundTurn {
    RewoundTurn {
        turn_number,
        started_at: Utc::now() - chrono::Duration::minutes(5),
        rewound_at: Utc::now(),
    }
}

#[test]
fn a_rewound_turn_is_a_rolled_back_run_between_the_live_ones() {
    let conversation = open_turn(conversation("task-1"), 4, "active");
    let transcript = Transcript {
        messages: vec![message("msg-1", Some(1), "first turn", MessageType::User)],
        rewound_turns: vec![rewound(2), rewound(3)],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    let shape: Vec<(&str, &str)> = runs
        .iter()
        .map(|run| (run["id"].as_str().unwrap(), run["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("task-1:1", "completed"),
            ("task-1:2", "rolled_back"),
            ("task-1:3", "rolled_back"),
            ("task-1:4", "running"),
        ]
    );
    assert_eq!(
        runs[1]["completedAt"],
        json!(instant(transcript.rewound_turns[0].rewound_at))
    );

    let nodes = root_nodes(&runs);
    assert_eq!(nodes[1]["status"], json!("rolled_back"));
    assert_eq!(attempts(&runs)[1]["status"], json!("completed"));
    let checkpoints = checkpoints(&conversation, &runs);
    let ordinals: Vec<&Value> = checkpoints.iter().map(|c| &c["appRunOrdinal"]).collect();
    assert_eq!(ordinals, vec![&Value::Null, &json!(1)]);
}

#[test]
fn after_a_rewind_the_agent_conversation_ends_at_the_cut_and_a_send_queues_above_it() {
    let conversation = awake(open_turn(conversation("task-1"), 1, "completed"));
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first turn", MessageType::User),
            held(message("msg-2", None, "again", MessageType::User)),
        ],
        rewound_turns: vec![rewound(2)],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    assert_eq!(runs[2]["id"], json!("task-1:queued:msg-2"));
    assert_eq!(runs[2]["ordinal"], json!(3));
    let thread = provider_thread(&conversation, &runs, "2026-01-01T00:00:00.000Z");
    assert_eq!(thread["lastRunOrdinal"], json!(1));
}

#[test]
fn a_rewound_turn_a_live_one_reuses_is_not_reported_twice() {
    let conversation = open_turn(conversation("task-1"), 2, "completed");
    let transcript = Transcript {
        rewound_turns: vec![rewound(2)],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["status"], json!("completed"));
}

#[test]
fn checkpoints_mark_the_start_and_each_settled_run_but_not_the_open_one() {
    let conversation = open_turn(conversation("task-1"), 2, "active");
    let transcript = Transcript {
        messages: vec![message("msg-1", Some(1), "first turn", MessageType::User)],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    let checkpoints = checkpoints(&conversation, &runs);
    let ordinals: Vec<&Value> = checkpoints.iter().map(|c| &c["appRunOrdinal"]).collect();
    assert_eq!(ordinals, vec![&Value::Null, &json!(1)]);
    assert_eq!(checkpoints[0]["ordinalWithinScope"], json!(0));
    assert_eq!(checkpoints[1]["runId"], json!("task-1:1"));
    assert_eq!(checkpoints[1]["status"], json!("ready"));
    assert_eq!(checkpoints[1]["parentCheckpointId"], checkpoints[0]["id"]);
    let id = checkpoints[1]["id"].as_str().unwrap();
    assert_eq!(turn_count_of_checkpoint(id), Some(1));
    assert_eq!(
        turn_count_of_checkpoint(checkpoints[0]["id"].as_str().unwrap()),
        Some(0)
    );
}

#[test]
fn a_pending_question_is_a_user_input_request_on_the_shell() {
    let mut conversation = conversation("task-1");
    let shell = thread_shell_v2(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &Lineage::root("task-1"),
    );
    assert_eq!(shell["pendingRuntimeRequest"], Value::Null);
    conversation.pending_question = true;
    let shell = thread_shell_v2(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &Lineage::root("task-1"),
    );
    assert_eq!(shell["pendingRuntimeRequest"]["kind"], json!("user_input"));
}

fn shell(conversation: &Conversation) -> Value {
    thread_shell_v2(
        conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &Lineage::root(&conversation.task_id),
    )
}

#[test]
fn a_working_shell_says_when_its_work_started() {
    let started = Utc::now() - chrono::Duration::minutes(3);
    let mut open = open_turn(conversation("task-1"), 2, "active");
    open.turn_started_at = Some(started);
    let row = shell(&open);
    assert_eq!(row["activityRunStartedAt"], json!(instant(started)));
    assert_eq!(row["latestRunStartedAt"], json!(instant(started)));
    assert_eq!(row["latestRunCompletedAt"], Value::Null);

    let mut owed = awake(open_turn(conversation("task-1"), 1, "completed"));
    let ended = Utc::now() - chrono::Duration::minutes(1);
    let sent = Utc::now();
    owed.turn_ended_at = Some(ended);
    owed.latest_queued_message_at = Some(sent);
    let row = shell(&owed);
    assert_eq!(row["status"], json!("starting"));
    assert_eq!(row["activityRunStartedAt"], json!(instant(sent)));
    assert_eq!(row["latestRunCompletedAt"], json!(instant(ended)));
}

#[test]
fn a_settled_or_unrun_shell_has_no_activity_start() {
    let mut settled = open_turn(conversation("task-1"), 1, "completed");
    settled.turn_ended_at = Some(Utc::now());
    assert_eq!(shell(&settled)["activityRunStartedAt"], Value::Null);
    let never = shell(&conversation("task-1"));
    assert_eq!(never["activityRunStartedAt"], Value::Null);
    assert_eq!(never["latestRunStartedAt"], Value::Null);
    assert_eq!(never["latestRunCompletedAt"], Value::Null);
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
        &[],
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

fn held(mut message: TranscriptMessage) -> TranscriptMessage {
    message.held = true;
    message
}

fn awake(mut conversation: Conversation) -> Conversation {
    conversation.sandbox_desired_state_raw = Some(
        moatless_types::sandbox::SandboxDesiredState::Running
            .as_str()
            .to_string(),
    );
    conversation.sandbox_started_at = Some(Utc::now());
    conversation
}

fn timeline_runs(conversation: &Conversation, transcript: &Transcript) -> Vec<(String, Value)> {
    let names = HashMap::new();
    let (items, _) = turn_items_and_messages(conversation, transcript, &origins(&names));
    items
        .iter()
        .filter(|item| item["type"] == json!("user_message"))
        .map(|item| {
            (
                item["messageId"].as_str().unwrap().to_string(),
                item["runId"].clone(),
            )
        })
        .collect()
}

#[test]
fn a_held_message_is_a_queued_run_and_stays_out_of_the_timeline() {
    let conversation = awake(open_turn(conversation("task-1"), 1, "active"));
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            held(message("msg-2", None, "next", MessageType::User)),
            held(message("msg-3", None, "after", MessageType::User)),
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
    assert_eq!(
        timeline_runs(&conversation, &transcript),
        vec![("msg-1".to_string(), json!("task-1:1"))]
    );
}

#[test]
fn a_moved_queued_message_runs_before_the_ones_nobody_moved() {
    let conversation = awake(open_turn(conversation("task-1"), 1, "active"));
    let moved = |mut message: TranscriptMessage, order: i32| {
        message.queue_order = Some(order);
        message
    };
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            held(message("msg-2", None, "arrived first", MessageType::User)),
            moved(held(message("msg-3", None, "moved", MessageType::User)), 1),
            held(message("msg-4", None, "arrived last", MessageType::User)),
        ],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    let queued: Vec<&Value> = runs[1..].iter().map(|run| &run["userMessageId"]).collect();
    assert_eq!(queued, [&json!("msg-3"), &json!("msg-2"), &json!("msg-4")]);
    assert!(runs[1..].iter().all(|run| run["queueHeld"] == json!(false)));
}

/// A stop leaves what was queued waiting, so the client offers Resume.
#[test]
fn a_stopped_turn_holds_the_queue() {
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            held(message("msg-2", None, "next", MessageType::User)),
        ],
        ..Transcript::default()
    };
    for (status, expected) in [("stopped", true), ("completed", false), ("error", false)] {
        let conversation = open_turn(conversation("task-1"), 1, status);
        let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
        assert_eq!(runs[1]["queueHeld"], json!(expected), "{status}");
    }
}

#[test]
fn a_send_during_an_open_turn_steers_into_that_turn() {
    let conversation = awake(open_turn(conversation("task-1"), 1, "active"));
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            message("msg-2", None, "also this", MessageType::User),
        ],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["status"], json!("running"));
    assert_eq!(
        timeline_runs(&conversation, &transcript)[1],
        ("msg-2".to_string(), json!("task-1:1"))
    );
}

#[test]
fn a_send_whose_turn_is_owed_is_a_starting_run_on_the_timeline() {
    let mut conversation = awake(open_turn(conversation("task-1"), 1, "completed"));
    conversation.turn_ended_at = Some(Utc::now() - chrono::Duration::minutes(1));
    conversation.latest_queued_message_at = Some(Utc::now());
    let transcript = Transcript {
        messages: vec![
            message("msg-1", Some(1), "first", MessageType::User),
            message("msg-2", None, "next", MessageType::User),
        ],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1]["id"], json!("task-1:starting:msg-2"));
    assert_eq!(runs[1]["status"], json!("starting"));
    assert_eq!(runs[1]["userMessageId"], json!("msg-2"));
    assert_eq!(turn_number_of_run("task-1:starting:msg-2"), None);
    assert_eq!(
        timeline_runs(&conversation, &transcript)[1],
        ("msg-2".to_string(), json!("task-1:starting:msg-2"))
    );
    assert_eq!(shell_status(&conversation), "starting");
    // No attempt or provider turn until the Sandbox opens the turn.
    assert_eq!(attempts(&runs).len(), 1);
    assert_eq!(provider_turns(&runs, &HashMap::new()).len(), 1);
}

#[test]
fn a_first_send_is_a_starting_run_and_a_starting_shell() {
    let mut conversation = awake(conversation("task-1"));
    conversation.latest_queued_message_at = Some(Utc::now());
    let transcript = Transcript {
        messages: vec![message("msg-1", None, "hello", MessageType::User)],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["status"], json!("starting"));
    assert_eq!(runs[0]["ordinal"], json!(1));
    assert_eq!(shell_status(&conversation), "starting");
}

#[test]
fn a_send_to_a_stopped_agent_waits_as_a_queued_run() {
    let mut conversation = open_turn(conversation("task-1"), 1, "completed");
    conversation.sandbox_desired_state_raw = Some(
        moatless_types::sandbox::SandboxDesiredState::Stopped
            .as_str()
            .to_string(),
    );
    let transcript = Transcript {
        messages: vec![message("msg-2", None, "next", MessageType::User)],
        ..Transcript::default()
    };
    let runs = runs(&conversation, &transcript, &AgentModelDefaults::default());
    assert_eq!(runs[1]["status"], json!("queued"));
    assert!(timeline_runs(&conversation, &transcript).is_empty());
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
fn a_notice_queued_before_the_question_leaves_it_pending_in_its_turn() {
    let conversation = open_turn(conversation("task-1"), 18, "active");
    let start = Utc::now() - chrono::Duration::seconds(30);
    let mut sent = message("msg-user", Some(18), "Seed both envs", MessageType::User);
    sent.created_at = Some(start);
    let mut notice = held(message(
        "msg-notice",
        None,
        "Child task completed turn 1.",
        MessageType::User,
    ));
    notice.child_outcome = Some("completed".to_string());
    notice.created_at = Some(start + chrono::Duration::seconds(8));
    let mut question = asked("act-1", "req-1");
    question.created_at = start + chrono::Duration::seconds(14);
    let transcript = Transcript {
        messages: vec![sent, notice],
        activities: vec![question],
        ..Transcript::default()
    };

    assert_eq!(
        runtime_requests(&conversation, &transcript)[0]["status"],
        json!("pending")
    );
    let (items, _) = turn_items_and_messages(&conversation, &transcript, &origins(&HashMap::new()));
    let item = items
        .iter()
        .find(|item| item["type"] == json!("user_input_request"))
        .expect("user_input_request item");
    assert_eq!(item["status"], json!("waiting"));
    assert_eq!(item["runId"], json!("task-1:18"));
}

fn turn_started(turn_number: i32, started_at: chrono::DateTime<Utc>) -> TurnOutcome {
    TurnOutcome {
        turn_number,
        status: "completed".to_string(),
        started_at: Some(started_at),
        ended_at: None,
        error: None,
        reason: None,
        resets_at: None,
    }
}

#[test]
fn a_notice_a_later_turn_took_does_not_pull_the_question_into_it() {
    let conversation = open_turn(conversation("task-1"), 19, "active");
    let start = Utc::now() - chrono::Duration::seconds(120);
    let mut sent = message("msg-user", Some(18), "Seed both envs", MessageType::User);
    sent.created_at = Some(start);
    let mut notice = message(
        "msg-notice",
        Some(19),
        "Child task completed turn 1.",
        MessageType::User,
    );
    notice.child_outcome = Some("completed".to_string());
    notice.created_at = Some(start + chrono::Duration::seconds(8));
    let mut question = asked("act-1", "req-1");
    question.created_at = start + chrono::Duration::seconds(14);
    let transcript = Transcript {
        messages: vec![sent, notice],
        activities: vec![question],
        turn_outcomes: vec![
            turn_started(18, start),
            turn_started(19, start + chrono::Duration::seconds(60)),
        ],
        ..Transcript::default()
    };

    let (items, _) = turn_items_and_messages(&conversation, &transcript, &origins(&HashMap::new()));
    let item = items
        .iter()
        .find(|item| item["type"] == json!("user_input_request"))
        .expect("user_input_request item");
    assert_eq!(item["runId"], json!("task-1:18"));
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
fn a_question_whose_call_came_back_unanswered_is_not_pending() {
    let conversation = open_turn(conversation("task-1"), 1, "active");
    let transcript = Transcript {
        activities: vec![asked("act-1", "req-1"), asked("act-2", "req-2")],
        settled_requests: ["req-1".to_string()].into(),
        ..Transcript::default()
    };
    let requests = runtime_requests(&conversation, &transcript);
    assert_eq!(requests[0]["status"], json!("expired"));
    assert_eq!(requests[1]["status"], json!("pending"));
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
        &Lineage::root("task-1"),
        &[],
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

fn listed_child(task_id: &str, parent: &str, relation: &str) -> Conversation {
    let mut child = conversation(task_id);
    child.parent_task_id = Some(parent.to_string());
    child.parent_relation = Some(relation.to_string());
    child
}

#[test]
fn a_listed_subagent_and_fork_carry_their_parent_and_the_listed_root() {
    let subagent = listed_child("child", "parent", "created_by");
    let mut fork = listed_child("fork", "child", "forked_from");
    fork.fork_at_turn = Some(3);
    let parents = HashMap::from([("child", "parent"), ("fork", "child")]);

    let lineage = Lineage::listed(&subagent, &parents).to_value();
    assert_eq!(lineage["parentThreadId"], json!("parent"));
    assert_eq!(lineage["relationshipToParent"], json!("subagent"));
    assert_eq!(lineage["rootThreadId"], json!("parent"));

    let fork_lineage = Lineage::listed(&fork, &parents);
    assert_eq!(
        fork_lineage.to_value()["relationshipToParent"],
        json!("fork")
    );
    assert_eq!(fork_lineage.to_value()["rootThreadId"], json!("parent"));
    assert_eq!(
        fork_lineage.forked_from_value(),
        json!({ "type": "run", "threadId": "child", "runId": "child:3" })
    );
}

#[test]
fn a_listed_row_without_a_known_parent_edge_is_its_own_root() {
    let parents = HashMap::new();
    let lineage = Lineage::listed(&conversation("solo"), &parents).to_value();
    assert_eq!(lineage["parentThreadId"], Value::Null);
    assert_eq!(lineage["rootThreadId"], json!("solo"));
    let unknown = listed_child("odd", "parent", "related_to");
    assert_eq!(
        Lineage::listed(&unknown, &parents).to_value()["relationshipToParent"],
        json!("subagent"),
        "any parent edge the listing returns is a child edge"
    );
}

#[test]
fn a_parent_cycle_in_the_listing_stops_the_root_walk() {
    let looped = listed_child("a", "b", "forked_from");
    let parents = HashMap::from([("a", "b"), ("b", "a")]);
    assert_eq!(
        Lineage::listed(&looped, &parents).to_value()["rootThreadId"],
        json!("b")
    );
}

fn ended_turn(status: &str) -> Conversation {
    let mut conversation = open_turn(conversation("task-1"), 1, status);
    conversation.turn_ended_at = Some(Utc::now());
    conversation
}

#[test]
fn background_work_is_reported_only_once_the_turn_has_ended() {
    let mut running = open_turn(conversation("task-1"), 1, "active");
    running.running_commands = 1;
    running.running_children = vec![RunningChild {
        task_id: "child-1".to_string(),
        name: Some("Write tests".to_string()),
    }];
    assert_eq!(shell(&running)["pendingBackgroundTasks"], json!([]));

    let mut ended = ended_turn("completed");
    ended.running_commands = running.running_commands;
    ended.running_children = running.running_children.clone();
    assert_eq!(
        shell(&ended)["pendingBackgroundTasks"],
        json!([
            {
                "taskId": "task-1:command:1",
                "kind": "background_task",
                "description": "Background command"
            },
            {
                "taskId": "child-1",
                "kind": "subagent",
                "childThreadId": "child-1",
                "description": "Write tests"
            }
        ])
    );
}

fn delegated(task_id: &str, created: chrono::DateTime<Utc>) -> Conversation {
    let mut child = conversation(task_id);
    child.name = Some(format!("Delegated {task_id}"));
    child.created_at_db = Some(created);
    child.parent_task_id = Some("task-1".to_string());
    child.parent_relation = Some("created_by".to_string());
    child
}

fn delegating_projection(transcript: &Transcript, children: &[Conversation]) -> Value {
    let mut parent = open_turn(conversation("task-1"), 1, "active");
    parent.created_at_db = Some(Utc::now() - chrono::Duration::minutes(10));
    thread_projection(
        &parent,
        "ws-a",
        &AgentModelDefaults::default(),
        transcript,
        &origins(&HashMap::new()),
        &Lineage::root("task-1"),
        children,
    )
}

#[test]
fn a_created_by_child_appears_in_subagents_under_the_run_that_spawned_it() {
    let mut running = delegated("child-run", Utc::now() - chrono::Duration::minutes(2));
    running = open_turn(running, 1, "active");
    let mut asking = open_turn(
        delegated("child-ask", Utc::now() - chrono::Duration::minutes(1)),
        1,
        "active",
    );
    asking.pending_question = true;
    let mut failed = open_turn(
        delegated("child-failed", Utc::now() - chrono::Duration::minutes(3)),
        1,
        "error",
    );
    failed.turn_error = Some("boom".to_string());
    failed.turn_ended_at = Some(Utc::now());

    let projection = delegating_projection(&Transcript::default(), &[running, asking, failed]);

    let subagents = projection["subagents"].as_array().expect("subagents");
    let children: Vec<&str> = subagents
        .iter()
        .map(|subagent| subagent["childThreadId"].as_str().unwrap())
        .collect();
    assert_eq!(children, ["child-failed", "child-run", "child-ask"]);
    let statuses: Vec<&str> = subagents
        .iter()
        .map(|subagent| subagent["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["failed", "running", "waiting"]);
    for subagent in subagents {
        assert_eq!(subagent["origin"], json!("app_owned"));
        assert_eq!(subagent["createdBy"], json!("agent"));
        assert_eq!(subagent["runId"], json!("task-1:1"));
        assert_eq!(subagent["parentNodeId"], json!(root_node_id("task-1:1")));
        assert!(subagent["startedAt"].is_string());
    }
    assert_eq!(subagents[0]["result"], json!("boom"));
    assert!(subagents[0]["completedAt"].is_string());
    assert_eq!(subagents[1]["completedAt"], Value::Null);
}

#[test]
fn each_delegated_task_is_a_thread_created_item_placed_by_its_creation() {
    let child = delegated("child-1", Utc::now() - chrono::Duration::minutes(5));
    let mut early = message("msg-1", Some(1), "start", MessageType::User);
    early.timestamp = instant(Utc::now() - chrono::Duration::minutes(8));
    let mut late = message("msg-2", Some(1), "done", MessageType::Assistant);
    late.timestamp = instant(Utc::now() - chrono::Duration::minutes(1));
    let transcript = Transcript {
        messages: vec![early, late],
        ..Transcript::default()
    };

    let projection = delegating_projection(&transcript, &[child]);

    let items = projection["turnItems"].as_array().expect("turnItems");
    let kinds: Vec<&str> = items
        .iter()
        .map(|item| item["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["user_message", "thread_created", "assistant_message"]
    );
    let ordinals: Vec<i64> = items
        .iter()
        .map(|item| item["ordinal"].as_i64().unwrap())
        .collect();
    assert_eq!(ordinals, [0, 1, 2]);
    assert_eq!(items[1]["targetThreadId"], json!("child-1"));
    assert_eq!(items[1]["runId"], json!("task-1:1"));
    assert_eq!(items[1]["title"], json!("Delegated child-1"));
}

#[test]
fn a_moat_tasks_create_command_is_the_card_of_the_task_it_created() {
    let child = delegated("child-1", Utc::now() - chrono::Duration::minutes(1));
    let create = tool_call(
        "call-create",
        "Bash",
        json!({ "command": "moat tasks create --repo app --text \"Fix it\"" }),
        Some("Task child-1 created and started (status: open)."),
    );
    let other = tool_call(
        "call-other",
        "Bash",
        json!({ "command": "moat tasks create --repo app" }),
        Some("Task somebody-else created (status: open)."),
    );
    let transcript = Transcript {
        activities: vec![create, other],
        ..Transcript::default()
    };

    let projection = delegating_projection(&transcript, &[child]);

    let items = projection["turnItems"].as_array().expect("turnItems");
    let card = items
        .iter()
        .find(|item| item["id"] == json!("item:activity:call-create"))
        .expect("the create call");
    assert_eq!(card["type"], json!("subagent"));
    assert_eq!(card["childThreadId"], json!("child-1"));
    assert_eq!(card["subagentId"], projection["subagents"][0]["id"]);
    assert_eq!(card["origin"], json!("app_owned"));
    assert!(card.get("input").is_none());
    let unmatched = items
        .iter()
        .find(|item| item["id"] == json!("item:activity:call-other"))
        .expect("the other call");
    assert_eq!(unmatched["type"], json!("command_execution"));
    assert!(
        items
            .iter()
            .all(|item| item["type"] != json!("thread_created")),
        "the card is the child's one entry"
    );
}

#[test]
fn the_shell_carries_the_child_task_count() {
    let mut parent = conversation("task-1");
    parent.child_count = 3;
    assert_eq!(shell(&parent)["childCount"], json!(3));
}

#[test]
fn a_shell_reports_what_its_activity_run_is_doing() {
    let mut open = open_turn(conversation("task-1"), 1, "active");
    assert_eq!(shell(&open)["activityRunStatus"], json!("running"));
    open.pending_question = true;
    assert_eq!(shell(&open)["activityRunStatus"], json!("waiting"));
    let mut owed = awake(open_turn(conversation("task-1"), 1, "completed"));
    owed.turn_ended_at = Some(Utc::now());
    owed.latest_queued_message_at = Some(Utc::now());
    assert_eq!(shell(&owed)["activityRunStatus"], json!("starting"));
    let mut settled = open_turn(conversation("task-1"), 1, "completed");
    settled.turn_ended_at = Some(Utc::now());
    assert_eq!(shell(&settled)["activityRunStatus"], Value::Null);
}

mod lifecycle;

fn secret_requested(activity_id: &str, request_id: &str) -> Activity {
    let mut row = activity(
        activity_id,
        None,
        ActivityKind::SecretRequested,
        ActivityStatus::Completed,
    );
    row.payload = json!({
        "requestId": request_id,
        "key": "STAGING_API_KEY",
        "scope": "user",
        "kind": "env",
        "label": "Staging API key",
        "reason": "To call the staging API",
        "requestedBy": "user-1",
    });
    row
}

/// A request stays pending whatever the turn does, unlike a question: the
/// agent ends its turn and is woken by the answer.
#[test]
fn a_secret_request_is_pending_until_its_answer_names_it() {
    let conversation = open_turn(conversation("task-1"), 1, "completed");
    let card = |activities: Vec<Activity>| {
        let transcript = Transcript {
            activities,
            ..Transcript::default()
        };
        let (items, _) =
            turn_items_and_messages(&conversation, &transcript, &origins(&HashMap::new()));
        let cards: Vec<Value> = items
            .into_iter()
            .filter(|item| item["type"] == json!("secret_request"))
            .collect();
        assert_eq!(cards.len(), 1, "the answer renders no item of its own");
        cards[0].clone()
    };

    let pending = card(vec![secret_requested("act-1", "req-1")]);
    assert_eq!(pending["secretStatus"], json!("pending"));
    assert_eq!(pending["label"], json!("Staging API key"));
    assert_eq!(pending["storage"]["key"], json!("STAGING_API_KEY"));

    let mut answered = activity(
        "act-2",
        None,
        ActivityKind::SecretRequestAnswered,
        ActivityStatus::Completed,
    );
    answered.payload = json!({
        "requestId": "req-1",
        "outcome": "declined",
        "answeredBy": "user-1",
        "answeredByLogin": "alice",
    });
    let declined = card(vec![secret_requested("act-1", "req-1"), answered]);
    assert_eq!(declined["secretStatus"], json!("declined"));
    assert_eq!(declined["id"], pending["id"]);
}
