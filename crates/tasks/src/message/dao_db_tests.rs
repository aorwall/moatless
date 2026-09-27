//! PostgreSQL tests for the message store.

use crate::activities::mapping::{
    ActivityKind, ActivityStatus, BlockContext, ConversationWrite, map_content,
};
use crate::message::dao::{MessageDao, PgMessageDao};
use crate::message::models::{MessageRole, MessageType, TaskMessageContext};
use moatless_ports::task::EventSubject;
use sqlx::PgPool;

use crate::message::test_support::{make_message_row, seeded_uuid};
use crate::task::test_support::make_task_row;
use moatless_workspaces::test_support::make_repository_row;

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
            source: uuid,
            source_block: 0,
        },
        content,
    )
}

/// Seed a repo + task so message FKs work.
async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");

    let task = make_task_row(task_id, user);
    crate::task::test_support::seed_task(pool, &task)
        .await
        .expect("seed task");
}

#[sqlx::test]
async fn append_and_get_all(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "msg-uuid-1");
    let msg_id =
        crate::message::test_support::seed_message(&pool, &msg, &[], &ConversationWrite::default())
            .await
            .unwrap();
    assert!(msg_id > 0);

    let (messages, contexts, blocks) = crate::message::test_support::messages_for(&pool, "task-1")
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].uuid, seeded_uuid("msg-uuid-1").to_string());
    assert!(contexts.is_empty());
    assert!(blocks.is_empty());
}

/// The raw `assistant_usage` blob, including the cache-write breakdown typed
/// columns miss, round-trips through a frozen row for deriving cost when the
/// transport omits `total_cost_usd` (tmux).
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

    crate::message::test_support::seed_message(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let (messages, _, _) = crate::message::test_support::messages_for(&pool, "task-1")
        .await
        .unwrap();
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
            source: "msg-uuid-2",
            source_block: 0,
        },
        &serde_json::json!([
            { "type": "text", "text": "Hello world" },
            { "type": "tool_use", "id": "tu-1", "name": "Bash", "input": {} },
        ]),
    );

    // Contexts ride on what a person sent: the user Message's inbox item.
    let asked = make_message_row("task-1", "msg-uuid-1");
    crate::message::test_support::seed_message(
        &pool,
        &asked,
        &contexts,
        &ConversationWrite::default(),
    )
    .await
    .unwrap();
    crate::message::test_support::seed_message(&pool, &msg, &[], &conversation)
        .await
        .unwrap();

    let (messages, ctx_rows, activities) =
        crate::message::test_support::messages_for(&pool, "task-1")
            .await
            .unwrap();
    assert_eq!(messages.len(), 2);
    let answer = crate::message::test_support::seeded_uuid("msg-uuid-2").to_string();
    let answer = messages
        .iter()
        .find(|m| m.uuid == answer)
        .expect("the answer");
    assert_eq!(answer.message_text.as_deref(), Some("Hello world"));
    assert_eq!(ctx_rows.len(), 2);
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].kind, ActivityKind::ToolCall);
    assert_eq!(activities[0].tool_use_id.as_deref(), Some("tu-1"));
    assert_eq!(activities[0].status, ActivityStatus::InProgress);
}

#[sqlx::test]
async fn get_pending_user(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    crate::message::test_support::queue_named(&pool, "task-1", "pending-1", "one").await;
    crate::message::test_support::queue_named(&pool, "task-1", "pending-2", "two").await;
    seed_acknowledged_message(&pool, "task-1", "acked-1", 1).await;

    let pending = crate::message::test_support::pending_user_messages(&pool, "task-1")
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
}

#[sqlx::test]
async fn get_last_result(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Insert two result messages with different timestamps
    let mut msg1 = make_message_row("task-1", "result-1");
    msg1.message_type = MessageType::Result;
    msg1.timestamp = "2024-01-01T00:00:00.000Z".to_string();
    crate::message::test_support::seed_message(&pool, &msg1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let mut msg2 = make_message_row("task-1", "result-2");
    msg2.message_type = MessageType::Result;
    msg2.timestamp = "2024-01-02T00:00:00.000Z".to_string();
    crate::message::test_support::seed_message(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let last = crate::message::test_support::last_result_message(&pool, "task-1")
        .await
        .unwrap();
    assert!(last.is_some());
    assert_eq!(last.unwrap().uuid, seeded_uuid("result-2").to_string());
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
    crate::message::test_support::seed_message(&pool, &asst1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // Result row in between — must NOT be selected.
    let mut result = make_message_row("task-1", "result-1");
    result.message_type = MessageType::Result;
    result.timestamp = "2024-01-02T00:00:00.000Z".to_string();
    crate::message::test_support::seed_message(&pool, &result, &[], &ConversationWrite::default())
        .await
        .unwrap();

    // Most recent assistant row.
    let mut asst2 = make_message_row("task-1", "asst-2");
    asst2.message_type = MessageType::Assistant;
    asst2.message_role = MessageRole::Assistant;
    asst2.timestamp = "2024-01-03T00:00:00.000Z".to_string();
    asst2.message_text = Some("latest".to_string());
    crate::message::test_support::seed_message(&pool, &asst2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let last = crate::message::test_support::last_assistant_message(&pool, "task-1")
        .await
        .unwrap();
    let last = last.expect("expected an assistant row");
    assert_eq!(last.uuid, seeded_uuid("asst-2").to_string());
    assert_eq!(last.message_type, MessageType::Assistant);
}

#[sqlx::test]
async fn get_last_assistant_message_returns_none_when_absent(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Only a non-assistant row exists.
    let mut result = make_message_row("task-1", "result-1");
    result.message_type = MessageType::Result;
    crate::message::test_support::seed_message(&pool, &result, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let last = crate::message::test_support::last_assistant_message(&pool, "task-1")
        .await
        .unwrap();
    assert!(last.is_none());
}

/// A comment id is allocated once by the provider, so the task that wrote the
/// comment is found without being told where to look.
#[sqlx::test]
async fn a_comment_id_names_its_author_across_every_subject(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let task2 = make_task_row("task-2", "user-2");
    crate::task::test_support::seed_task(&pool, &task2)
        .await
        .unwrap();

    let repo = PgMessageDao(pool.clone());
    repo.record_external_event("task-1", "github:ic:555", "github_pr", Some("foo/bar"))
        .await
        .unwrap();
    repo.record_external_event("task-2", "github:ic:777", "github_pr", Some("foo/bar"))
        .await
        .unwrap();

    let found = repo
        .find_task_id_by_any_external_event_id(&["github:ic:555".to_string(), "555".to_string()])
        .await
        .unwrap();
    assert_eq!(found.as_deref(), Some("task-1"));
}

/// Rows written before PR-level ids named their repository share one id across
/// every repo with a matching PR number.
///
/// Asked on a specific subject, the lookup must name only the task actually on
/// that repository's PR — never a same-numbered PR elsewhere.
#[sqlx::test]
async fn a_pr_number_keyed_id_names_only_the_task_on_that_repositorys_pull_request(pool: PgPool) {
    seed_task(&pool, "task-specs", "user-1").await;
    let moatless_task = make_task_row("task-moatless", "user-2");
    crate::task::test_support::seed_task(&pool, &moatless_task)
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
        crate::message::test_support::seed_message(&pool, &msg, &[], &ConversationWrite::default())
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

/// Symmetric to the inbound case: a task that opened `#607` in one repo claims
/// an id every other repo's `#607` would also arrive under.
///
/// `task_outbound_events` is keyed on the id alone (a `moat` command carries
/// nothing else), so `source_name` is the only column that tells such claims
/// apart.
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

/// A row that records no subject cannot be shown to have happened here — this
/// covers rows predating the subject columns and the `unknown`-provider
/// activity backfill.
///
/// An unplaceable row must yield nothing rather than guess: better a binding
/// not auto-attached than a PR's event stream delivered into an unrelated task.
#[sqlx::test]
async fn an_event_that_records_no_subject_names_no_task(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    crate::task::test_support::seed_task(&pool, &make_task_row("task-2", "user-2"))
        .await
        .expect("seed second task in the same repository");
    let repo = PgMessageDao(pool.clone());

    let mut msg = make_message_row("task-1", "no-subject");
    msg.external_event_id = Some("github:pr:opened:607".to_string());
    msg.external_adapter = Some("github_pr".to_string());
    msg.external_subject_key = None;
    crate::message::test_support::seed_message(&pool, &msg, &[], &ConversationWrite::default())
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

/// A claim recorded by id alone answers both readers: dedup and provenance
/// routing.
///
/// This is the shape a `moat` command in a sandbox has — it knows the task and
/// id but no `tool_use_id`; without this, the dedup gate would echo the task
/// its own comment.
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
    crate::task::test_support::seed_task(&pool, &task2)
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
/// The claim path is at-least-once (retried calls, redelivered requests); an id
/// names one event, so the first claimant keeps it.
#[sqlx::test]
async fn re_recording_an_id_leaves_the_first_row_standing(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let task2 = make_task_row("task-2", "user-2");
    crate::task::test_support::seed_task(&pool, &task2)
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
    crate::message::test_support::seed_message(&pool, &assistant, &[], &conversation)
        .await
        .unwrap();

    let repo = PgMessageDao(pool.clone());

    // The result arrives on a later source event and settles the call in place.
    crate::message::test_support::seed_activities(
        &pool,
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
    assert_eq!(
        found.parent_message_uuid,
        seeded_uuid("assistant-uuid").to_string()
    );
    assert_eq!(found.result_content.as_deref(), Some("hi"));
}

/// A plain chat message can also answer a pending question, not just the panel.
///
/// Nothing on the message's way in settles the call; the acknowledgement names
/// it, so the call comes back with the resolved row that closes the panel.
#[sqlx::test]
async fn a_plain_message_settles_the_question_it_answered(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    // The question froze in the turn still open when history froze, the one
    // frozen row a later source may settle.
    crate::message::test_support::freeze(
        &pool,
        "task-1",
        Some(chrono::Utc::now() - chrono::Duration::hours(1)),
    )
    .await
    .unwrap();

    let mut asking = make_message_row("task-1", "m-ask");
    asking.message_type = MessageType::Assistant;
    asking.message_role = MessageRole::Assistant;
    crate::message::test_support::seed_message(
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
    let written =
        crate::message::test_support::queue_named(&pool, "task-1", "m-written", "the first one")
            .await;

    let messages = crate::message::operations::Messages::new(
        std::sync::Arc::new(PgMessageDao(pool.clone())),
        crate::inbox::InboxService::new(pool.clone()),
    );
    crate::message::send::settle_answered_prompt(&messages, "task-1", "tu-1", &written).await;

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
        .get_message_for_assembly("task-1", &written)
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

/// Seam 2 (loader): a change names the row it's about, so the loader reads that
/// row and every activity written on it, including ones a later event settled.
///
/// It never resolves a folded `tool_result` row back to the assistant Message —
/// the change already names the right row directly.
#[sqlx::test]
async fn get_message_for_assembly_loads_the_named_message_with_its_activities(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // A turn that said nothing and called one tool: no text, so no Message row
    // beyond the skeleton the call hangs on.
    let mut assistant = make_message_row("task-1", "a1");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    crate::message::test_support::seed_message(
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
    crate::message::test_support::seed_activities(
        &pool,
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

    let a1 = seeded_uuid("a1").to_string();
    let ((messages, _ctx, activities), target) = repo
        .get_message_for_assembly("task-1", &a1)
        .await
        .unwrap()
        .expect("the named Message loads");
    assert_eq!(target, a1);
    let uuids: Vec<&str> = messages.iter().map(|m| m.uuid.as_str()).collect();
    assert_eq!(
        uuids,
        vec![a1.as_str()],
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
        repo.get_message_for_assembly("task-1", &seeded_uuid("sys1").to_string())
            .await
            .unwrap()
            .is_none()
    );
}

/// A turn this Task ran: its acknowledged user row and its `task_turns` row.
async fn seed_acknowledged_turn(pool: &PgPool, task_id: &str, uuid: &str, turn_number: i32) {
    seed_acknowledged_message(pool, task_id, uuid, turn_number).await;
    seed_task_turn(pool, task_id, turn_number).await;
}

async fn seed_task_turn(pool: &PgPool, task_id: &str, turn: i32) {
    let sql = "INSERT INTO task_turns (task_id, turn_number, status) VALUES ($1, $2, 'active')";
    let query = sqlx::query(sql).bind(task_id).bind(turn);
    query.execute(pool).await.unwrap();
}

/// The delivered item alone, with no `task_turns` row.
async fn seed_acknowledged_message(pool: &PgPool, task_id: &str, name: &str, turn_number: i32) {
    let id = crate::message::test_support::queue_named(pool, task_id, name, name).await;
    crate::message::test_support::acknowledge_messages(
        pool,
        task_id,
        &[id],
        &format!("session-{turn_number}"),
        turn_number,
    )
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
               (event_id, task_id, type, sub_type, turn_number, agent_profile, payload, timestamp)
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

/// `taskTurn.turn_interrupted` marks a pod torn down mid-turn (node drain, spot
/// reclaim, eviction).
///
/// It must never suppress recovery — it's precisely the signal a replacement
/// pod has a turn to resume; treating it as terminal would strand that turn
/// forever.
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

/// A delivered item whose turn has no `task_turns` row is no interrupted turn.
#[sqlx::test]
async fn find_interrupted_turn_skips_a_turn_copied_by_a_fork(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_message(&pool, "task-1", "forked-user", 7).await;

    let repo = PgMessageDao(pool.clone());
    assert!(
        repo.find_interrupted_turn("task-1")
            .await
            .unwrap()
            .is_none()
    );
}

/// A finished later turn means the open one was not running at shutdown.
#[sqlx::test]
async fn find_interrupted_turn_skips_a_turn_a_finished_later_turn_superseded(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "left-open-user", 1).await;
    seed_acknowledged_turn(&pool, "task-1", "finished-user", 2).await;
    seed_turn_event(&pool, "task-1", "taskTurn", "completed", 2).await;

    let repo = PgMessageDao(pool.clone());
    assert!(
        repo.find_interrupted_turn("task-1")
            .await
            .unwrap()
            .is_none()
    );
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

#[sqlx::test]
async fn find_interrupted_turn_aggregates_sibling_user_messages(pool: PgPool) {
    // One agent run can consume several pending messages, so a turn holds all
    // of them. Recovery must replay the same joined prompt the dispatcher built,
    // not an arbitrary one of the siblings.
    seed_task(&pool, "task-1", "user-1").await;
    let mut ids = Vec::new();
    for (name, text) in [("sib-a", "first"), ("sib-b", "second")] {
        ids.push(crate::message::test_support::queue_named(&pool, "task-1", name, text).await);
    }
    crate::message::test_support::acknowledge_messages(&pool, "task-1", &ids, "session-1", 1)
        .await
        .unwrap();
    seed_task_turn(&pool, "task-1", 1).await;

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
    seed_acknowledged_message(&pool, "task-1", "recovering-user", 1).await;
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
