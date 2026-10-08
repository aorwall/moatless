#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on, here \
              and in the shared `common` module compiled into this binary; both lints only \
              fire when the db-tests feature is enabled"
)]
//! What a buffer window does to a Binding that receives two events: they reach
//! the Task as one turn, a Loop that set the window to zero still gets two, and
//! a Binding closed before the window shuts gets neither.
//!
//! Slack stands in for every Adapter here. Buffering is decided in
//! `deliver_to_binding`, below the point any provider is named, and the durable
//! `slack.inbound_message` kind is the shortest route to it that needs no live
//! chat sender.

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use moatless_adapter_model::delivery::AdapterDeliveryRequest;
use moatless_backend::state::AppState;
use moatless_loops::bindings::test_support::upsert_binding;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_types::access_control::Scope;
use moatless_types::adapter::{AdapterKind, TaskBinding};
use moatless_types::loops::{RoutingMode, SubscriptionMatch};
use moatless_workspaces::compose;

use common::{
    USER_1, make_repository_row, make_task_row, seed_team, test_app_state_with_adapters,
    test_config,
};

const KIND: &str = "slack.inbound_message";
const CONNECTION: &str = "conn-slack-1";
const LOOP: &str = "loop-slack-1";
const TASK: &str = "task-slack-1";
const BINDING: &str = "tb-slack-1";
const SUBJECT_KIND: &str = "slack_thread";
const SUBJECT_KEY: &str = "T1:C1:1700000000.000100";

/// One event on the bound thread, in the shape the Slack handler admits.
fn event(external_event_id: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "connection_id": CONNECTION,
        "loop_id": LOOP,
        "subject_kind": SUBJECT_KIND,
        "subject_key": SUBJECT_KEY,
        "destination_config": { "channel": "C1", "thread_ts": "1700000000.000100" },
        "event_type": "event_callback",
        "message": {
            "adapter_kind": "slack",
            "source_id": "C1",
            // No `thread_id`: a reply on a thread makes `deliver_now` fetch the
            // prior messages from Slack, which is unreachable from here.
            "thread_id": null,
            "message_thread_id": "1700000000.000100",
            "user_id": USER_1,
            "author": "@ada",
            "text": text,
            "file_ids": [],
            "thread_properties": { "channel": "C1", "ts": "1700000000.000100" },
            "adapter_context": {},
            "external_event_id": external_event_id,
            "source_task_id": null,
        },
    })
}

/// A Task already bound to one Slack thread, on a Loop that holds inbound
/// events for `window` seconds.
async fn seed_bound_thread(state: &AppState, pool: &PgPool, window: i32) {
    seed_team(pool, &format!("personal-{USER_1}")).await;
    let _ = compose::seed_repository(pool, &make_repository_row("repo-1", USER_1)).await;
    common::seed_workspace_placing(pool, common::FIXTURE_WORKSPACE, "repo-1", USER_1).await;
    task_fixtures::seed_task(pool, &make_task_row(TASK, USER_1))
        .await
        .expect("seed task");

    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'slack', 'env', 'T1', $2, 'user', $2)",
    )
    .bind(CONNECTION)
    .bind(USER_1)
    .execute(pool)
    .await
    .expect("seed Slack connection");

    let now = Utc::now();
    moatless_loops::loops::test_support::upsert_subscription(
        &state.loops,
        &SubscriptionMatch {
            name: "test-loop".to_string(),
            id: LOOP.to_string(),
            adapter_kind: AdapterKind::Slack,
            connection_id: CONNECTION.to_string(),
            source_matcher: "C1".to_string(),
            source_name: Some("#eng".to_string()),
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            workspace_id: common::FIXTURE_WORKSPACE.to_string(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: Some(USER_1.to_string()),
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            inactive_ttl_days: None,
            node_selector: None,
            stop_when_done: None,
            // The whole subject of this file. Slack buffers for nothing by
            // default, so every window here is the Loop's own.
            message_buffer_seconds: Some(window),
            allow_direct_send: false,
            adapter_config: None,
            ongoing_task_id: None,
            close_task_on_close: false,
            notify_on_success: false,
            active: true,
            created_by: USER_1.to_string(),
            scope: Scope::User,
            owner_user_id: Some(USER_1.to_string()),
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .expect("seed Slack loop");

    let binding = TaskBinding {
        id: BINDING.to_string(),
        task_id: TASK.to_string(),
        adapter_kind: AdapterKind::Slack,
        subject_kind: SUBJECT_KIND.to_string(),
        subject_key: SUBJECT_KEY.to_string(),
        connection_id: CONNECTION.to_string(),
        loop_id: Some(LOOP.to_string()),
        destination_config: serde_json::json!({"channel": "C1"}),
        metadata: serde_json::json!({}),
        last_external_event_id: None,
        origin: "provider_event".to_string(),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
        watching: true,
    };
    upsert_binding(&state.task_bindings, &binding)
        .await
        .expect("seed thread binding");
}

/// Admit each event the way the Slack webhook does, then route them all.
///
/// Routing is what decides to buffer, so nothing below has happened until this
/// returns.
async fn route(state: &AppState, pool: &PgPool, events: &[(&str, &str)]) {
    for (external_event_id, text) in events {
        state
            .adapter_delivery
            .admit(AdapterDeliveryRequest {
                adapter: "slack",
                kind: KIND,
                event_id: (*external_event_id).to_string(),
                payload: event(external_event_id, text),
            })
            .await
            .expect("admit the event");
    }

    let handlers = state
        .adapter_installations
        .as_ref()
        .expect("test state installs external Adapters")
        .delivery_handlers(state);
    let cancel = CancellationToken::new();
    let handle = state
        .adapter_delivery_worker
        .spawn(Arc::new(handlers), cancel.clone());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut drained = false;
    while tokio::time::Instant::now() < deadline {
        let unfinished: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM adapter_delivery WHERE status IN ('pending', 'claimed')",
        )
        .fetch_one(pool)
        .await
        .expect("read the delivery backlog");
        if unfinished == 0 {
            drained = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    cancel.cancel();
    handle.await.expect("worker exits cleanly on cancellation");

    assert!(drained, "every event reached a terminal state");
    let failed: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT event_id, last_error FROM adapter_delivery WHERE status = 'failed'")
            .fetch_all(pool)
            .await
            .expect("read failed records");
    assert!(failed.is_empty(), "no event should have failed: {failed:?}");
}

/// Close every open window and run the real flusher until the buffer empties.
async fn flush_windows(state: &AppState, pool: &PgPool) {
    sqlx::query("UPDATE inbound_message_buffer SET flush_at = NOW() - INTERVAL '1 second'")
        .execute(pool)
        .await
        .expect("close the open windows");

    let cancel = CancellationToken::new();
    let handle = state
        .message_buffer_worker
        .spawn(state.routing.clone(), cancel.clone());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline && buffered(pool).await > 0 {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    cancel.cancel();
    handle.await.expect("flusher exits cleanly on cancellation");
}

/// Events still held for a window.
async fn buffered(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM inbound_message_buffer")
        .fetch_one(pool)
        .await
        .expect("read the inbound buffer")
}

/// Every message Slack has put on the task, oldest first.
async fn delivered_messages(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT i.text FROM task_inbox i \
         WHERE i.task_id = $1 AND i.adapter = 'slack' \
         ORDER BY i.created_at, i.id",
    )
    .bind(TASK)
    .fetch_all(pool)
    .await
    .expect("read forwarded messages")
    .into_iter()
    .map(Option::unwrap_or_default)
    .collect()
}

/// The claim this whole feature makes: a burst on one Binding is one turn, not
/// one turn per event.
#[sqlx::test]
async fn two_events_inside_one_window_reach_the_task_as_one_message(pool: PgPool) {
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    seed_bound_thread(&state, &pool, 60).await;

    route(
        &state,
        &pool,
        &[
            ("slack:ts:1700000000.000101", "first comment"),
            ("slack:ts:1700000000.000102", "second comment"),
        ],
    )
    .await;

    assert_eq!(buffered(&pool).await, 2, "both events are held");
    assert!(
        delivered_messages(&pool).await.is_empty(),
        "nothing reaches the task while the window is open"
    );

    flush_windows(&state, &pool).await;

    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        1,
        "one window is one turn, got {messages:?}"
    );
    let combined = &messages[0];
    assert!(
        combined.contains("first comment") && combined.contains("second comment"),
        "the turn carries both bodies: {combined}"
    );
    assert!(
        combined.contains("--- message 1 of 2"),
        "the parts are labelled oldest first: {combined}"
    );
    assert_eq!(buffered(&pool).await, 0, "a flushed window drops its rows");
}

/// The control: the same two events on a Loop that turned buffering off never
/// reach the buffer at all.
#[sqlx::test]
async fn a_loop_with_the_window_at_zero_delivers_each_event_on_its_own(pool: PgPool) {
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    seed_bound_thread(&state, &pool, 0).await;

    route(
        &state,
        &pool,
        &[
            ("slack:ts:1700000000.000101", "first comment"),
            ("slack:ts:1700000000.000102", "second comment"),
        ],
    )
    .await;

    assert_eq!(buffered(&pool).await, 0, "nothing was held");
    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        2,
        "each event is its own turn, got {messages:?}"
    );
    assert!(
        messages
            .iter()
            .all(|text| !text.contains("--- message 1 of")),
        "a lone event renders as it did before buffering existed: {messages:?}"
    );
}

/// A Loop deactivated while its window is open. The Binding keeps receiving,
/// under a Subscription derived from its connection, because a Binding whose
/// Subscription is gone still works.
#[sqlx::test]
async fn a_loop_deactivated_inside_its_window_still_delivers(pool: PgPool) {
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    seed_bound_thread(&state, &pool, 60).await;

    route(
        &state,
        &pool,
        &[("slack:ts:1700000000.000101", "first comment")],
    )
    .await;
    assert_eq!(buffered(&pool).await, 1);

    sqlx::query("UPDATE loops SET active = false WHERE id = $1")
        .bind(LOOP)
        .execute(&pool)
        .await
        .expect("deactivate the loop");

    flush_windows(&state, &pool).await;

    assert_eq!(buffered(&pool).await, 0, "the window flushed");
    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        1,
        "the event is delivered, got {messages:?}"
    );
    assert!(
        messages[0].contains("first comment"),
        "the turn carries the event: {}",
        messages[0]
    );
}

/// A Binding that closes while its window is open: the Task is finished with,
/// and the flusher must drop the events rather than resurrect it.
#[sqlx::test]
async fn events_buffered_for_a_binding_that_closed_are_dropped(pool: PgPool) {
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    seed_bound_thread(&state, &pool, 60).await;

    route(
        &state,
        &pool,
        &[
            ("slack:ts:1700000000.000101", "first comment"),
            ("slack:ts:1700000000.000102", "second comment"),
        ],
    )
    .await;
    assert_eq!(buffered(&pool).await, 2);

    sqlx::query("UPDATE task_bindings SET active = false, deactivated_at = NOW() WHERE id = $1")
        .bind(BINDING)
        .execute(&pool)
        .await
        .expect("close the binding");

    flush_windows(&state, &pool).await;

    assert_eq!(buffered(&pool).await, 0, "the held events are dropped");
    assert!(
        delivered_messages(&pool).await.is_empty(),
        "a closed binding receives nothing"
    );
}
