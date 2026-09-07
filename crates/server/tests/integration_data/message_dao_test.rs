#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for `message_dao`.

use crate::common;

use moatless_ports::task::EventSubject;
use moatless_tasks::message::dao as message_dao;
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::message::dao::PgMessageDao;
use moatless_tasks::message::models::MessageRole;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::models::TaskMessageContext;
use moatless_tasks::task_activities::mapping::ActivityKind;
use moatless_tasks::task_activities::mapping::ActivityStatus;
use moatless_tasks::task_activities::mapping::BlockContext;
use moatless_tasks::task_activities::mapping::ConversationWrite;
use moatless_tasks::task_activities::mapping::map_content;
use sqlx::PgPool;

use common::{make_message_row, make_repository_row, make_task_row};

/// Map one Message's content the way the write path does: text onto the row,
/// each call and each outcome onto the conversation. Every test goes through
/// this same seam, rather than handing `append` a block list directly.
fn conversation_write(
    uuid: &str,
    message_type: MessageType,
    content: &serde_json::Value,
) -> ConversationWrite {
    map_content(
        BlockContext {
            task_id: "task-1",
            message_uuid: uuid,
            parent_uuid: None,
            message_type,
            block_index: 0,
        },
        content,
    )
}

/// Seed a repo + task so message FKs work.
async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    moatless_workspaces::repository::dao::insert(pool, &repo)
        .await
        .expect("seed repo");

    let task = make_task_row(task_id, user);
    moatless_tasks::task::dao::insert_task(pool, &task)
        .await
        .expect("seed task");
}

#[sqlx::test]
async fn append_and_get_all(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "msg-uuid-1");
    let msg_id = message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();
    assert!(msg_id > 0);

    let (messages, contexts, blocks) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].uuid, "msg-uuid-1");
    assert!(contexts.is_empty());
    assert!(blocks.is_empty());
}

/// The full raw `assistant_usage` blob (including the ephemeral cache-write
/// breakdown that the typed columns don't capture) round-trips through the
/// `task_messages` column. This is the data cost is derived from when the
/// transport reports no `total_cost_usd` (tmux).
#[sqlx::test]
async fn append_persists_assistant_usage(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let usage = serde_json::json!({
        "input_tokens": 10,
        "output_tokens": 197,
        "cache_creation_input_tokens": 3980,
        "cache_read_input_tokens": 18111,
        "cache_creation": {
            "ephemeral_1h_input_tokens": 3980,
            "ephemeral_5m_input_tokens": 0
        },
        "server_tool_use": { "web_search_requests": 0, "web_fetch_requests": 0 },
        "service_tier": "standard"
    });

    let mut msg = make_message_row("task-1", "usage-msg-1");
    msg.assistant_message_model = Some("claude-haiku-4-5-20251001".to_string());
    msg.assistant_usage = Some(usage.clone());

    message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    let stored = &messages[0];
    assert_eq!(
        stored.assistant_message_model.as_deref(),
        Some("claude-haiku-4-5-20251001")
    );
    assert_eq!(stored.assistant_usage.as_ref(), Some(&usage));
    // The ephemeral cache-write breakdown (absent from the typed columns) is
    // preserved for downstream cost derivation.
    assert_eq!(
        stored
            .assistant_usage
            .as_ref()
            .and_then(|u| u.get("cache_creation"))
            .and_then(|c| c.get("ephemeral_1h_input_tokens"))
            .and_then(serde_json::Value::as_i64),
        Some(3980)
    );
}

/// Codex reports token usage only on the terminal result — its assistant
/// message row is inserted with no usage. `backfill_last_assistant_usage` fills
/// the most recent assistant row (guarded by `assistant_usage IS NULL`), so the
/// turn's tokens + raw usage blob (incl. `reasoning_output_tokens`) land on the
/// message. A second call is a no-op because the row now has usage.
#[sqlx::test]
async fn backfill_fills_last_assistant_usage_once(pool: PgPool) {
    use moatless_tasks::message::models::MessageRole;
    use moatless_tasks::message::models::MessageType;

    seed_task(&pool, "task-1", "user-1").await;

    // Assistant reply inserted WITHOUT usage (the Codex shape).
    let mut assistant = make_message_row("task-1", "codex-assistant-1");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    assistant.assistant_message_model = Some("gpt-5.5".to_string());
    message_dao::append(&pool, &assistant, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let usage = serde_json::json!({
        "input_tokens": 10943,
        "output_tokens": 5,
        "cache_read_input_tokens": 9088,
        "cache_creation_input_tokens": serde_json::Value::Null,
        "reasoning_output_tokens": 128
    });

    let updated = message_dao::backfill_last_assistant_usage(
        &pool,
        "task-1",
        Some(10943),
        Some(5),
        None,
        Some(9088),
        Some(&usage),
    )
    .await
    .unwrap();
    assert!(updated, "expected the assistant row to be backfilled");

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let stored = messages
        .iter()
        .find(|m| m.uuid == "codex-assistant-1")
        .expect("assistant row present");
    assert_eq!(stored.assistant_usage_input_tokens, Some(10943));
    assert_eq!(stored.assistant_usage_output_tokens, Some(5));
    assert_eq!(stored.assistant_usage_cache_read_input_tokens, Some(9088));
    assert_eq!(stored.assistant_message_model.as_deref(), Some("gpt-5.5"));
    assert_eq!(stored.assistant_usage.as_ref(), Some(&usage));
    // Codex reasoning tokens survive in the raw blob.
    assert_eq!(
        stored
            .assistant_usage
            .as_ref()
            .and_then(|u| u.get("reasoning_output_tokens"))
            .and_then(serde_json::Value::as_i64),
        Some(128)
    );

    // Idempotent: the row now has usage, so a second backfill touches nothing.
    let again = message_dao::backfill_last_assistant_usage(
        &pool,
        "task-1",
        Some(1),
        Some(1),
        None,
        Some(1),
        Some(&serde_json::json!({"input_tokens": 1})),
    )
    .await
    .unwrap();
    assert!(
        !again,
        "second backfill must be a no-op (usage already set)"
    );
}

/// Claude reports usage inline, but one API response can span several rows when
/// the CLI interleaves other traffic between its content-block lines — and the
/// repeats deliberately carry none, so the tokens are counted once. The backfill
/// must leave those alone: writing the run's cumulative total onto a repeat
/// would count the same tokens a second time.
#[sqlx::test]
async fn backfill_skips_tasks_that_already_report_usage(pool: PgPool) {
    use moatless_tasks::message::models::MessageRole;
    use moatless_tasks::message::models::MessageType;

    seed_task(&pool, "task-1", "user-1").await;

    // First row of the response — carries the per-message usage.
    let mut first = make_message_row("task-1", "claude-assistant-1");
    first.message_type = MessageType::Assistant;
    first.message_role = MessageRole::Assistant;
    first.assistant_message_id = Some("msg_split".to_string());
    first.assistant_usage_input_tokens = Some(10);
    first.assistant_usage_output_tokens = Some(6);
    first.assistant_usage = Some(serde_json::json!({"input_tokens": 10, "output_tokens": 6}));
    message_dao::append(&pool, &first, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // Repeat of the same API message after interleaved traffic — no usage.
    let mut repeat = make_message_row("task-1", "claude-assistant-2");
    repeat.message_type = MessageType::Assistant;
    repeat.message_role = MessageRole::Assistant;
    repeat.assistant_message_id = Some("msg_split".to_string());
    message_dao::append(&pool, &repeat, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let updated = message_dao::backfill_last_assistant_usage(
        &pool,
        "task-1",
        Some(20),
        Some(1050),
        None,
        Some(37987),
        Some(&serde_json::json!({"input_tokens": 20, "output_tokens": 1050})),
    )
    .await
    .unwrap();
    assert!(
        !updated,
        "the task already reports per-message usage; the repeat must stay empty"
    );

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let repeat_row = messages
        .iter()
        .find(|m| m.uuid == "claude-assistant-2")
        .expect("repeat row present");
    assert_eq!(repeat_row.assistant_usage_output_tokens, None);
    assert_eq!(repeat_row.assistant_usage, None);
}

#[sqlx::test]
async fn append_with_contexts_and_activities(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let mut msg = make_message_row("task-1", "msg-uuid-2");
    msg.message_type = MessageType::Assistant;
    msg.message_role = MessageRole::Assistant;
    let contexts = vec![
        TaskMessageContext {
            id: 0,
            task_message_id: 0, // ignored, set by append
            context_type: "file".to_string(),
            reference_id: "ref-1".to_string(),
            data: serde_json::json!({"path": "/test.ts"}),
            created_at: None,
        },
        TaskMessageContext {
            id: 0,
            task_message_id: 0,
            context_type: "url".to_string(),
            reference_id: "ref-2".to_string(),
            data: serde_json::json!({"url": "https://example.com"}),
            created_at: None,
        },
    ];
    // What the agent said, and the one thing it did — the split the write path
    // makes: the text goes on the row, the call becomes an activity.
    let conversation = map_content(
        BlockContext {
            task_id: "task-1",
            message_uuid: "msg-uuid-2",
            parent_uuid: None,
            message_type: msg.message_type,
            block_index: 0,
        },
        &serde_json::json!([
            { "type": "text", "text": "Hello world" },
            { "type": "tool_use", "id": "tu-1", "name": "Bash", "input": {} },
        ]),
    );

    message_dao::append(&pool, &msg, &contexts, &conversation)
        .await
        .unwrap();

    let (messages, ctx_rows, activities) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].message_text.as_deref(), Some("Hello world"));
    assert_eq!(ctx_rows.len(), 2);
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].kind, ActivityKind::ToolCall);
    assert_eq!(activities[0].tool_use_id.as_deref(), Some("tu-1"));
    assert_eq!(activities[0].status, ActivityStatus::InProgress);
}

#[sqlx::test]
async fn append_upsert_on_conflict(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "msg-uuid-dup");
    message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // Re-append with same uuid — should update, not duplicate
    let mut msg2 = make_message_row("task-1", "msg-uuid-dup");
    msg2.message_text = Some("updated".to_string());
    message_dao::append(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].message_text.as_deref(), Some("updated"));
}

#[sqlx::test]
async fn get_pending_user(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Two pending user messages (no agent_session_id)
    let msg1 = make_message_row("task-1", "pending-1");
    message_dao::append(&pool, &msg1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let msg2 = make_message_row("task-1", "pending-2");
    message_dao::append(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // One acknowledged message
    let mut msg3 = make_message_row("task-1", "acked-1");
    msg3.agent_session_id = Some("sess-1".to_string());
    message_dao::append(&pool, &msg3, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let pending = message_dao::get_pending_user(&pool, "task-1")
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
}

#[sqlx::test]
async fn set_session_id_also_sets_sent_at(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Two pending user messages (no agent_session_id, no sent_at)
    let msg1 = make_message_row("task-1", "sess-msg-1");
    message_dao::append(&pool, &msg1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let msg2 = make_message_row("task-1", "sess-msg-2");
    message_dao::append(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // Simulate session_init: sets session_id, agent_session_id, and sent_at
    let count = message_dao::set_session_id_for_task(&pool, "task-1", "session-abc")
        .await
        .unwrap();
    assert_eq!(count, 2);

    // Messages should no longer be pending
    let pending = message_dao::get_pending_user(&pool, "task-1")
        .await
        .unwrap();
    assert!(pending.is_empty());

    // Verify sent_at is set on all messages
    let (all_msgs, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    for msg in &all_msgs {
        assert!(
            msg.sent_at.is_some(),
            "sent_at should be set after set_session_id_for_task"
        );
        assert_eq!(msg.session_id.as_deref(), Some("session-abc"));
        assert_eq!(msg.agent_session_id.as_deref(), Some("session-abc"));
    }
}

/// The fork copy inserts through `message_columns!()`, so a column absent from
/// that list is dropped without failing to compile. `sent_at` absent left every
/// copied Message claimed by `agent_session_id` and unsent by `sent_at`, which
/// reads as queued forever while no dispatch will ever pick it up again.
#[sqlx::test]
async fn a_copied_message_carries_the_sent_at_that_claimed_it(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let fork = make_task_row("task-2", "user-1");
    moatless_tasks::task::dao::insert_task(&pool, &fork)
        .await
        .expect("seed fork task");

    let msg = make_message_row("task-1", "forked-msg-1");
    message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();
    message_dao::acknowledge_messages(
        &pool,
        "task-1",
        &["forked-msg-1".to_string()],
        "session-abc",
        1,
    )
    .await
    .unwrap();

    moatless_tasks::message::fork::copy_conversation(&pool, "task-1", "task-2", None)
        .await
        .unwrap();

    let (copied, _, _) = message_dao::get_all(&pool, "task-2").await.unwrap();
    let copied = copied.first().expect("the acknowledged Message is copied");
    assert!(
        copied.sent_at.is_some(),
        "a copy of a Message the parent had already sent must not read as queued"
    );
    assert_eq!(copied.agent_session_id.as_deref(), Some("session-abc"));
    assert!(
        message_dao::get_pending_user(&pool, "task-2")
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test]
async fn get_last_result(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Insert two result messages with different timestamps
    let mut msg1 = make_message_row("task-1", "result-1");
    msg1.message_type = MessageType::Result;
    msg1.timestamp = "2024-01-01T00:00:00.000Z".to_string();
    message_dao::append(&pool, &msg1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let mut msg2 = make_message_row("task-1", "result-2");
    msg2.message_type = MessageType::Result;
    msg2.timestamp = "2024-01-02T00:00:00.000Z".to_string();
    message_dao::append(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let last = message_dao::get_last_result(&pool, "task-1").await.unwrap();
    assert!(last.is_some());
    assert_eq!(last.unwrap().uuid, "result-2");
}

#[sqlx::test]
async fn get_last_assistant_message_picks_latest_assistant(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Earlier assistant row.
    let mut asst1 = make_message_row("task-1", "asst-1");
    asst1.message_type = MessageType::Assistant;
    asst1.message_role = MessageRole::Assistant;
    asst1.timestamp = "2024-01-01T00:00:00.000Z".to_string();
    asst1.message_text = Some("first".to_string());
    message_dao::append(&pool, &asst1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // Result row in between — must NOT be selected.
    let mut result = make_message_row("task-1", "result-1");
    result.message_type = MessageType::Result;
    result.timestamp = "2024-01-02T00:00:00.000Z".to_string();
    message_dao::append(&pool, &result, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // Most recent assistant row.
    let mut asst2 = make_message_row("task-1", "asst-2");
    asst2.message_type = MessageType::Assistant;
    asst2.message_role = MessageRole::Assistant;
    asst2.timestamp = "2024-01-03T00:00:00.000Z".to_string();
    asst2.message_text = Some("latest".to_string());
    message_dao::append(&pool, &asst2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let last = message_dao::get_last_assistant_message(&pool, "task-1")
        .await
        .unwrap();
    let last = last.expect("expected an assistant row");
    assert_eq!(last.uuid, "asst-2");
    assert_eq!(last.message_type, MessageType::Assistant);
}

#[sqlx::test]
async fn get_last_assistant_message_returns_none_when_absent(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Only a non-assistant row exists.
    let mut result = make_message_row("task-1", "result-1");
    result.message_type = MessageType::Result;
    message_dao::append(&pool, &result, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let last = message_dao::get_last_assistant_message(&pool, "task-1")
        .await
        .unwrap();
    assert!(last.is_none());
}

#[sqlx::test]
async fn set_external_source_metadata_writes_all_columns_and_is_idempotent(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "tag-msg-1");
    message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let repo = PgMessageDao(pool.clone());
    let updated = repo
        .set_external_source_metadata(
            "tag-msg-1",
            "github:pr:opened:42",
            "github_pr",
            Some("foo/bar"),
        )
        .await
        .unwrap();
    assert!(updated);

    let (rows, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let row = rows.iter().find(|m| m.uuid == "tag-msg-1").unwrap();
    assert_eq!(
        row.external_event_id.as_deref(),
        Some("github:pr:opened:42")
    );
    assert_eq!(row.external_adapter.as_deref(), Some("github_pr"));
    assert_eq!(row.external_source_name.as_deref(), Some("foo/bar"));

    // Second call must be a no-op (idempotency guard on external_event_id IS NULL).
    let again = repo
        .set_external_source_metadata(
            "tag-msg-1",
            "github:pr:opened:99",
            "github_pr",
            Some("other/repo"),
        )
        .await
        .unwrap();
    assert!(!again);

    let (rows2, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let row2 = rows2.iter().find(|m| m.uuid == "tag-msg-1").unwrap();
    assert_eq!(
        row2.external_event_id.as_deref(),
        Some("github:pr:opened:42")
    );
    assert_eq!(row2.external_source_name.as_deref(), Some("foo/bar"));
}

/// A comment id is allocated once by the provider, so the task that wrote the
/// comment is found without being told where to look.
#[sqlx::test]
async fn a_comment_id_names_its_author_across_every_subject(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let task2 = make_task_row("task-2", "user-2");
    moatless_tasks::task::dao::insert_task(&pool, &task2)
        .await
        .unwrap();

    let msg1 = make_message_row("task-1", "tag-origin-1");
    message_dao::append(&pool, &msg1, &[], &ConversationWrite::default())
        .await
        .unwrap();
    let msg2 = make_message_row("task-2", "tag-origin-2");
    message_dao::append(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let repo = PgMessageDao(pool.clone());
    repo.set_external_source_metadata(
        "tag-origin-1",
        "github:ic:555",
        "github_pr",
        Some("foo/bar"),
    )
    .await
    .unwrap();
    repo.set_external_source_metadata(
        "tag-origin-2",
        "github:ic:777",
        "github_pr",
        Some("foo/bar"),
    )
    .await
    .unwrap();

    let found = repo
        .find_task_id_by_any_external_event_id(&["github:ic:555".to_string(), "555".to_string()])
        .await
        .unwrap();
    assert_eq!(found.as_deref(), Some("task-1"));
}

/// Two repositories, one pull request number, two tasks — and one id string
/// recorded against both.
///
/// This is the shape of a row written before PR-level ids named their
/// repository: `github:pr:opened:607` is a pull request in every repository
/// that has a #607, so the task working `soaplabs/soap-specs#607` holds the
/// very id that `soaplabs/moatless#607` would arrive under. Migration `0148`
/// rewrites the rows it can place; this is what answers for the rest, and for
/// any provider that numbers per subject. Asked on the subject, the lookup
/// names the task actually on that pull request — the caller binds the answer,
/// so naming the other one attaches a task to a pull request it never touched.
#[sqlx::test]
async fn a_pr_number_keyed_id_names_only_the_task_on_that_repositorys_pull_request(pool: PgPool) {
    seed_task(&pool, "task-specs", "user-1").await;
    let moatless_task = make_task_row("task-moatless", "user-2");
    moatless_tasks::task::dao::insert_task(&pool, &moatless_task)
        .await
        .unwrap();

    for (task_id, uuid, subject_key) in [
        ("task-specs", "specs-607", "soaplabs/soap-specs#607"),
        ("task-moatless", "moatless-607", "soaplabs/moatless#607"),
    ] {
        let mut msg = make_message_row(task_id, uuid);
        msg.external_event_id = Some("github:pr:opened:607".to_string());
        msg.external_adapter = Some("github_pr".to_string());
        msg.external_subject_key = Some(subject_key.to_string());
        message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
            .await
            .unwrap();
    }

    let repo = PgMessageDao(pool.clone());
    let ids = ["github:pr:opened:607".to_string()];

    assert_eq!(
        repo.find_task_id_by_any_external_event_id_on_subject(
            &ids,
            EventSubject {
                adapter: "github_pr",
                source_name: "soaplabs/moatless",
                subject_key: "soaplabs/moatless#607",
            },
        )
        .await
        .unwrap()
        .as_deref(),
        Some("task-moatless"),
    );

    assert_eq!(
        repo.find_task_id_by_any_external_event_id_on_subject(
            &ids,
            EventSubject {
                adapter: "github_pr",
                source_name: "soaplabs/soap-specs",
                subject_key: "soaplabs/soap-specs#607",
            },
        )
        .await
        .unwrap()
        .as_deref(),
        Some("task-specs"),
    );

    // A third repository's #607: neither task wrote it, and the shared id must
    // not offer one up.
    assert_eq!(
        repo.find_task_id_by_any_external_event_id_on_subject(
            &ids,
            EventSubject {
                adapter: "github_pr",
                source_name: "soaplabs/soap-backend",
                subject_key: "soaplabs/soap-backend#607",
            },
        )
        .await
        .unwrap(),
        None,
    );
}

/// The same shape on the outbound side: a task that opened `#607` in one
/// repository claimed an id every other repository's `#607` would arrive under.
///
/// `task_outbound_events` is keyed on the id alone, because a `moat` command
/// holds nothing else, so `source_name` is the only column that tells such
/// claims apart.
#[sqlx::test]
async fn an_outbound_claim_names_its_task_only_at_the_source_that_made_it(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    repo.record_external_event(
        "task-1",
        "github:pr:opened:607",
        "github_pr",
        Some("soaplabs/soap-specs"),
    )
    .await
    .unwrap();

    let ids = ["github:pr:opened:607".to_string()];

    assert_eq!(
        repo.find_task_id_by_any_external_event_id_on_subject(
            &ids,
            EventSubject {
                adapter: "github_pr",
                source_name: "soaplabs/soap-specs",
                subject_key: "soaplabs/soap-specs#607",
            },
        )
        .await
        .unwrap()
        .as_deref(),
        Some("task-1"),
    );

    assert_eq!(
        repo.find_task_id_by_any_external_event_id_on_subject(
            &ids,
            EventSubject {
                adapter: "github_pr",
                source_name: "soaplabs/moatless",
                subject_key: "soaplabs/moatless#607",
            },
        )
        .await
        .unwrap(),
        None,
    );
}

/// A row that records no subject cannot be shown to have happened here.
///
/// Rows predating the subject columns, and the `unknown` provider the
/// activity backfill wrote, are the ones this covers. The answer binds a task
/// to a subject, so an unplaceable row must yield nothing rather than a guess:
/// the cost is a binding not auto-attached, against delivering a pull
/// request's whole event stream into an unrelated task.
#[sqlx::test]
async fn an_event_that_records_no_subject_names_no_task(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    moatless_tasks::task::dao::insert_task(&pool, &make_task_row("task-2", "user-2"))
        .await
        .expect("seed second task in the same repository");
    let repo = PgMessageDao(pool.clone());

    let mut msg = make_message_row("task-1", "no-subject");
    msg.external_event_id = Some("github:pr:opened:607".to_string());
    msg.external_adapter = Some("github_pr".to_string());
    msg.external_subject_key = None;
    message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();

    repo.record_external_event("task-2", "github:pr:opened:608", "github_pr", None)
        .await
        .unwrap();

    for id in ["github:pr:opened:607", "github:pr:opened:608"] {
        assert_eq!(
            repo.find_task_id_by_any_external_event_id_on_subject(
                &[id.to_string()],
                EventSubject {
                    adapter: "github_pr",
                    source_name: "soaplabs/moatless",
                    subject_key: "soaplabs/moatless#607",
                },
            )
            .await
            .unwrap(),
            None,
        );
    }
}

/// A claim recorded by the id alone answers both readers.
///
/// This is the shape a `moat` command in a sandbox has: it knows the task and
/// the id, and has no `tool_use_id` to hang an activity tag on. Without it the
/// dedup gate would deliver the task its own comment back.
#[sqlx::test]
async fn an_outbound_event_by_id_alone_suppresses_the_echo_and_names_its_task(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    repo.record_external_event("task-1", "github:rc:987", "github_pr", Some("foo/bar"))
        .await
        .unwrap();

    // Layer 2 of self-echo suppression: this task already made this comment.
    assert!(
        repo.has_any_external_event_id("task-1", &["github:rc:987".to_string()])
            .await
            .unwrap()
    );
    // Provenance routing: the event belongs to the task that claimed it.
    assert_eq!(
        repo.find_task_id_by_any_external_event_id(&["github:rc:987".to_string()])
            .await
            .unwrap()
            .as_deref(),
        Some("task-1")
    );

    // A different task's dedup gate is unaffected — suppression is per task.
    let task2 = make_task_row("task-2", "user-2");
    moatless_tasks::task::dao::insert_task(&pool, &task2)
        .await
        .unwrap();
    assert!(
        !repo
            .has_any_external_event_id("task-2", &["github:rc:987".to_string()])
            .await
            .unwrap()
    );
}

/// Re-claiming an id already held changes nothing.
///
/// The claim path is at-least-once — a retried CLI call, a redelivered
/// request — and an id names one event, so the first claimant keeps it.
#[sqlx::test]
async fn re_recording_an_id_leaves_the_first_row_standing(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let task2 = make_task_row("task-2", "user-2");
    moatless_tasks::task::dao::insert_task(&pool, &task2)
        .await
        .unwrap();
    let repo = PgMessageDao(pool.clone());

    repo.record_external_event("task-1", "github:ic:555", "github_pr", None)
        .await
        .unwrap();
    repo.record_external_event("task-2", "github:ic:555", "github_pr", None)
        .await
        .unwrap();

    assert_eq!(
        repo.find_task_id_by_any_external_event_id(&["github:ic:555".to_string()])
            .await
            .unwrap()
            .as_deref(),
        Some("task-1")
    );
}

#[sqlx::test]
async fn find_tool_call_returns_parent_message_uuid(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Assistant row containing the tool_use block.
    let mut assistant = make_message_row("task-1", "assistant-uuid");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    assistant.user_type = "assistant".to_string();
    let conversation = conversation_write(
        "assistant-uuid",
        assistant.message_type,
        &serde_json::json!([
            { "type": "text", "text": "running" },
            {
                "type": "tool_use",
                "id": "tu-123",
                "name": "Bash",
                "input": { "command": "echo hi" }
            }
        ]),
    );
    message_dao::append(&pool, &assistant, &[], &conversation)
        .await
        .unwrap();

    let repo = PgMessageDao(pool.clone());

    // The result arrives on a later source event and settles the call in place.
    repo.append_activities(
        "task-1",
        "assistant-uuid",
        &conversation_write(
            "system-uuid",
            MessageType::User,
            &serde_json::json!([
                {
                    "type": "tool_result",
                    "tool_use_id": "tu-123",
                    "content": "hi",
                    "is_error": false
                }
            ]),
        ),
    )
    .await
    .unwrap();

    let found = repo
        .find_tool_call("task-1", "tu-123")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.tool_use_id, "tu-123");
    assert_eq!(found.tool_name, "Bash");
    assert_eq!(found.parent_message_uuid, "assistant-uuid");
    assert_eq!(found.result_content.as_deref(), Some("hi"));
}

/// Someone who writes in the chat box instead of using the panel answers the
/// question too. Nothing on that message's way in settles the call, so the
/// acknowledgement names it and the call comes back here — with the resolved row
/// beside it that closes the panel.
#[sqlx::test]
async fn a_plain_message_settles_the_question_it_answered(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let mut asking = make_message_row("task-1", "m-ask");
    asking.message_type = MessageType::Assistant;
    asking.message_role = MessageRole::Assistant;
    message_dao::append(
        &pool,
        &asking,
        &[],
        &conversation_write(
            "m-ask",
            MessageType::Assistant,
            &serde_json::json!([
                { "type": "tool_use", "id": "tu-1", "name": "AskUserQuestion", "input": {} }
            ]),
        ),
    )
    .await
    .unwrap();

    let repo = PgMessageDao(pool.clone());
    let mut written = make_message_row("task-1", "m-written");
    written.message_text = Some("the first one".to_string());
    repo.append(&written, &[], &ConversationWrite::default())
        .await
        .unwrap();

    moatless_tasks::message::send::settle_answered_prompt(&repo, "task-1", "tu-1", "m-written")
        .await;

    let call = repo
        .find_tool_call("task-1", "tu-1")
        .await
        .unwrap()
        .expect("the call is still there");
    assert_eq!(
        call.is_error,
        Some(false),
        "the call came back, so it is no longer in progress"
    );

    let ((_, _, activities), _) = repo
        .get_message_for_assembly("task-1", "m-written")
        .await
        .unwrap()
        .expect("the message that answered loads");
    let resolved = activities
        .iter()
        .find(|activity| activity.kind == ActivityKind::UserInputResolved)
        .expect("the answer wrote a resolved row");
    assert_eq!(
        resolved.payload["requestId"], "tu-1",
        "the panel closes on a row naming the call"
    );
    assert_eq!(resolved.payload["answers"]["response"], "the first one");
}

/// Seam 2 (loader): a change names the row it is about, so the loader reads
/// that row and the activities written on it — including the ones a later
/// source event settled.
///
/// The loader does not need to resolve a folded `tool_result` row back to the
/// assistant Message it answered: the change already names the right row
/// directly.
#[sqlx::test]
async fn get_message_for_assembly_loads_the_named_message_with_its_activities(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // A turn that said nothing and called one tool: no text, so no Message row
    // beyond the skeleton the call hangs on.
    let mut assistant = make_message_row("task-1", "a1");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    message_dao::append(
        &pool,
        &assistant,
        &[],
        &conversation_write(
            "a1",
            MessageType::Assistant,
            &serde_json::json!([
                { "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } }
            ]),
        ),
    )
    .await
    .unwrap();

    let repo = PgMessageDao(pool.clone());
    repo.append_activities(
        "task-1",
        "a1",
        &conversation_write(
            "sys1",
            MessageType::User,
            &serde_json::json!([
                { "type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": false }
            ]),
        ),
    )
    .await
    .unwrap();

    let ((messages, _ctx, activities), target) = repo
        .get_message_for_assembly("task-1", "a1")
        .await
        .unwrap()
        .expect("the named Message loads");
    assert_eq!(target, "a1");
    let uuids: Vec<&str> = messages.iter().map(|m| m.uuid.as_str()).collect();
    assert_eq!(
        uuids,
        vec!["a1"],
        "the named row, and nothing dragged along"
    );
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].status, ActivityStatus::Completed);
    assert_eq!(
        activities[0].result().and_then(serde_json::Value::as_str),
        Some("ok"),
        "the outcome the later event carried is on the call's own activity"
    );

    // A uuid this Task never saw resolves to nothing.
    assert!(
        repo.get_message_for_assembly("task-1", "sys1")
            .await
            .unwrap()
            .is_none()
    );
}

async fn seed_acknowledged_turn(pool: &PgPool, task_id: &str, uuid: &str, turn_number: i32) {
    let mut msg = make_message_row(task_id, uuid);
    msg.session_id = Some(format!("session-{turn_number}"));
    msg.agent_session_id = Some(format!("session-{turn_number}"));
    msg.turn_number = Some(turn_number);
    msg.sent_at = Some(chrono::Utc::now());
    message_dao::append(pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();
}

async fn seed_turn_event(
    pool: &PgPool,
    task_id: &str,
    event_type: &str,
    sub_type: &str,
    turn_number: i32,
) {
    sqlx::query(
        r#"INSERT INTO task_events
               (event_id, task_id, type, sub_type, turn_number, agent_type, payload, timestamp)
           VALUES ($1, $2, $3, $4, $5, 'claude-code', '{}'::jsonb, '')"#,
    )
    .bind(format!("{task_id}-{event_type}-{sub_type}-{turn_number}"))
    .bind(task_id)
    .bind(event_type)
    .bind(sub_type)
    .bind(turn_number)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn find_interrupted_turn_returns_acknowledged_unfinished_turn(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "interrupted-user", 1).await;

    let repo = PgMessageDao(pool.clone());
    let interrupted = repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .expect("expected interrupted turn");

    assert_eq!(interrupted.turn_number, 1);
    assert_eq!(interrupted.session_id, "session-1");
}

#[sqlx::test]
async fn find_interrupted_turn_suppresses_terminal_agent_event(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "agent-completed-user", 1).await;
    seed_turn_event(&pool, "task-1", "agent", "completed", 1).await;

    let repo = PgMessageDao(pool.clone());
    assert!(
        repo.find_interrupted_turn("task-1")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn find_interrupted_turn_suppresses_terminal_task_turn_event(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "turn-completed-user", 1).await;
    seed_turn_event(&pool, "task-1", "taskTurn", "completed", 1).await;

    let repo = PgMessageDao(pool.clone());
    assert!(
        repo.find_interrupted_turn("task-1")
            .await
            .unwrap()
            .is_none()
    );
}

/// `taskTurn.turn_interrupted` is the marker a pod leaves when it is torn down
/// mid-turn — a node drain, spot reclaim, or eviction. It must NOT suppress
/// recovery: it is precisely the signal that the replacement pod has a turn to
/// resume. Adding it to the terminal list would put the sandbox back to
/// publishing a turn nobody can resume, which is the bug this pair of behaviors
/// exists to prevent.
#[sqlx::test]
async fn find_interrupted_turn_survives_the_interrupted_marker(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "turn-interrupted-user", 1).await;
    seed_turn_event(&pool, "task-1", "taskTurn", "turn_interrupted", 1).await;

    let repo = PgMessageDao(pool.clone());
    let interrupted = repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .expect("an interrupted turn is recoverable, not terminal");

    assert_eq!(interrupted.turn_number, 1);
    assert_eq!(interrupted.session_id, "session-1");
}

#[sqlx::test]
async fn find_interrupted_turn_suppresses_explicit_sandbox_stop(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "sandbox-stopped-user", 1).await;
    seed_turn_event(&pool, "task-1", "sandbox", "stopped", 1).await;

    let repo = PgMessageDao(pool.clone());
    assert!(
        repo.find_interrupted_turn("task-1")
            .await
            .unwrap()
            .is_none()
    );
}

/// Build an `assistant` message row with a controlled `stop_reason`,
/// `turn_number`, and timestamp so ordering/scoping is deterministic.
fn make_assistant_row(
    task_id: &str,
    uuid: &str,
    stop_reason: Option<&str>,
    turn_number: Option<i32>,
    ts: &str,
) -> moatless_tasks::message::models::TaskMessage {
    let mut msg = make_message_row(task_id, uuid);
    msg.message_type = MessageType::Assistant;
    msg.message_role = MessageRole::Assistant;
    msg.assistant_stop_reason = stop_reason.map(ToString::to_string);
    msg.turn_number = turn_number;
    msg.timestamp = ts.to_string();
    msg
}

#[sqlx::test]
async fn last_message_is_completed_reply_detects_final_assistant(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    // No messages yet — not a completed reply.
    assert!(
        !repo
            .last_message_is_completed_reply("task-1", Some(1))
            .await
            .unwrap()
    );

    // A user message alone is a genuine orphan (agent never replied).
    let mut user = make_message_row("task-1", "u-1");
    user.turn_number = Some(1);
    // Stamped explicitly: the ordering under test is `timestamp DESC`, and the
    // fixture's default is `now`, which would sort this message *after* the
    // reply below and make the assertion depend on the date the suite is run.
    user.timestamp = "2026-07-14T11:47:00.000Z".to_string();
    message_dao::append(&pool, &user, &[], &ConversationWrite::default())
        .await
        .unwrap();
    assert!(
        !repo
            .last_message_is_completed_reply("task-1", Some(1))
            .await
            .unwrap()
    );

    // An assistant message that finished its turn is a completed reply.
    let done = make_assistant_row(
        "task-1",
        "a-1",
        Some("end_turn"),
        Some(1),
        "2026-07-14T11:47:30.000Z",
    );
    message_dao::append(&pool, &done, &[], &ConversationWrite::default())
        .await
        .unwrap();
    assert!(
        repo.last_message_is_completed_reply("task-1", Some(1))
            .await
            .unwrap()
    );
    // A None scope falls back to the task-wide last message.
    assert!(
        repo.last_message_is_completed_reply("task-1", None)
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn last_message_is_completed_reply_ignores_mid_tool_interruption(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    // An assistant message that stopped to call a tool is NOT a final reply —
    // the turn was cut off mid-tool and stays a genuine orphan.
    let tool = make_assistant_row(
        "task-1",
        "a-1",
        Some("tool_use"),
        Some(1),
        "2026-07-14T11:47:30.000Z",
    );
    message_dao::append(&pool, &tool, &[], &ConversationWrite::default())
        .await
        .unwrap();
    assert!(
        !repo
            .last_message_is_completed_reply("task-1", Some(1))
            .await
            .unwrap()
    );

    // An assistant message with no stop_reason is likewise not treated as final.
    let unknown = make_assistant_row("task-1", "a-2", None, Some(1), "2026-07-14T11:47:31.000Z");
    message_dao::append(&pool, &unknown, &[], &ConversationWrite::default())
        .await
        .unwrap();
    assert!(
        !repo
            .last_message_is_completed_reply("task-1", Some(1))
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn last_message_is_completed_reply_is_scoped_to_turn(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    // Turn 1 was cut off mid-tool (no final reply); turn 2 later finished.
    let turn1_tool = make_assistant_row(
        "task-1",
        "a-1",
        Some("tool_use"),
        Some(1),
        "2026-07-14T11:00:00.000Z",
    );
    message_dao::append(&pool, &turn1_tool, &[], &ConversationWrite::default())
        .await
        .unwrap();
    let turn2_done = make_assistant_row(
        "task-1",
        "a-2",
        Some("end_turn"),
        Some(2),
        "2026-07-14T12:00:00.000Z",
    );
    message_dao::append(&pool, &turn2_done, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // A stale open turn 1 must NOT be closed as completed on the strength of
    // turn 2's final reply.
    assert!(
        !repo
            .last_message_is_completed_reply("task-1", Some(1))
            .await
            .unwrap()
    );
    // Turn 2 did finish.
    assert!(
        repo.last_message_is_completed_reply("task-1", Some(2))
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn last_message_is_completed_reply_detects_coalesced_later_reply(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    // Turn 1 was part of the same sandbox batch, but lifecycle/result events
    // were attributed to turn 2 by current-turn resolution. Because turn 1 has
    // no substantive assistant/system/result state, the reaper can close it
    // silently as completed instead of surfacing a false orphan error.
    let mut turn1_user = make_message_row("task-1", "u-1");
    turn1_user.message_type = MessageType::User;
    turn1_user.message_role = MessageRole::User;
    turn1_user.turn_number = Some(1);
    turn1_user.timestamp = "2026-07-14T11:00:00.000Z".to_string();
    message_dao::append(&pool, &turn1_user, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let mut empty_placeholder = make_assistant_row(
        "task-1",
        "a-empty",
        None,
        Some(1),
        "2026-07-14T11:00:01.000Z",
    );
    // Said nothing: the placeholder a coalescing CLI leaves behind carries no
    // text at all, which is what makes it insubstantial here.
    empty_placeholder.message_text = None;
    message_dao::append(
        &pool,
        &empty_placeholder,
        &[],
        &ConversationWrite::default(),
    )
    .await
    .unwrap();

    let mut turn2_user = make_message_row("task-1", "u-2");
    turn2_user.message_type = MessageType::User;
    turn2_user.message_role = MessageRole::User;
    turn2_user.turn_number = Some(2);
    turn2_user.timestamp = "2026-07-14T11:00:02.000Z".to_string();
    message_dao::append(&pool, &turn2_user, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let turn2_done = make_assistant_row(
        "task-1",
        "a-2",
        Some("end_turn"),
        Some(2),
        "2026-07-14T11:01:00.000Z",
    );
    message_dao::append(&pool, &turn2_done, &[], &ConversationWrite::default())
        .await
        .unwrap();

    assert!(
        repo.last_message_is_completed_reply("task-1", Some(1))
            .await
            .unwrap()
    );
    assert!(
        repo.last_message_is_completed_reply("task-1", Some(2))
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn find_interrupted_turn_aggregates_sibling_user_messages(pool: PgPool) {
    // One agent run can consume several pending messages, so a turn holds all
    // of them. Recovery must replay the same joined prompt the dispatcher built,
    // not an arbitrary one of the siblings.
    seed_task(&pool, "task-1", "user-1").await;
    for (uuid, text) in [("sib-a", "first"), ("sib-b", "second")] {
        let mut msg = make_message_row("task-1", uuid);
        msg.session_id = Some("session-1".to_string());
        msg.agent_session_id = Some("session-1".to_string());
        msg.turn_number = Some(1);
        msg.sent_at = Some(chrono::Utc::now());
        msg.message_text = Some(text.to_string());
        message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
            .await
            .unwrap();
    }

    let repo = PgMessageDao(pool.clone());
    let interrupted = repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .expect("expected interrupted turn");

    assert_eq!(interrupted.turn_number, 1);
    assert_eq!(interrupted.prompt_text, "first\nsecond");
}

#[sqlx::test]
async fn find_interrupted_turn_reads_recovery_attempts_from_task_turns(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "recovering-user", 1).await;
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status, recovery_attempts, run_id) \
         VALUES ($1, 1, 'interrupted', 2, 'run-orig')",
    )
    .bind("task-1")
    .execute(&pool)
    .await
    .unwrap();

    let repo = PgMessageDao(pool.clone());
    let interrupted = repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .expect("expected interrupted turn");

    assert_eq!(interrupted.recovery_attempts, 2);
    // The turn's run_id rides along so the recovery re-drive can reuse it.
    assert_eq!(interrupted.run_id.as_deref(), Some("run-orig"));
}

/// Insert an `agent.<sub_type>` event on `turn_number` at an explicit
/// `created_at`, so a test can order recovery markers against output.
async fn seed_agent_event_at(
    pool: &PgPool,
    task_id: &str,
    sub_type: &str,
    turn_number: i32,
    created_at: &str,
) {
    sqlx::query(
        r#"INSERT INTO task_events
               (event_id, task_id, type, sub_type, turn_number, agent_type, payload, timestamp, created_at)
           VALUES ($1, $2, 'agent', $3, $4, 'claude-code', '{}'::jsonb, $5, $5::timestamptz)"#,
    )
    .bind(format!("{task_id}-agent-{sub_type}-{turn_number}-{created_at}"))
    .bind(task_id)
    .bind(sub_type)
    .bind(turn_number)
    .bind(created_at)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn turn_progressed_since_last_recovery_tracks_output_after_the_newest_recovery(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    // Nothing published yet: no progress.
    assert!(
        !repo
            .turn_progressed_since_last_recovery("task-1", 1)
            .await
            .unwrap()
    );

    // A recovery re-drive that then produced an agent.message: progress.
    seed_agent_event_at(
        &pool,
        "task-1",
        "recovery_started",
        1,
        "2026-01-01T00:00:00Z",
    )
    .await;
    seed_agent_event_at(&pool, "task-1", "message", 1, "2026-01-01T00:01:00Z").await;
    assert!(
        repo.turn_progressed_since_last_recovery("task-1", 1)
            .await
            .unwrap()
    );

    // A newer recovery re-drive with no output after it: a crash loop, no
    // progress — the earlier message predates this attempt.
    seed_agent_event_at(
        &pool,
        "task-1",
        "recovery_started",
        1,
        "2026-01-01T00:02:00Z",
    )
    .await;
    assert!(
        !repo
            .turn_progressed_since_last_recovery("task-1", 1)
            .await
            .unwrap()
    );
}
