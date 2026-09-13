#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for per-block Message assembly in
//! `event_handler::agent_handlers::handle_agent_message`.
//!
//! A native Message may arrive as several per-block events tagged with
//! `blockIndex` (same usage repeated by some CLIs). These fold into one Message
//! with usage admitted once even under duplicate delivery; an interleaved tool
//! result settles its call instead of becoming its own Message, and calls
//! without `blockIndex` stay compatible.

use crate::common;

use moatless_backend::event_handler::agent_handlers::handle_agent_message;
use moatless_tasks::agent_event::AgentEvent;
use moatless_tasks::message::transform::transform_subagent_transcript;
use moatless_tasks::message::transform::transform_to_ui;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_tasks::task_activities::Activity;
use moatless_tasks::task_activities::mapping::ActivityKind;
use moatless_tasks::task_activities::mapping::ActivityStatus;
use moatless_workspaces::compose;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row, seed_team, test_app_state};

/// The activities written on one Message, in the order they were recorded.
fn on_message<'a>(activities: &'a [Activity], uuid: &str) -> Vec<&'a Activity> {
    let mut found: Vec<_> = activities
        .iter()
        .filter(|activity| activity.message_uuid.as_deref() == Some(uuid))
        .collect();
    found.sort_by_key(|activity| activity.position);
    found
}

/// A per-block assistant source event: one native block at `block_index`, with
/// usage repeated on every block (as a coalescing-free CLI would emit it).
fn per_block_event(
    event_id: &str,
    task_id: &str,
    uuid: &str,
    block_index: usize,
    block: serde_json::Value,
) -> AgentEvent {
    let payload = serde_json::json!({
        "uuid": uuid,
        "session_id": "session-1",
        "blockIndex": block_index,
        "message": {
            "role": "assistant",
            "usage": { "input_tokens": 10, "output_tokens": 50 },
            "content": [block],
        },
    });
    event_from_payload(event_id, task_id, payload)
}

/// A coalesced event carrying the full content array and no `blockIndex`.
fn coalesced_event(
    event_id: &str,
    task_id: &str,
    role: &str,
    uuid: &str,
    content: serde_json::Value,
) -> AgentEvent {
    let payload = serde_json::json!({
        "uuid": uuid,
        "session_id": "session-1",
        "message": {
            "role": role,
            "content": content,
        },
    });
    event_from_payload(event_id, task_id, payload)
}

fn event_from_payload(event_id: &str, task_id: &str, payload: serde_json::Value) -> AgentEvent {
    let value = serde_json::json!({
        "eventId": event_id,
        "type": "agent",
        "subType": "message",
        "taskId": task_id,
        "timestamp": "2026-04-30T00:00:00.000Z",
        "isError": false,
        "agentType": "claude-code",
        "turnNumber": 1,
        "payload": payload,
    });
    serde_json::from_value(value).unwrap()
}

fn text(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "text": text })
}

fn tool_use(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({ "type": "tool_use", "id": id, "name": name, "input": {} })
}

async fn seed(pool: &PgPool, task_id: &str) -> moatless_backend::state::AppState {
    seed_team(pool, "personal-user-1").await;
    let repo = make_repository_row("repo-1", "user-1");
    compose::seed_repository(pool, &repo).await.unwrap();
    let task = make_task_row(task_id, "user-1");
    task_fixtures::seed_task(pool, &task).await.unwrap();
    test_app_state(pool.clone())
}

#[sqlx::test]
async fn per_block_events_assemble_into_one_ordered_message(pool: PgPool) {
    let task_id = "task-per-block-order";
    let state = seed(&pool, task_id).await;
    let uuid = "asst-1";

    // Three native blocks arrive in native order as separate source events.
    handle_agent_message(
        &state,
        &per_block_event("e0", task_id, uuid, 0, text("first")),
    )
    .await;
    handle_agent_message(
        &state,
        &per_block_event("e1", task_id, uuid, 1, tool_use("tu-1", "Bash")),
    )
    .await;
    handle_agent_message(
        &state,
        &per_block_event("e2", task_id, uuid, 2, text("second")),
    )
    .await;

    let (rows, contexts, activities) =
        moatless_tasks::message::test_support::messages_for(&pool, task_id)
            .await
            .unwrap();

    // Exactly one persisted row for the native Message.
    let assistant: Vec<_> = rows.iter().filter(|m| m.uuid == uuid).collect();
    assert_eq!(assistant.len(), 1, "one row per native Message identity");

    // The two text blocks fold into one string, in native block order. The
    // tool_use between them left a hole in the parts array rather than text.
    assert_eq!(
        assistant[0].message_text.as_deref(),
        Some("first\n\nsecond"),
        "text folded in native order, the call skipped"
    );

    // The call is an activity keyed by its own block, written once.
    let calls = on_message(&activities, uuid);
    assert_eq!(calls.len(), 1, "one activity, no dupes");
    assert_eq!(calls[0].activity_id, format!("{uuid}:1"));
    assert_eq!(calls[0].kind, ActivityKind::ToolCall);
    assert_eq!(calls[0].summary, "Bash");
    assert_eq!(calls[0].status, ActivityStatus::InProgress);

    // The read seam assembles exactly one top-level UI Message: the folded
    // text, then the call.
    let ui = transform_to_ui(&rows, &contexts, &activities);
    let ui_assistant: Vec<_> = ui.iter().filter(|m| m.uuid == uuid).collect();
    assert_eq!(ui_assistant.len(), 1, "one assembled UI Message");
    assert_eq!(ui_assistant[0].content.len(), 2);
    assert_eq!(
        ui_assistant[0].content[0].text.as_deref(),
        Some("first\n\nsecond")
    );
    assert_eq!(ui_assistant[0].content[1].block_type, "tool_use");
}

#[sqlx::test]
async fn usage_repeated_across_blocks_is_admitted_once(pool: PgPool) {
    let task_id = "task-per-block-usage";
    let state = seed(&pool, task_id).await;
    let uuid = "asst-usage";

    // Every per-block event repeats the same usage (output_tokens = 50).
    for (i, block) in [text("a"), text("b"), text("c")].into_iter().enumerate() {
        handle_agent_message(
            &state,
            &per_block_event(&format!("e{i}"), task_id, uuid, i, block),
        )
        .await;
    }

    let (rows, _, _) = moatless_tasks::message::test_support::messages_for(&pool, task_id)
        .await
        .unwrap();
    let row = rows.iter().find(|m| m.uuid == uuid).unwrap();

    // Admitted exactly once — not summed across the three blocks (would be 150).
    assert_eq!(row.assistant_usage_output_tokens, Some(50));
    assert_eq!(row.assistant_usage_input_tokens, Some(10));
}

#[sqlx::test]
async fn duplicate_per_block_delivery_is_idempotent(pool: PgPool) {
    let task_id = "task-per-block-dupe";
    let state = seed(&pool, task_id).await;
    let uuid = "asst-dupe";

    handle_agent_message(&state, &per_block_event("e0", task_id, uuid, 0, text("a"))).await;
    handle_agent_message(&state, &per_block_event("e1", task_id, uuid, 1, text("b"))).await;
    // Redeliver block 0 — a duplicate must not duplicate blocks or usage.
    handle_agent_message(&state, &per_block_event("e0b", task_id, uuid, 0, text("a"))).await;

    let (rows, _, activities) = moatless_tasks::message::test_support::messages_for(&pool, task_id)
        .await
        .unwrap();
    let row = rows.iter().find(|m| m.uuid == uuid).unwrap();

    // Block 0 was written twice and landed in its own slot both times, so the
    // redelivery neither appended nor reordered.
    assert_eq!(
        row.message_text.as_deref(),
        Some("a\n\nb"),
        "duplicate did not add a paragraph"
    );

    assert!(
        on_message(&activities, uuid).is_empty(),
        "text-only blocks record no activity"
    );
    assert_eq!(row.assistant_usage_output_tokens, Some(50));
}

#[sqlx::test]
async fn interleaved_tool_result_settles_the_call_on_its_parent(pool: PgPool) {
    let task_id = "task-per-block-parent";
    let state = seed(&pool, task_id).await;

    // Assistant Message assembled per-block: text then a tool_use.
    handle_agent_message(
        &state,
        &per_block_event("e0", task_id, "asst-p", 0, text("running")),
    )
    .await;
    handle_agent_message(
        &state,
        &per_block_event("e1", task_id, "asst-p", 1, tool_use("tu-9", "Bash")),
    )
    .await;

    // The matching tool_result arrives as a user-role event.
    handle_agent_message(
        &state,
        &coalesced_event(
            "e2",
            task_id,
            "user",
            "tr-p",
            serde_json::json!([{
                "type": "tool_result",
                "tool_use_id": "tu-9",
                "content": "done",
                "is_error": false,
            }]),
        ),
    )
    .await;

    let (rows, contexts, activities) =
        moatless_tasks::message::test_support::messages_for(&pool, task_id)
            .await
            .unwrap();

    // The result carried no text, so it wrote no Message of its own — it
    // settled the activity the call already wrote, in place.
    assert!(
        rows.iter().all(|m| m.uuid != "tr-p"),
        "a tool result is not something anybody said"
    );
    let calls = on_message(&activities, "asst-p");
    assert_eq!(calls.len(), 1, "still one activity for the one call");
    assert_eq!(calls[0].tool_use_id.as_deref(), Some("tu-9"));
    assert_eq!(calls[0].status, ActivityStatus::Completed);
    assert!(!calls[0].is_error);

    // The read seam renders one top-level Message carrying the inlined result.
    let ui = transform_to_ui(&rows, &contexts, &activities);
    assert_eq!(ui.len(), 1, "tool_result folded into its parent Message");
    assert_eq!(ui[0].uuid, "asst-p");
    assert_eq!(
        ui[0].tool_results.len(),
        1,
        "the parent renders the interleaved tool result"
    );
    assert_eq!(ui[0].tool_results[0].content.as_deref(), Some("done"));
}

#[sqlx::test]
async fn coalesced_events_remain_compatible(pool: PgPool) {
    let task_id = "task-coalesced";
    let state = seed(&pool, task_id).await;

    // A legacy coalesced event carries the whole content array and no
    // blockIndex; it must still assemble into one Message with both blocks.
    handle_agent_message(
        &state,
        &coalesced_event(
            "e0",
            task_id,
            "assistant",
            "asst-coalesced",
            serde_json::json!([text("hello"), tool_use("tu-c", "Bash")]),
        ),
    )
    .await;

    let (rows, contexts, activities) =
        moatless_tasks::message::test_support::messages_for(&pool, task_id)
            .await
            .unwrap();
    let row = rows.iter().find(|m| m.uuid == "asst-coalesced").unwrap();
    assert_eq!(row.message_text.as_deref(), Some("hello"));

    let calls = on_message(&activities, "asst-coalesced");
    assert_eq!(calls.len(), 1, "the call in the array is one activity");
    assert_eq!(calls[0].tool_use_id.as_deref(), Some("tu-c"));

    let ui = transform_to_ui(&rows, &contexts, &activities);
    let assembled = ui.iter().find(|m| m.uuid == "asst-coalesced").unwrap();
    assert_eq!(assembled.content.len(), 2);
}

/// A per-block assistant source event belonging to a subagent: it carries the
/// spawning `Agent` tool-use id as `parent_tool_use_id`, so its assembled row
/// is a subagent row (excluded from the top-level conversation).
fn subagent_per_block_event(
    event_id: &str,
    task_id: &str,
    uuid: &str,
    parent_tool_use_id: &str,
    block_index: usize,
    block: serde_json::Value,
) -> AgentEvent {
    let payload = serde_json::json!({
        "uuid": uuid,
        "session_id": "session-1",
        "parent_tool_use_id": parent_tool_use_id,
        "blockIndex": block_index,
        "message": {
            "role": "assistant",
            "usage": { "input_tokens": 10, "output_tokens": 50 },
            "content": [block],
        },
    });
    event_from_payload(event_id, task_id, payload)
}

#[sqlx::test]
async fn subagent_per_block_rows_are_excluded_from_top_level(pool: PgPool) {
    let task_id = "task-per-block-subagent";
    let state = seed(&pool, task_id).await;
    let parent = "agent-tool-1";

    // A subagent assembles its own Message per-block, exactly like the top level.
    handle_agent_message(
        &state,
        &subagent_per_block_event("s0", task_id, "sub-asst", parent, 0, text("sub-first")),
    )
    .await;
    handle_agent_message(
        &state,
        &subagent_per_block_event(
            "s1",
            task_id,
            "sub-asst",
            parent,
            1,
            tool_use("sub-tu", "Bash"),
        ),
    )
    .await;

    let (rows, contexts, activities) =
        moatless_tasks::message::test_support::messages_for(&pool, task_id)
            .await
            .unwrap();

    // The subagent row is persisted and assembled per-block, carrying its parent.
    let row = rows.iter().find(|m| m.uuid == "sub-asst").unwrap();
    assert_eq!(row.parent_uuid.as_deref(), Some(parent));
    assert_eq!(row.message_text.as_deref(), Some("sub-first"));

    // Its activity is not in the Task's own stream at all: the parent scopes it
    // out at the read, so nothing above the DAO has to filter it.
    assert!(
        on_message(&activities, "sub-asst").is_empty(),
        "a subagent's activities are not the Task's own"
    );

    // Top-level read excludes it — a subagent never appears in the task's own
    // conversation.
    let top = transform_to_ui(&rows, &contexts, &activities);
    assert!(
        top.iter().all(|m| m.uuid != "sub-asst"),
        "subagent row must not appear at the top level"
    );

    // The subagent's own page is where it lives, loaded on demand behind the
    // call that spawned it.
    let (sub_rows, sub_contexts, sub_activities) =
        moatless_tasks::message::test_support::subagent_messages(&pool, task_id, parent)
            .await
            .unwrap();
    let calls = on_message(&sub_activities, "sub-asst");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].parent_uuid.as_deref(), Some(parent));

    // The subagent's own transcript, scoped by its tool-use id, includes it with
    // its text and its call.
    let transcript =
        transform_subagent_transcript(&sub_rows, &sub_contexts, &sub_activities, parent);
    let assembled = transcript
        .iter()
        .find(|m| m.uuid == "sub-asst")
        .expect("subagent transcript includes the row");
    assert_eq!(assembled.content.len(), 2);
}

#[sqlx::test]
async fn paginated_read_folds_per_block_rows_into_one_message(pool: PgPool) {
    let task_id = "task-per-block-paginated";
    let state = seed(&pool, task_id).await;
    let uuid = "asst-paged";

    // Three native blocks arrive as separate per-block source events.
    for (i, block) in [text("one"), tool_use("tu-p", "Bash"), text("three")]
        .into_iter()
        .enumerate()
    {
        handle_agent_message(
            &state,
            &per_block_event(&format!("p{i}"), task_id, uuid, i, block),
        )
        .await;
    }

    // The keyset-paginated read returns the same assembled Message and the
    // activities written on it — per-block rows fold within the page, not
    // across it.
    let (rows, _contexts, activities) =
        moatless_tasks::message::test_support::paginated_messages(&pool, task_id, None, 50)
            .await
            .unwrap();

    let row = rows.iter().find(|m| m.uuid == uuid).unwrap();
    assert_eq!(
        row.message_text.as_deref(),
        Some("one\n\nthree"),
        "paginated read returns the fully folded per-block text"
    );
    let calls = on_message(&activities, uuid);
    assert_eq!(
        calls.len(),
        1,
        "the page carries the Message's activities, none duplicated"
    );
    assert_eq!(calls[0].tool_use_id.as_deref(), Some("tu-p"));
}
