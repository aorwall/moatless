//! The Conversation, Workspace and Transcript values every case here projects,
//! and the two readers that project one.

use super::*;

pub(super) fn conversation(task_id: &str, workspace_id: &str) -> Conversation {
    Conversation {
        task_id: task_id.to_string(),
        workspace_id: Some(workspace_id.to_string()),
        name: Some("Fix the checkout".to_string()),
        branch: Some("feat/checkout".to_string()),
        agent_type: AgentType(AgentType::CLAUDE_CODE.to_string()),
        created_at: "2026-07-01T09:00:00.000Z".to_string(),
        created_at_db: Some(Utc::now()),
        updated_at: Some(Utc::now()),
        closed_at: None,
        visibility: moatless_types::task::Visibility::Private,
        owner_name: None,
        turn_number: None,
        turn_status: None,
        turn_started_at: None,
        turn_ended_at: None,
        sandbox_desired_state_raw: None,
        sandbox_stop_reason_raw: None,
        sandbox_started_at: None,
        sandbox_updated_at: None,
        running_commands: 0,
        latest_user_message_at: None,
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

/// A conversation with every optional part of the row filled in, so the key
/// assertions see the widest shape this module ever emits.
pub(super) fn running_conversation(task_id: &str, workspace_id: &str) -> Conversation {
    let now = Utc::now();
    Conversation {
        turn_number: Some(3),
        turn_status: Some("active".to_string()),
        turn_started_at: Some(now),
        sandbox_desired_state_raw: Some(SandboxDesiredState::Running.as_str().to_string()),
        sandbox_started_at: Some(now),
        sandbox_updated_at: Some(now),
        latest_user_message_at: Some(now),
        assistant_model: Some("claude-sonnet-4-6".to_string()),
        owner_name: Some("Ada Lovelace".to_string()),
        running_commands: 2,
        ..conversation(task_id, workspace_id)
    }
}

pub(super) fn threads(
    workspaces: &[UiRpcListedWorkspace],
    conversations: &[Conversation],
) -> Vec<Value> {
    shell_snapshot(workspaces, conversations, &AgentModelDefaults::default(), 0)["threads"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

pub(super) fn one_thread(conversation: &Conversation) -> Value {
    let listed = threads(
        &[workspace("ws-a", "Alpha")],
        std::slice::from_ref(conversation),
    );
    assert_eq!(listed.len(), 1, "the conversation should have been listed");
    listed[0].clone()
}

/// A workspace this viewer may read. The widened half of the union — a
/// workspace listed only because a conversation references it — is
/// [`a_referenced_workspace`].
pub(super) fn workspace(id: &str, name: &str) -> UiRpcListedWorkspace {
    UiRpcListedWorkspace {
        readable: true,
        workspace: Workspace {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            icon: None,
            default_model: None,
            default_effort: None,
            kind: "blank".to_string(),
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            scripts: None,
            resources: None,
            network_profile: None,
            source: "manual".to_string(),
            source_repository_id: None,
            source_key: None,
            source_config_path: None,
            scope: Scope::User,
            owner_user_id: Some("user-1".to_string()),
            created_by: Some("user-1".to_string()),
            deleted: Some(false),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        },
    }
}

/// The same row, listed only because something the viewer follows runs in
/// it. The read rule refused it.
pub(super) fn a_referenced_workspace(id: &str, name: &str) -> UiRpcListedWorkspace {
    UiRpcListedWorkspace {
        readable: false,
        ..workspace(id, name)
    }
}

/// A fixed instant the fixture's rows are dated from, so the ordering
/// between a Message and an activity is stated rather than raced.
pub(super) fn at(offset: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_780_000_000 + offset, 0).expect("valid instant")
}

pub(super) fn message(id: i32, message_type: MessageType, text: &str) -> TranscriptMessage {
    TranscriptMessage {
        id,
        uuid: format!("msg-{id}"),
        message_type,
        message_role: match message_type {
            MessageType::User => MessageRole::User,
            MessageType::Assistant => MessageRole::Assistant,
            _ => MessageRole::System,
        },
        message_text: Some(text.to_string()),
        user_type: "human".to_string(),
        turn_number: Some(3),
        // Minute `id` past the hour, so the fixture's own order is legible
        // in an assertion failure.
        timestamp: format!("2026-07-01T10:{id:02}:00.000Z"),
        created_at: Some(at(i64::from(id) * 100)),
        external_subject_key: None,
        external_author: None,
        source_task_id: None,
        binding_url: None,
        binding_label: None,
        sent_by_name: None,
        file_contexts: None,
        external_event_id: None,
        external_adapter: None,
        external_source_name: None,
    }
}

/// One activity of `kind`, written by block `ordinal` of the Message at
/// `position`. Every activity of one Message shares that Message's single
/// change position, which is what the writer stores.
pub(super) fn activity(
    message_uuid: &str,
    position: i64,
    ordinal: i32,
    kind: ActivityKind,
    summary: &str,
) -> Activity {
    Activity {
        activity_id: format!("{message_uuid}:{ordinal}"),
        task_id: "task-a".to_string(),
        position,
        ordinal,
        message_uuid: Some(message_uuid.to_string()),
        parent_uuid: None,
        kind,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: summary.to_string(),
        payload: json!({ "summary": summary }),
        detail: None,
        is_error: false,
        created_at: at(100 + position),
    }
}

/// One tool call, as the writer stores it once its result has landed.
pub(super) fn call(
    message_uuid: &str,
    position: i64,
    ordinal: i32,
    id: &str,
    name: &str,
    input: Value,
) -> Activity {
    let mut activity = activity(
        message_uuid,
        position,
        ordinal,
        ActivityKind::ToolCall,
        name,
    );
    activity.tool_use_id = Some(id.to_string());
    let mut payload = json!({
        "title": name,
        "itemType": moatless_types::activity::item_type(name),
        "status": "completed",
        "data": { "toolCallId": id },
        "input": input,
    });
    if let Some(command) = input.get("command") {
        payload["command"] = command.clone();
    }
    activity.payload = payload;
    activity
}

/// The rows, plus the requests the database would report settled for them —
/// which for rows that carry their own call is exactly what it reads off
/// them. A test whose call sits outside the rows sets the field directly.
pub(super) fn transcript(
    messages: Vec<TranscriptMessage>,
    activities: Vec<Activity>,
) -> Transcript {
    let settled_requests = activities
        .iter()
        .filter(|activity| {
            activity.kind == ActivityKind::ToolCall && ActivityView::new(activity).is_settled()
        })
        .filter_map(|activity| activity.tool_use_id.clone())
        .collect();
    let mut turns: Vec<i32> = messages.iter().filter_map(|m| m.turn_number).collect();
    turns.sort_unstable();
    turns.dedup();
    Transcript {
        messages,
        activities,
        omitted: 0,
        // What `task_turns` holds for these rows. A test about a turn with
        // no Message of its own sets this itself.
        turns,
        settled_requests,
        subagent_usage: HashMap::new(),
    }
}

/// The fixture transcript: a person asks; the agent thinks, runs a command
/// that works and one that fails — saying nothing at all while it does, so
/// that turn has activities and no Message row — and then answers.
pub(super) fn a_conversation() -> Transcript {
    let mut bash = call(
        "msg-2",
        2,
        1,
        "call-1",
        "Bash",
        json!({ "command": "cargo build" }),
    );
    bash.payload["result"] = json!("Compiling");
    bash.detail = Some("Compiling".to_string());
    let mut edit = call(
        "msg-2",
        2,
        2,
        "call-2",
        "Edit",
        json!({ "file_path": "src/main.rs" }),
    );
    edit.payload["status"] = json!("failed");
    edit.payload["result"] = json!("no such file");
    edit.detail = Some("no such file".to_string());
    edit.status = ActivityStatus::Failed;
    edit.is_error = true;

    transcript(
        vec![
            message(1, MessageType::User, "Fix the build"),
            message(4, MessageType::Assistant, "Done."),
        ],
        vec![
            activity("msg-2", 2, 0, ActivityKind::Thinking, "Thinking"),
            bash,
            edit,
            // The answering Message thought once before it spoke: a bubble
            // and an activity in the same, later change.
            activity("msg-4", 4, 0, ActivityKind::Thinking, "Thinking"),
        ],
    )
}

pub(super) fn detail(transcript: &Transcript) -> Value {
    detail_of(running_conversation("task-a", "ws-a"), transcript)
}

/// The same, with the fixture's turn finished — the only state that offers
/// a checkpoint, since a rewind is refused while a turn is in flight.
pub(super) fn quiet_detail(transcript: &Transcript) -> Value {
    let mut conversation = running_conversation("task-a", "ws-a");
    conversation.turn_status = Some("completed".to_string());
    detail_of(conversation, transcript)
}

pub(super) fn detail_of(conversation: Conversation, transcript: &Transcript) -> Value {
    thread_detail(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        transcript,
        &OriginContext {
            frontend_url: "http://localhost:3000",
            source_task_names: &HashMap::new(),
        },
    )
}

/// One `file` context row, as the join reads it.
pub(super) fn a_file_context(
    reference_id: &str,
    filename: &str,
    mime_type: &str,
    size: i64,
) -> Value {
    json!({
        "referenceId": reference_id,
        "data": { "filename": filename, "mimeType": mime_type, "size": size }
    })
}

pub(super) fn with_files(
    mut message: TranscriptMessage,
    contexts: Vec<Value>,
) -> TranscriptMessage {
    message.file_contexts = Some(Value::Array(contexts));
    message
}

pub(super) fn instant_at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("a well-formed instant")
        .with_timezone(&Utc)
}

/// The change that names Message `uuid`, at `position`.
pub(super) fn change(position: i64, uuid: Option<&str>) -> ConversationChange {
    ConversationChange {
        position,
        message_uuid: uuid.map(str::to_string),
        activity_id: None,
        reverted_to_turn: None,
    }
}

pub(super) fn events(change: &ConversationChange, window: &Transcript) -> Vec<Value> {
    change_events(
        &running_conversation("task-a", "ws-a"),
        change,
        window,
        &OriginContext {
            frontend_url: "http://localhost:3000",
            source_task_names: &HashMap::new(),
        },
    )
    .into_iter()
    .map(|item| {
        assert_eq!(item["kind"], "event", "every item is a stream event");
        item["event"].clone()
    })
    .collect()
}
