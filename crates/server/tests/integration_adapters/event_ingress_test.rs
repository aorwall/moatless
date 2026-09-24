#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for durable event admission — requires `DATABASE_URL` and
//! `--features db-tests`.
//!
//! Drives the real gRPC ingress, outbox, relay, and consumer so each case is
//! asserted at the seam the sandbox and consumer meet: a failed and recovered
//! transport, a lost gRPC response, concurrent duplicates, and full message
//! content reaching `task_messages` exactly once with state committed before
//! fan-out.

use crate::common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use tonic::Request;

use moatless_backend::event_handler::agent_handlers::handle_agent_message;
use moatless_backend::event_handler::dispatcher::persist_event;
use moatless_backend::sandbox::grpc_server::{BackendSandboxServiceImpl, IngressPorts};
use moatless_core::events::{EventPublisher, PublishError};
use moatless_tasks::agent_event::AgentEvent;
use moatless_tasks::compose::TaskGraph;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_types::agent::AgentSubType;
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use moatless_types::task::{TaskSubType, TaskTurnSubType};
use moatless_workspaces::compose;
use sandbox_proto::PublishEventRequest;
use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;

use common::{make_repository_row, make_task_row, test_app_state};

// =========================================================================
// A transport stand-in
// =========================================================================

/// Captures raw publishes and can be made to fail, standing in for the feed.
#[derive(Default)]
struct FakeTransport {
    published: Mutex<Vec<(String, String, Value)>>,
    fail_next: Mutex<usize>,
}

impl FakeTransport {
    fn failing(fail_next: usize) -> Self {
        Self {
            fail_next: Mutex::new(fail_next),
            ..Self::default()
        }
    }

    /// `(event_id, sub_type, payload)` per successful publish, in order.
    fn published(&self) -> Vec<(String, String, Value)> {
        self.published.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventPublisher for FakeTransport {
    async fn publish_sandbox_event(&self, _: &str, _: SandboxStatus, _: &str) {}
    async fn publish_sandbox_event_with_payload(
        &self,
        _: &str,
        _: SandboxStatus,
        _: &SandboxEventPayload,
    ) {
    }
    async fn publish_task_event(&self, _: &str, _: TaskSubType, _: Option<&str>, _: Value) {}
    async fn publish_agent_event(&self, _: &str, _: AgentSubType, _: Option<&str>, _: Value) {}
    async fn publish_task_turn_event(
        &self,
        _: &str,
        _: TaskTurnSubType,
        _: Option<&str>,
        _: Option<i32>,
        _: Value,
    ) {
    }
    async fn publish_raw_event(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<i32>,
        _: Value,
    ) {
    }
    async fn publish_raw_event_with_id(
        &self,
        event_id: &str,
        _task_id: &str,
        _event_type: &str,
        sub_type: &str,
        _agent_type: Option<&str>,
        _turn_number: Option<i32>,
        payload: Value,
    ) -> Result<(), PublishError> {
        {
            let mut remaining = self.fail_next.lock().unwrap();
            if *remaining > 0 {
                *remaining -= 1;
                return Err(PublishError::Transport {
                    source: sqlx::Error::PoolTimedOut,
                });
            }
        }
        self.published
            .lock()
            .unwrap()
            .push((event_id.to_string(), sub_type.to_string(), payload));
        Ok(())
    }
}

// =========================================================================
// Fixtures
// =========================================================================

async fn seed_task(pool: &PgPool, task_id: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    compose::seed_repository(pool, &repo).await.unwrap();
    let task = make_task_row(task_id, "user-1");
    task_fixtures::seed_task(pool, &task).await.unwrap();
}

/// The tasks band's stores over `pool`, as the composition root builds them.
fn task_graph(pool: &PgPool) -> TaskGraph {
    moatless_tasks::compose::task_graph(pool.clone())
}

/// The gRPC ingress wired to a real pool, with `transport` as its event
/// transport (`None` models a process composed without one).
fn ingress(pool: &PgPool, transport: Option<Arc<dyn EventPublisher>>) -> BackendSandboxServiceImpl {
    let graph = task_graph(pool);
    BackendSandboxServiceImpl::for_ingress_tests(
        transport.clone(),
        IngressPorts {
            admission: graph.event_ingress(transport),
            task_turns: graph.task_turns(),
            task_events: graph.task_events(),
        },
    )
}

/// An `agent.message` carrying real content — the payload kind whose full body
/// has no durable home outside the ingress outbox.
fn message_request(
    task_id: &str,
    event_id: &str,
    sequence: u64,
    text: &str,
) -> PublishEventRequest {
    let payload = serde_json::json!({
        "uuid": "msg-1",
        "session_id": "session-1",
        "message": {
            "role": "assistant",
            "content": [{ "type": "text", "text": text }],
        },
    });
    PublishEventRequest {
        task_id: task_id.to_string(),
        event_type: sandbox_proto::EventType::Agent as i32,
        sub_type: "message".to_string(),
        payload_json: payload.to_string(),
        event_id: event_id.to_string(),
        sequence,
        run_id: String::new(),
    }
}

/// Rebuild the dispatcher's view of a published event, exactly as it would
/// parse it off the feed.
fn consumed(task_id: &str, event_id: &str, payload: &Value) -> AgentEvent {
    serde_json::from_value(serde_json::json!({
        "eventId": event_id,
        "type": "agent",
        "subType": "message",
        "taskId": task_id,
        "timestamp": "2026-07-20T00:00:00.000Z",
        "isError": false,
        "agentType": "claude-code",
        "turnNumber": 1,
        "payload": payload,
    }))
    .unwrap()
}

/// What the event log kept for `event_id`, if it kept anything.
async fn event_log_payload(pool: &PgPool, event_id: &str) -> Option<Value> {
    sqlx::query_scalar("SELECT payload FROM task_events WHERE event_id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .unwrap()
}

async fn outbox_event_ids(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT event_id FROM event_ingress_outbox ORDER BY event_id")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// What the Task's Messages say, in order. Text is the Message now — the blocks
/// that are not text are activities and have rows of their own.
async fn message_texts(pool: &PgPool, task_id: &str) -> Vec<Option<String>> {
    sqlx::query_scalar("SELECT message_text FROM task_messages WHERE task_id = $1 ORDER BY id")
        .bind(task_id)
        .fetch_all(pool)
        .await
        .unwrap()
}

// =========================================================================
// Admission → consumer → delete
// =========================================================================

#[sqlx::test]
async fn full_message_content_reaches_task_messages_exactly_once(pool: PgPool) {
    let task_id = "task-admit-1";
    seed_task(&pool, task_id).await;
    let transport = Arc::new(FakeTransport::default());
    let svc = ingress(&pool, Some(transport.clone()));

    let response = svc
        .publish_event(Request::new(message_request(
            task_id,
            "evt-1",
            1,
            "the whole message body",
        )))
        .await
        .expect("a spooled payload is admitted");
    assert_eq!(response.into_inner().event_id, "evt-1");

    // Durable admission: the full payload is in the outbox, and `task_events`
    // has no copy of it.
    assert_eq!(outbox_event_ids(&pool).await, ["evt-1"]);
    let spooled: Value =
        sqlx::query_scalar("SELECT payload FROM event_ingress_outbox WHERE event_id = $1")
            .bind("evt-1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        spooled["message"]["content"][0]["text"],
        "the whole message body"
    );
    // `task_events` holds nothing for it, not even a skeleton. The Message's
    // durable home on the way in is the outbox, and afterwards it is the
    // conversation. A row here would be a second copy of that state.
    assert!(
        event_log_payload(&pool, "evt-1").await.is_none(),
        "admission writes no task_events row for an agent.message"
    );

    // The consumer receives the published payload whole and commits it, in the
    // order its loop runs: append to the event log, then dispatch.
    let published = transport.published();
    assert_eq!(published.len(), 1);
    let state = test_app_state(pool.clone());
    let event = consumed(task_id, &published[0].0, &published[0].2);
    persist_event(&state, &event).await;
    handle_agent_message(&state, &event).await;

    // The consumer does not write one either. The Sandbox publishes one
    // `agent.message` per block, so a row here would be one event-log row per
    // activity.
    let history = event_log_payload(&pool, "evt-1").await;
    assert!(
        history.is_none(),
        "the consumer writes no task_events row for an agent.message: {history:?}"
    );

    let texts = message_texts(&pool, task_id).await;
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].as_deref(), Some("the whole message body"));

    // Committing the message is what deletes the record — and only that.
    assert!(outbox_event_ids(&pool).await.is_empty());

    // At-least-once redelivery converges on the same single row.
    handle_agent_message(&state, &event).await;
    assert_eq!(message_texts(&pool, task_id).await.len(), 1);
}

/// What the previous test asserts is missing must not be missing for everything
/// else: `task_events` is still the lifecycle log, and only a Message is kept
/// out of it.
#[sqlx::test]
async fn a_lifecycle_event_still_reaches_the_event_log(pool: PgPool) {
    let task_id = "task-admit-1b";
    seed_task(&pool, task_id).await;
    let state = test_app_state(pool.clone());

    let event: AgentEvent = serde_json::from_value(serde_json::json!({
        "eventId": "evt-lifecycle",
        "type": "agent",
        "subType": "completed",
        "taskId": task_id,
        "timestamp": "2026-07-20T00:00:00.000Z",
        "isError": false,
        "agentType": "claude-code",
        "turnNumber": 1,
        "payload": { "reason": "done" },
    }))
    .unwrap();
    persist_event(&state, &event).await;

    assert_eq!(
        event_log_payload(&pool, "evt-lifecycle").await,
        Some(serde_json::json!({ "reason": "done" }))
    );
}

#[sqlx::test]
async fn a_response_lost_after_admission_is_not_re_admitted(pool: PgPool) {
    let task_id = "task-admit-2";
    seed_task(&pool, task_id).await;
    let transport = Arc::new(FakeTransport::default());
    let svc = ingress(&pool, Some(transport.clone()));

    // The sandbox never saw the first response, so it resends the same event
    // under the same id and sequence.
    let request = message_request(task_id, "evt-1", 7, "hello");
    svc.publish_event(Request::new(request.clone()))
        .await
        .unwrap();
    let retry = svc.publish_event(Request::new(request)).await.unwrap();

    assert_eq!(retry.into_inner().event_id, "evt-1");
    assert_eq!(
        transport.published().len(),
        1,
        "a retry of an admitted event must not reach the stream twice"
    );
    assert_eq!(outbox_event_ids(&pool).await, ["evt-1"]);
}

/// A retry the dedup cache has forgotten (post-restart, record already consumed
/// and deleted) still converges rather than duplicating, since the consumer's
/// idempotency covers the feed's redelivery too.
#[sqlx::test]
async fn a_retry_that_outlives_the_dedup_cache_converges(pool: PgPool) {
    let task_id = "task-admit-2b";
    seed_task(&pool, task_id).await;
    let transport = Arc::new(FakeTransport::default());
    let state = test_app_state(pool.clone());
    let request = message_request(task_id, "evt-1", 3, "the whole message body");

    // First delivery: admitted, consumed, record deleted.
    ingress(&pool, Some(transport.clone()))
        .publish_event(Request::new(request.clone()))
        .await
        .unwrap();
    let first = transport.published();
    handle_agent_message(&state, &consumed(task_id, &first[0].0, &first[0].2)).await;
    assert!(outbox_event_ids(&pool).await.is_empty());

    // A restarted backend — a brand-new dedup cache with no memory of `evt-1`.
    ingress(&pool, Some(transport.clone()))
        .publish_event(Request::new(request))
        .await
        .expect("a forgotten retry is re-admitted, never rejected");

    // It does fan out again, and that is fine: the consumer upserts the same
    // uuid, so the message body is neither duplicated nor truncated.
    let published = transport.published();
    assert_eq!(published.len(), 2, "the forgotten retry is published again");
    handle_agent_message(&state, &consumed(task_id, &published[1].0, &published[1].2)).await;

    let texts = message_texts(&pool, task_id).await;
    assert_eq!(texts.len(), 1, "one message, not two");
    assert_eq!(texts[0].as_deref(), Some("the whole message body"));

    // And the re-admitted record is released again rather than stranded.
    assert!(outbox_event_ids(&pool).await.is_empty());
}

#[sqlx::test]
async fn concurrent_duplicate_requests_fan_out_once(pool: PgPool) {
    let task_id = "task-admit-3";
    seed_task(&pool, task_id).await;
    let transport = Arc::new(FakeTransport::default());
    let svc = Arc::new(ingress(&pool, Some(transport.clone())));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let svc = svc.clone();
        let task_id = task_id.to_string();
        handles.push(tokio::spawn(async move {
            svc.publish_event(Request::new(message_request(&task_id, "evt-1", 1, "hi")))
                .await
                .map(|_| ())
                .map_err(|status| status.code())
        }));
    }
    for handle in handles {
        // Every answer is either "admitted" or "retry" — never a silent
        // acknowledgement of an admission that has not happened.
        match handle.await.unwrap() {
            Ok(()) => {}
            Err(code) => assert_eq!(code, tonic::Code::Unavailable),
        }
    }

    assert_eq!(transport.published().len(), 1);
    assert_eq!(outbox_event_ids(&pool).await, ["evt-1"]);
}

// =========================================================================
// Transport failures
// =========================================================================

#[sqlx::test]
async fn a_state_bearing_event_is_admitted_without_a_transport(pool: PgPool) {
    let task_id = "task-admit-5";
    seed_task(&pool, task_id).await;
    let svc = ingress(&pool, None);

    // Its committed state is the durable substitute for the fan-out, so this
    // one *is* admitted with no transport at all.
    svc.publish_event(Request::new(PublishEventRequest {
        task_id: task_id.to_string(),
        event_type: sandbox_proto::EventType::TaskTurn as i32,
        sub_type: "active".to_string(),
        payload_json: "{}".to_string(),
        event_id: "evt-state".to_string(),
        sequence: 1,
        run_id: "run-1".to_string(),
    }))
    .await
    .expect("committed state is a durable admission");

    let status: String =
        sqlx::query_scalar("SELECT status FROM task_turns WHERE task_id = $1 AND turn_number = 1")
            .bind(task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "active");
    // The state is committed before any fan-out could have happened, so the
    // row exists even though nothing was ever published.
    let event_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM task_events WHERE event_id = $1")
            .bind("evt-state")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(event_rows, 1);
}

#[sqlx::test]
async fn a_failed_transport_recovers_through_the_relay(pool: PgPool) {
    let task_id = "task-admit-6";
    seed_task(&pool, task_id).await;
    // The transport fails the ingress publish, then recovers.
    let transport = Arc::new(FakeTransport::failing(1));
    let svc = ingress(&pool, Some(transport.clone()));

    svc.publish_event(Request::new(message_request(
        task_id,
        "evt-1",
        1,
        "survives the outage",
    )))
    .await
    .expect("durably admitted despite the failed publish");

    assert!(transport.published().is_empty());
    assert_eq!(outbox_event_ids(&pool).await, ["evt-1"]);

    // The record is scheduled for its first relay attempt in the future; age it
    // so one pass picks it up rather than sleeping through the retry window.
    sqlx::query("UPDATE event_ingress_outbox SET next_attempt_at = NOW() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();

    let pass = task_graph(&pool)
        .event_ingress(Some(transport.clone()))
        .relay()
        .expect("an outbox and a transport compose a relay")
        .relay_once()
        .await;
    assert_eq!(pass.republished, 1);
    assert_eq!(pass.failed, 0);

    // The republished payload is the whole one, and consuming it commits the
    // message and retires the record.
    let published = transport.published();
    assert_eq!(published.len(), 1);
    assert_eq!(
        published[0].2["message"]["content"][0]["text"],
        "survives the outage"
    );

    let state = test_app_state(pool.clone());
    handle_agent_message(&state, &consumed(task_id, &published[0].0, &published[0].2)).await;

    let texts = message_texts(&pool, task_id).await;
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].as_deref(), Some("survives the outage"));
    assert!(outbox_event_ids(&pool).await.is_empty());
}
