//! What a batch of conversation changes is read as: a change names the row it
//! is about, and for an agent's own writes the named rows are complete.
//!
//! The `tool_result` case: it arrives on its own Message but settles the
//! *call*'s activity on an earlier row, and the change is recorded against the
//! calling Message. A person's answer does not: it lands on a Message of its
//! own, so `changed_messages` recovers the call it settles.
//!
//! Held against a real database: a property of `append_activities` and
//! `changed_messages` together.

use crate::dao::{ConversationDao, PgConversationDao};
use moatless_api_schemas::task::{FileContextData, MessageContextInput, PreviewUrlContextData};
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_core::change_feed::conversation::changes_from;
use moatless_core::change_feed::conversation::current_head;
use moatless_tasks::activities::mapping::BlockContext;
use moatless_tasks::activities::mapping::ConversationWrite;
use moatless_tasks::activities::mapping::ToolOutcome;
use moatless_tasks::activities::mapping::context_window_updated;
use moatless_tasks::activities::mapping::map_content;
use moatless_tasks::activities::mapping::sandbox_activity;
use moatless_tasks::activities::mapping::user_input_resolved;
use moatless_tasks::activities::mapping::{ActivityKind, ActivityStatus};
use moatless_tasks::message::contexts::message_context_input_to_row;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::test_support as message_fixtures;
use moatless_tasks::message::test_support::make_message_row;
use moatless_tasks::task::test_support::make_task_row;
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::SandboxEventPayload;
use moatless_types::task::TaskStatus;
use moatless_types::task::Visibility;
use moatless_workspaces::test_support::make_repository_row;
use sqlx::PgPool;

const TASK: &str = "task-thread-window";
const USER_1: &str = "11111111-1111-1111-1111-111111111111";
const USER_2: &str = "22222222-2222-2222-2222-222222222222";

/// The read model over `pool`, with its activity seam on the same pool.
fn conversations_over(pool: &PgPool) -> PgConversationDao {
    let tasks = moatless_tasks::compose::task_graph(pool.clone());
    PgConversationDao::new(
        pool.clone(),
        tasks.activities(),
        tasks.transcript_reads(),
        tasks.task_reader(),
    )
}

async fn seed_task(pool: &PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row(TASK, USER_1);
    moatless_tasks::task::test_support::seed_task(pool, &task)
        .await
        .expect("seed task");
}

/// Map one Message's content the way the write path does.
fn write(uuid: &str, message_type: MessageType, content: &serde_json::Value) -> ConversationWrite {
    map_content(
        BlockContext {
            task_id: TASK,
            message_uuid: uuid,
            parent_uuid: None,
            message_type,
            block_index: 0,
            source: uuid,
            source_block: 0,
        },
        content,
    )
}

/// Append one Message and return its row id, in the order the calls are made.
async fn append(pool: &PgPool, uuid: &str, content: serde_json::Value) -> i32 {
    let msg = make_message_row(TASK, uuid);
    let conversation = write(uuid, msg.message_type, &content);
    message_fixtures::seed_message(pool, &msg, &[], &conversation)
        .await
        .unwrap()
}

fn tool_use(id: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "tool_use", "id": id, "name": "Bash", "input": {} }])
}

fn tool_result(id: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "tool_result", "tool_use_id": id, "content": "ok" }])
}

/// A result reaches the reader as a change to the row it answers.
///
/// No text of its own; it settles the call's activity and names the *calling*
/// Message, so no run before it is needed.
#[sqlx::test]
async fn a_result_is_read_as_a_change_to_the_call_it_answers(pool: PgPool) {
    seed_task(&pool).await;

    let rewritten = append(
        &pool,
        "old-text",
        serde_json::json!([{"type": "text", "text": "hi"}]),
    )
    .await;
    let call = append(&pool, "the-call", tool_use("toolu_1")).await;

    // The result arrives later, on its own source event, and settles in place.
    moatless_tasks::message::test_support::seed_activities(
        &pool,
        TASK,
        "the-call",
        &write("the-result", MessageType::User, &tool_result("toolu_1")),
    )
    .await
    .unwrap();

    let dao = conversations_over(&pool);

    let window = dao
        .changed_messages(
            TASK,
            &[message_fixtures::seeded_uuid("the-call").to_string()],
            &[],
        )
        .await
        .unwrap();

    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        vec![call],
        "the change names the call, so exactly that row is read — not a run \
         reaching back over {rewritten}"
    );
    assert_eq!(
        window.activities.len(),
        1,
        "the call's activity comes with it"
    );
    assert_eq!(
        window.activities[0].status,
        ActivityStatus::Completed,
        "the result settled the activity rather than arriving as a row of its own"
    );
    assert!(
        window.activities[0].result().is_some(),
        "the outcome is on the activity the reader already has"
    );
}

/// Only the rows a batch names are read.
///
/// Every change describes the row it names — no lookback, no row dragged along
/// for context.
#[sqlx::test]
async fn a_batch_is_read_as_the_rows_it_names(pool: PgPool) {
    seed_task(&pool).await;

    append(
        &pool,
        "first",
        serde_json::json!([{"type": "text", "text": "one"}]),
    )
    .await;
    append(
        &pool,
        "second",
        serde_json::json!([{"type": "text", "text": "two"}]),
    )
    .await;
    let third = append(
        &pool,
        "third",
        serde_json::json!([{"type": "text", "text": "three"}]),
    )
    .await;

    let window = conversations_over(&pool)
        .changed_messages(
            TASK,
            &[message_fixtures::seeded_uuid("third").to_string()],
            &[],
        )
        .await
        .unwrap();

    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        vec![third],
        "a change must not drag the rows before it along"
    );
}

/// A question is stored twice: as the call, and as the question the panel is
/// derived from.
///
/// Both share one change, block and ordinal; the ordinal must survive a
/// redelivery, and the read must return both in written order.
#[sqlx::test]
async fn a_question_is_stored_as_a_call_and_as_the_question_it_asked(pool: PgPool) {
    seed_task(&pool).await;

    let content = serde_json::json!([
        { "type": "thinking", "thinking": "what to ask" },
        {
            "type": "tool_use",
            "id": "toolu_ask",
            "name": "AskUserQuestion",
            "input": { "questions": [{
                "question": "Which store?",
                "options": [{ "label": "Postgres" }, { "label": "Redis" }]
            }] }
        }
    ]);
    append(&pool, "the-question", content.clone()).await;
    // The same source event redelivered must change nothing about where its
    // rows sit.
    moatless_tasks::message::test_support::seed_activities(
        &pool,
        TASK,
        "the-question",
        &write("the-question", MessageType::Assistant, &content),
    )
    .await
    .unwrap();

    let transcript = conversations_over(&pool).transcript(TASK).await.unwrap();

    let rows: Vec<(String, ActivityKind, i32)> = transcript
        .activities
        .iter()
        .map(|activity| {
            (
                activity.activity_id.clone(),
                activity.kind,
                activity.ordinal,
            )
        })
        .collect();
    let id = message_fixtures::seeded_activity_id;
    assert_eq!(
        rows,
        vec![
            (id("the-question:0"), ActivityKind::Thinking, 0),
            (id("the-question:1"), ActivityKind::ToolCall, 1),
            (
                id("the-question:1:user-input"),
                ActivityKind::UserInputRequested,
                1
            ),
        ],
        "the joined row shares the call's block, and the read hands them back in \
         written order"
    );

    // All three are one Message's, so they share its change position — the
    // ordinal is what separates them.
    let positions: Vec<i64> = transcript
        .activities
        .iter()
        .map(|activity| activity.position)
        .collect();
    assert_eq!(positions[0], positions[1]);
    assert_eq!(positions[1], positions[2]);

    let asked = &transcript.activities[2];
    assert_eq!(asked.payload["requestId"], "toolu_ask");
    assert_eq!(asked.payload["questions"][0]["id"], "Which store?");

    // The call itself is still a call, with its input where it has always been.
    let call = conversations_over(&pool)
        .interactive_call(TASK, "toolu_ask")
        .await
        .unwrap()
        .expect("the call is stored");
    assert_eq!(call.tool_name, "AskUserQuestion");
    assert!(!call.settled, "nobody has answered it yet");
}

/// A question re-presented on a later Message is read as settled, though that
/// Message holds no call.
///
/// The call is stored once (unique on `(task_id, tool_use_id)`); the panel row
/// is rewritten on the new Message, so a client waiting on settlement needs
/// this read to find it.
#[sqlx::test]
async fn a_re_presented_question_is_read_as_settled_without_its_call(pool: PgPool) {
    seed_task(&pool).await;

    let asked = serde_json::json!([{
        "type": "tool_use",
        "id": "toolu_ask",
        "name": "AskUserQuestion",
        "input": { "questions": [{
            "question": "Which store?",
            "options": [{ "label": "Postgres" }, { "label": "Redis" }]
        }] }
    }]);
    append(&pool, "the-question", asked.clone()).await;

    // Answered, wherever from: the call comes back.
    moatless_tasks::message::test_support::seed_activities(
        &pool,
        TASK,
        "the-question",
        &write("the-answer", MessageType::User, &tool_result("toolu_ask")),
    )
    .await
    .unwrap();

    // The session resumes and presents the same call again, on a new Message.
    append(&pool, "the-resume", asked).await;

    let dao = conversations_over(&pool);
    let window = dao
        .changed_messages(
            TASK,
            &[message_fixtures::seeded_uuid("the-resume").to_string()],
            &[],
        )
        .await
        .unwrap();

    assert!(
        window
            .activities
            .iter()
            .all(|activity| activity.kind != ActivityKind::ToolCall),
        "the call stayed on the Message it was first presented on: {:?}",
        window
            .activities
            .iter()
            .map(|activity| (activity.activity_id.as_str(), activity.kind))
            .collect::<Vec<_>>()
    );
    assert!(
        window.activities.iter().any(|activity| {
            activity.kind == ActivityKind::UserInputRequested
                && activity.payload["requestId"] == "toolu_ask"
        }),
        "the panel row was written again"
    );
    assert!(
        window.settled_requests.contains("toolu_ask"),
        "the request is read as settled from the call, wherever it is stored"
    );
}

/// An activity that happened on no Message is named by the change, and read
/// back by that name.
///
/// A context-window reading is the live case: written through the standalone
/// path, read back by activity id.
#[sqlx::test]
async fn a_standalone_activity_change_names_the_activity_and_reads_back(pool: PgPool) {
    seed_task(&pool).await;

    let reading = context_window_updated("evt-window", TASK, 1_000, Some(200_000));
    let written =
        moatless_tasks::message::test_support::record_standalone_activity(&pool, &reading)
            .await
            .unwrap();
    assert!(written, "the first reading is a new row");

    let changes = changes_from(&pool, TASK, 0, 10).await.unwrap();
    assert_eq!(changes.len(), 1, "one change, for the one reading");
    assert_eq!(
        changes[0].activity_id,
        Some(message_fixtures::seeded_activity_id("evt-window")),
        "the change names the activity, not a Message"
    );
    assert!(changes[0].message_uuid.is_none());

    let window = conversations_over(&pool)
        .changed_messages(
            TASK,
            &[],
            &[message_fixtures::seeded_activity_id("evt-window")],
        )
        .await
        .unwrap();
    assert!(window.messages.is_empty(), "it happened on no Message");
    assert_eq!(window.activities.len(), 1);
    assert_eq!(
        window.activities[0].activity_id,
        message_fixtures::seeded_activity_id("evt-window")
    );
    assert_eq!(
        window.activities[0].kind,
        ActivityKind::ContextWindowUpdated
    );
    assert_eq!(window.activities[0].payload["usedTokens"], 1_000);
}

/// A Sandbox status that restates the one before it writes nothing and spends
/// no position: the collapse check runs before the allocation, so a restated
/// status records no change for a subscriber to be woken by.
#[sqlx::test]
async fn a_restated_sandbox_status_records_no_change_and_spends_no_position(pool: PgPool) {
    seed_task(&pool).await;
    let ready = sandbox_activity(
        "evt-1",
        TASK,
        "ready",
        false,
        &SandboxEventPayload::default(),
    );
    assert!(
        moatless_tasks::message::test_support::record_sandbox_activity(&pool, &ready)
            .await
            .unwrap(),
        "the first status is a new row"
    );
    let after_first = current_head(&pool, TASK).await.unwrap().position;

    // The same status restated, under a new event id.
    let restated = sandbox_activity(
        "evt-2",
        TASK,
        "ready",
        false,
        &SandboxEventPayload::default(),
    );
    assert!(
        !moatless_tasks::message::test_support::record_sandbox_activity(&pool, &restated)
            .await
            .unwrap(),
        "a restated status writes no row"
    );
    let after_restate = current_head(&pool, TASK).await.unwrap().position;
    assert_eq!(
        after_first, after_restate,
        "the collapse spends no position, so it records no change"
    );

    let changes = changes_from(&pool, TASK, 0, 10).await.unwrap();
    assert_eq!(changes.len(), 1, "only the first status recorded a change");
    assert_eq!(
        changes[0].activity_id,
        Some(message_fixtures::seeded_activity_id("evt-1"))
    );
}

/// A Message's uploads reach the reader on the Message itself.
///
/// Joined from `file` context rows only, scoped to this Message, each carrying
/// the metadata a rendered attachment needs.
#[sqlx::test]
async fn a_message_carries_the_uploads_attached_to_it(pool: PgPool) {
    seed_task(&pool).await;

    let contexts = vec![
        message_context_input_to_row(&MessageContextInput::File {
            reference_id: "file-1".to_string(),
            data: FileContextData {
                filename: "screenshot.png".to_string(),
                mime_type: "image/png".to_string(),
                size: 4096,
            },
        }),
        // A context of another type shares the table and must not be read as
        // an upload.
        message_context_input_to_row(&MessageContextInput::PreviewUrl {
            reference_id: "preview".to_string(),
            data: PreviewUrlContextData {
                path: "/".to_string(),
                url: None,
            },
        }),
    ];

    let with_upload = make_message_row(TASK, "with-upload");
    message_fixtures::seed_message(
        &pool,
        &with_upload,
        &contexts,
        &ConversationWrite::default(),
    )
    .await
    .unwrap();
    append(&pool, "no-upload", serde_json::json!([])).await;

    let transcript = conversations_over(&pool).transcript(TASK).await.unwrap();

    let attached = transcript
        .messages
        .iter()
        .find(|m| m.uuid == message_fixtures::seeded_uuid("with-upload").to_string())
        .expect("the message is in the transcript");
    assert_eq!(
        attached.file_contexts,
        Some(serde_json::json!([{
            "referenceId": "file-1",
            "data": { "filename": "screenshot.png", "mimeType": "image/png", "size": 4096 }
        }])),
        "only the file context, and with the metadata the attachment renders from"
    );

    let plain = transcript
        .messages
        .iter()
        .find(|m| m.uuid == message_fixtures::seeded_uuid("no-upload").to_string())
        .expect("the message is in the transcript");
    assert_eq!(
        plain.file_contexts, None,
        "a Message with no uploads joins to nothing rather than to an empty array"
    );
}

/// The window a plan's answer names holds the call it answers, though that call
/// is on an earlier Message that recorded no change of its own.
///
/// Without the call the projection has no plan to read, so a client is never
/// told the plan was answered.
#[sqlx::test]
async fn a_windowed_answer_reaches_the_plan_call_it_settles(pool: PgPool) {
    seed_task(&pool).await;

    append(
        &pool,
        "the-plan",
        serde_json::json!([{
            "type": "tool_use",
            "id": "toolu_plan",
            "name": "ExitPlanMode",
            "input": { "plan": "1. Do the thing" }
        }]),
    )
    .await;

    // The answer, exactly as `message::send` writes one: the resolved row on a
    // Message of its own, and an outcome that settles the call where it lives.
    let answer = serde_json::json!([{ "answer": "Approve" }]);
    let msg = make_message_row(TASK, "the-answer");
    let conversation = ConversationWrite {
        outcomes: vec![ToolOutcome {
            task_id: TASK.to_string(),
            tool_use_id: "toolu_plan".to_string(),
            status: ActivityStatus::Completed,
            result: answer.clone(),
            typed_result: None,
            is_error: false,
        }],
        activities: vec![user_input_resolved(
            TASK,
            "the-answer",
            "toolu_plan",
            &answer,
        )],
        ..Default::default()
    };
    message_fixtures::seed_message(&pool, &msg, &[], &conversation)
        .await
        .unwrap();

    let dao = conversations_over(&pool);
    let window = dao
        .changed_messages(
            TASK,
            &[message_fixtures::seeded_uuid("the-answer").to_string()],
            &[],
        )
        .await
        .unwrap();

    let call = window
        .activities
        .iter()
        .find(|activity| activity.tool_use_id.as_deref() == Some("toolu_plan"))
        .expect("the call the answer settles");
    assert_eq!(call.summary, "ExitPlanMode");
    assert_eq!(
        call.tool_input()["plan"],
        "1. Do the thing",
        "the plan the card renders came back with the call"
    );
    assert!(call.is_settled(), "the answer settled it");

    // The call's own Message, which the card's `turnId` and `createdAt` are
    // read off: without it the projection falls back to the conversation's
    // newest Turn and the call's insert time, and a reload disagrees.
    let anchor = call.message_uuid.as_deref().expect("the call's Message");
    assert!(
        window.messages.iter().any(|message| message.uuid == anchor),
        "the call's Message came back with it, got {:?}",
        window
            .messages
            .iter()
            .map(|message| message.uuid.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        window.activities.windows(2).all(|pair| {
            (pair[0].position, pair[0].ordinal) <= (pair[1].position, pair[1].ordinal)
        }),
        "the widened window is still oldest first"
    );
}

/// Seed closed Tasks `archived-{from}` to `archived-{to - 1}`, owned and so
/// followed by `USER_1`, each a minute older than the one before it.
async fn seed_archived(pool: &PgPool, from: i64, to: i64) {
    for index in from..to {
        let mut task = make_task_row(&format!("archived-{index:03}"), USER_1);
        task.workspace_id = Some("ws-archive".to_string());
        task.status = TaskStatus::Closed;
        moatless_tasks::task::test_support::seed_task(pool, &task)
            .await
            .expect("seed task");
    }
    sqlx::query(
        "UPDATE tasks SET updated_at = now() - substring(task_id FROM 10)::int * interval '1 minute'
         WHERE task_id LIKE 'archived-%'",
    )
    .execute(pool)
    .await
    .expect("stagger updated_at");
}

/// The archive's cursor reaches every row past the first page, down to one
/// with no `updated_at`, which sorts last.
///
/// A page filled exactly to the cap has no cursor: the extra row the query
/// reads is only a signal.
#[sqlx::test]
async fn the_archive_pages_through_every_closed_task_newest_first(pool: PgPool) {
    let limit = crate::dao::ARCHIVED_LIMIT;
    seed_archived(&pool, 0, limit).await;
    let tasks = moatless_tasks::compose::task_graph(pool.clone());
    let graph = crate::compose::t3_graph(pool.clone(), &tasks);
    let defaults = crate::config::AgentModelDefaults::default();
    let viewer = ScopeContext::user(USER_1.to_string());
    let page = |before: Option<crate::dao::ArchiveCursor>| {
        let (graph, defaults, viewer) = (&graph, &defaults, &viewer);
        async move {
            crate::snapshots::archived_shell_page(
                graph.reads(),
                defaults,
                viewer,
                before,
                crate::config::OrchestrationProtocol::V1,
            )
            .await
            .unwrap()
        }
    };
    let ids = |snapshot: &serde_json::Value| -> Vec<String> {
        snapshot["threads"]
            .as_array()
            .expect("threads")
            .iter()
            .map(|thread| thread["id"].as_str().expect("id").to_string())
            .collect()
    };

    let full = page(None).await;
    assert_eq!(ids(&full).len() as i64, limit);
    assert_eq!(full["nextCursor"], serde_json::Value::Null);

    seed_archived(&pool, limit, limit + 2).await;
    let undated = format!("archived-{:03}", limit + 1);
    sqlx::query("UPDATE tasks SET updated_at = NULL WHERE task_id = $1")
        .bind(&undated)
        .execute(&pool)
        .await
        .expect("clear updated_at");
    let rows = conversations_over(&pool)
        .list_followed_closed(&viewer, None)
        .await
        .unwrap();
    assert_eq!(
        rows.len() as i64,
        limit + 1,
        "the query stops one row past the cap"
    );

    let first = page(None).await;
    let first_ids = ids(&first);
    assert_eq!(first_ids.len() as i64, limit);
    assert_eq!(first_ids.first().map(String::as_str), Some("archived-000"));
    let cursor = first["nextCursor"].as_str().expect("a cursor past the cap");

    let second = page(Some(
        crate::dao::ArchiveCursor::decode(cursor).expect("the cursor decodes"),
    ))
    .await;
    assert_eq!(
        ids(&second),
        vec![format!("archived-{limit:03}"), undated],
        "the next page holds exactly the rows the first left out, undated last"
    );
    assert_eq!(second["nextCursor"], serde_json::Value::Null);
}

/// The one-row access check a silent thread poll asks gives the same answer
/// as the full read it stands in for, across the read and status rules.
#[sqlx::test]
async fn the_access_check_agrees_with_the_conversation_read(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    let private = make_task_row("check-private", USER_1);
    let mut public = make_task_row("check-public", USER_1);
    public.visibility = Visibility::Public;
    let mut closed = make_task_row("check-closed", USER_1);
    closed.status = TaskStatus::Closed;
    let mut errored = make_task_row("check-error", USER_1);
    errored.status = TaskStatus::Error;
    for task in [&private, &public, &closed, &errored] {
        moatless_tasks::task::test_support::seed_task(&pool, task)
            .await
            .expect("seed task");
    }

    let dao = conversations_over(&pool);
    let owner = ScopeContext::user(USER_1.to_string());
    let stranger = ScopeContext::user(USER_2.to_string());
    let cases = [
        (&owner, "check-private", true),
        (&stranger, "check-private", false),
        (&stranger, "check-public", true),
        (&owner, "check-closed", true),
        (&owner, "check-error", true),
        (&stranger, "check-error", false),
        (&owner, "check-missing", false),
    ];
    for (viewer, task_id, expected) in cases {
        let read = dao.readable_by_id(viewer, task_id).await.unwrap().is_some();
        let checked = dao.can_read_conversation(viewer, task_id).await.unwrap();
        assert_eq!(read, expected, "{task_id} read by {}", viewer.user_id);
        assert_eq!(checked, read, "{task_id} checked by {}", viewer.user_id);
    }
}

/// A Message's origin names come from three tables the Task band does not own,
/// joined beside the rows `TranscriptReads` returns: the Binding its subject
/// key names, the Loop a `scheduled` Message was fired by, and its sender.
#[sqlx::test]
async fn a_message_reads_the_names_its_origin_resolves_to(pool: PgPool) {
    seed_task(&pool).await;
    sqlx::query(
        "INSERT INTO users (id, login, name, email, provider, created_at, last_login_at) \
         VALUES ($1::uuid, 'ada', 'Ada Lovelace', 'ada@example.test', 'local', now(), now())",
    )
    .bind(USER_2)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO loops (id, name, kind, scope, created_by) \
         VALUES ('loop-1', 'Nightly sweep', 'schedule', 'global', 'system')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO adapter_connections \
             (id, adapter_kind, connection_kind, external_account_id, created_by, scope) \
         VALUES ('conn-1', 'linear', 'app', 'team-1', 'system', 'global')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO task_bindings \
             (id, task_id, adapter_kind, connection_id, subject_kind, subject_key, \
              destination_config) \
         VALUES ('binding-1', $1, 'linear', 'conn-1', 'issue', 'issue-1', $2)",
    )
    .bind(TASK)
    .bind(serde_json::json!({
        "issue_url": "https://linear.app/soap/issue/SP-1",
        "identifier": "SP-1"
    }))
    .execute(&pool)
    .await
    .unwrap();

    let mut from_linear = make_message_row(TASK, "from-linear");
    from_linear.external_adapter = Some("linear".into());
    from_linear.external_subject_key = Some("issue-1".into());
    from_linear.sent_by = Some(USER_2.into());
    let mut from_loop = make_message_row(TASK, "from-loop");
    from_loop.external_adapter = Some("scheduled".into());
    from_loop.external_source_name = Some("loop-1".into());
    for message in [&from_linear, &from_loop] {
        message_fixtures::seed_message(&pool, message, &[], &ConversationWrite::default())
            .await
            .unwrap();
    }
    append(&pool, "plain", serde_json::json!([])).await;

    let transcript = conversations_over(&pool).transcript(TASK).await.unwrap();
    let named = |id: &str| {
        let uuid = message_fixtures::seeded_uuid(id).to_string();
        transcript
            .messages
            .iter()
            .find(|m| m.uuid == uuid)
            .cloned()
            .expect("the message is in the transcript")
    };

    let linear = named("from-linear");
    assert_eq!(
        linear.binding_url.as_deref(),
        Some("https://linear.app/soap/issue/SP-1")
    );
    assert_eq!(linear.binding_label.as_deref(), Some("SP-1"));
    assert_eq!(linear.sent_by_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(
        named("from-loop").loop_name.as_deref(),
        Some("Nightly sweep")
    );
    let plain = named("plain");
    assert_eq!(
        (plain.binding_url, plain.loop_name, plain.sent_by_name),
        (None, None, None)
    );
}

/// The poller's one read answers every watched conversation, including one
/// with no change yet, at the head each would report alone.
#[sqlx::test]
async fn conversation_heads_reads_each_head_in_one_statement(pool: PgPool) {
    seed_task(&pool).await;
    append(&pool, "msg-1", tool_use("call-1")).await;
    let dao = conversations_over(&pool);

    let mut heads = dao
        .conversation_heads(&[TASK.to_string(), "never-written".to_string()])
        .await
        .unwrap();
    heads.sort_by(|a, b| a.0.cmp(&b.0));

    let head = current_head(&pool, TASK).await.unwrap();
    assert!(head.position > 0);
    assert_eq!(
        heads,
        vec![
            (
                "never-written".to_string(),
                ConversationHead { position: 0 }
            ),
            (TASK.to_string(), head),
        ]
    );
}
