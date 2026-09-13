//! What a batch of conversation changes is read as: a change names the row it
//! is about, and reading exactly the named rows is complete.
//!
//! The `tool_result` case: it arrives on its own Message but settles the
//! *call*'s activity on an earlier row, and the change is recorded against the
//! calling Message — so no reader needs a run of Messages before it.
//!
//! Held against a real database: a property of `append_activities` and
//! `changed_messages` together.

use crate::dao::{ConversationBrowse, ConversationDao, PgConversationDao};
use moatless_api_schemas::task::{FileContextData, MessageContextInput, PreviewUrlContextData};
use moatless_core::change_feed::conversation::changes_from;
use moatless_core::change_feed::conversation::current_head;
use moatless_tasks::message::contexts::message_context_input_to_row;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::test_support as message_fixtures;
use moatless_tasks::message::test_support::make_message_row;
use moatless_tasks::tag::test_support as tag_fixtures;
use moatless_tasks::task::test_support::make_task_row;
use moatless_tasks::task_activities::mapping::BlockContext;
use moatless_tasks::task_activities::mapping::ConversationWrite;
use moatless_tasks::task_activities::mapping::context_window_updated;
use moatless_tasks::task_activities::mapping::map_content;
use moatless_tasks::task_activities::mapping::sandbox_activity;
use moatless_tasks::task_activities::mapping::{ActivityKind, ActivityStatus};
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::SandboxEventPayload;
use moatless_types::task::TaskStatus;
use moatless_types::task::Visibility;
use moatless_workspaces::test_support::make_repository_row;
use sqlx::PgPool;

const TASK: &str = "task-thread-window";
const USER_1: &str = "11111111-1111-1111-1111-111111111111";
const USER_2: &str = "22222222-2222-2222-2222-222222222222";
/// The browsing viewer, who owns none of the Tasks below and follows none.
const ADMIN: &str = "33333333-3333-3333-3333-333333333333";

/// The read model over `pool`, with its activity seam on the same pool.
fn conversations_over(pool: &PgPool) -> PgConversationDao {
    let tasks = moatless_tasks::compose::task_graph(pool.clone());
    PgConversationDao::new(pool.clone(), tasks.activities())
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
        .changed_messages(TASK, &["the-call".to_string()], &[])
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
        .changed_messages(TASK, &["third".to_string()], &[])
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

    let rows: Vec<(&str, ActivityKind, i32)> = transcript
        .activities
        .iter()
        .map(|activity| {
            (
                activity.activity_id.as_str(),
                activity.kind,
                activity.ordinal,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("the-question:0", ActivityKind::Thinking, 0),
            ("the-question:1", ActivityKind::ToolCall, 1),
            (
                "the-question:1:user-input",
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
        .changed_messages(TASK, &["the-resume".to_string()], &[])
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
        changes[0].activity_id.as_deref(),
        Some("evt-window"),
        "the change names the activity, not a Message"
    );
    assert!(changes[0].message_uuid.is_none());

    let window = conversations_over(&pool)
        .changed_messages(TASK, &[], &["evt-window".to_string()])
        .await
        .unwrap();
    assert!(window.messages.is_empty(), "it happened on no Message");
    assert_eq!(window.activities.len(), 1);
    assert_eq!(window.activities[0].activity_id, "evt-window");
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
    assert_eq!(changes[0].activity_id.as_deref(), Some("evt-1"));
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
        .find(|m| m.uuid == "with-upload")
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
        .find(|m| m.uuid == "no-upload")
        .expect("the message is in the transcript");
    assert_eq!(
        plain.file_contexts, None,
        "a Message with no uploads joins to nothing rather than to an empty array"
    );
}

/// Seed one open Task owned by `user_id`, tagged with each name given.
async fn seed_browsable(pool: &PgPool, task_id: &str, user_id: &str, tags: &[&str]) {
    let task = make_task_row(task_id, user_id);
    moatless_tasks::task::test_support::seed_task(pool, &task)
        .await
        .expect("seed task");
    tag_with(pool, task_id, tags).await;
}

async fn tag_with(pool: &PgPool, task_id: &str, tags: &[&str]) {
    if tags.is_empty() {
        return;
    }
    let names: Vec<String> = tags.iter().map(|name| (*name).to_string()).collect();
    let created = tag_fixtures::find_or_create_tags(pool, &names)
        .await
        .expect("create tags");
    let ids: Vec<String> = created.into_iter().map(|tag| tag.tag_id).collect();
    tag_fixtures::add_task_tags(pool, task_id, &ids)
        .await
        .expect("tag task");
}

/// A browse reaches the Tasks of somebody the viewer is not, and each filter
/// narrows it.
///
/// The browse drops only the *follow* rule — none of these Tasks is followed or
/// owned by the viewer.
#[sqlx::test]
async fn a_browse_reaches_tasks_the_viewer_neither_follows_nor_owns(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    seed_browsable(&pool, "browse-hers-tagged", USER_1, &["backend"]).await;
    seed_browsable(&pool, "browse-hers-plain", USER_1, &[]).await;
    seed_browsable(&pool, "browse-theirs-tagged", USER_2, &["backend"]).await;

    let dao = conversations_over(&pool);
    let viewer = ScopeContext {
        user_id: ADMIN.to_string(),
        is_admin: true,
    };

    let by_owner = dao
        .list_browsable(
            &viewer,
            &ConversationBrowse::new(Some(USER_1.to_string()), None, false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&by_owner),
        vec!["browse-hers-plain", "browse-hers-tagged"],
        "an owner filter reaches every Task of that person's and nobody else's"
    );

    let by_tag = dao
        .list_browsable(
            &viewer,
            &ConversationBrowse::new(None, Some("backend".to_string()), false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&by_tag),
        vec!["browse-hers-tagged", "browse-theirs-tagged"],
        "a tag filter reaches the tagged Tasks of everyone"
    );

    let by_both = dao
        .list_browsable(
            &viewer,
            &ConversationBrowse::new(
                Some(USER_1.to_string()),
                Some("backend".to_string()),
                false,
            )
            .expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&by_both),
        vec!["browse-hers-tagged"],
        "the two filters narrow together rather than widening"
    );
}

/// Closed Tasks come back only when the browse asks for them.
///
/// Closing is T3's archiving; a browse folds both halves into one read, and
/// this flag picks which.
#[sqlx::test]
async fn a_browse_leaves_closed_tasks_out_until_it_is_asked_for_them(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    seed_browsable(&pool, "browse-open", USER_1, &[]).await;
    let mut closed = make_task_row("browse-closed", USER_1);
    closed.status = TaskStatus::Closed;
    moatless_tasks::task::test_support::seed_task(&pool, &closed)
        .await
        .expect("seed task");

    let dao = conversations_over(&pool);
    let viewer = ScopeContext {
        user_id: ADMIN.to_string(),
        is_admin: true,
    };
    let owner = || Some(USER_1.to_string());

    let open_only = dao
        .list_browsable(
            &viewer,
            &ConversationBrowse::new(owner(), None, false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(browsed_ids(&open_only), vec!["browse-open"]);

    let with_closed = dao
        .list_browsable(
            &viewer,
            &ConversationBrowse::new(owner(), None, true).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&with_closed),
        vec!["browse-closed", "browse-open"],
        "asked for them, the closed half arrives alongside the open one"
    );
}

/// A viewer who is not an administrator browses what they may read, which is
/// what makes the filter safe to serve to everyone: the read rule is untouched.
#[sqlx::test]
async fn a_browse_by_a_non_administrator_reaches_only_what_they_may_read(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    seed_browsable(&pool, "browse-private", USER_1, &["backend"]).await;
    let mut public = make_task_row("browse-public", USER_1);
    public.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &public)
        .await
        .expect("seed task");
    tag_with(&pool, "browse-public", &["backend"]).await;

    let browsed = conversations_over(&pool)
        .list_browsable(
            &ScopeContext {
                user_id: USER_2.to_string(),
                is_admin: false,
            },
            &ConversationBrowse::new(None, Some("backend".to_string()), false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&browsed),
        vec!["browse-public"],
        "somebody else's private Task is not reachable by tagging it"
    );
}

fn browsed_ids(conversations: &[crate::dao::Conversation]) -> Vec<String> {
    let mut ids: Vec<String> = conversations
        .iter()
        .map(|conversation| conversation.task_id.clone())
        .collect();
    ids.sort();
    ids
}
