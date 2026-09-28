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
//! Drives the real gRPC ingress and consumer so each case is asserted at the
//! seam the sandbox and consumer meet: a failed transport, a lost gRPC
//! response, concurrent duplicates, and state committed before fan-out.

use crate::common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use tonic::Request;

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
        _agent_profile: Option<&str>,
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

/// An event that bears no turn state, so admission commits its own
/// `task_events` row before fan-out.
fn lifecycle_request(task_id: &str, event_id: &str, sequence: u64) -> PublishEventRequest {
    PublishEventRequest {
        task_id: task_id.to_string(),
        event_type: sandbox_proto::EventType::Agent as i32,
        sub_type: "session_init".to_string(),
        payload_json: r#"{"sessionId":"session-1"}"#.to_string(),
        event_id: event_id.to_string(),
        sequence,
        run_id: String::new(),
    }
}

/// What the event log kept for `event_id`, if it kept anything.
async fn event_log_payload(pool: &PgPool, event_id: &str) -> Option<Value> {
    sqlx::query_scalar("SELECT payload FROM task_events WHERE event_id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .unwrap()
}

// =========================================================================
// Admission → consumer → delete
// =========================================================================

/// `task_events` is the lifecycle log.
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
        "agentProfile": "claude-code",
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
    let request = lifecycle_request(task_id, "evt-1", 7);
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
    assert!(event_log_payload(&pool, "evt-1").await.is_some());
}

/// Holds every publish until released, so admission outlasts its caller.
struct GatedTransport {
    inner: FakeTransport,
    gate: tokio::sync::Semaphore,
    /// Signalled when a publish reaches the gate.
    entered: tokio::sync::Notify,
}

#[async_trait]
impl EventPublisher for GatedTransport {
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
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: Value,
    ) -> Result<(), PublishError> {
        self.entered.notify_one();
        let _permit = self.gate.acquire().await.unwrap();
        self.inner
            .publish_raw_event_with_id(
                event_id,
                task_id,
                event_type,
                sub_type,
                agent_profile,
                turn_number,
                payload,
            )
            .await
    }
}

#[sqlx::test]
async fn a_call_cancelled_mid_admission_is_acknowledged_on_retry(pool: PgPool) {
    let task_id = "task-admit-cancel";
    seed_task(&pool, task_id).await;
    let transport = Arc::new(GatedTransport {
        inner: FakeTransport::default(),
        gate: tokio::sync::Semaphore::new(0),
        entered: tokio::sync::Notify::new(),
    });
    let svc = ingress(&pool, Some(transport.clone()));
    let request = lifecycle_request(task_id, "evt-1", 1);

    // The call is dropped, as the sandbox's deadline drops it, once admission
    // has committed and is blocked publishing.
    tokio::select! {
        _ = svc.publish_event(Request::new(request.clone())) => {
            panic!("the gate holds admission until released");
        }
        () = transport.entered.notified() => {}
    }

    transport.gate.add_permits(1);
    svc.admissions.close();
    svc.admissions.wait().await;

    let retry = svc.publish_event(Request::new(request)).await;
    assert_eq!(retry.unwrap().into_inner().event_id, "evt-1");
    assert_eq!(transport.inner.published().len(), 1);
    assert!(event_log_payload(&pool, "evt-1").await.is_some());
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
            svc.publish_event(Request::new(lifecycle_request(&task_id, "evt-1", 1)))
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
    assert!(event_log_payload(&pool, "evt-1").await.is_some());
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

/// A transport that fails after the row is committed loses nothing: the row is
/// the durable copy, and the sandbox is not asked to resend.
#[sqlx::test]
async fn a_failed_transport_keeps_the_committed_row(pool: PgPool) {
    let task_id = "task-admit-6";
    seed_task(&pool, task_id).await;
    let transport = Arc::new(FakeTransport::failing(1));
    let svc = ingress(&pool, Some(transport.clone()));

    svc.publish_event(Request::new(lifecycle_request(task_id, "evt-1", 1)))
        .await
        .expect("durably admitted despite the failed publish");

    assert!(transport.published().is_empty());
    assert_eq!(
        event_log_payload(&pool, "evt-1").await,
        Some(serde_json::json!({ "sessionId": "session-1" }))
    );
}
