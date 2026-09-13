//! Behavioural tests for the sandbox event send buffer.
//!
//! The admission policy is exercised directly on [`EventBuffer`] where the
//! assertion is about the policy itself; everything else drives the real
//! [`EventOutbox`] + drainer against a controllable backend, because the
//! properties that matter (bounds hold while the drainer is stalled, markers
//! land in stream order, a terminal rejection does not wedge the head) are only
//! observable end to end.

use super::*;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use sandbox_proto::{
    GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery, PublishEventRequest,
};
use tonic::Status;

fn agent_event(sub_type: &str) -> PublishEventRequest {
    PublishEventRequest {
        task_id: "task-1".to_string(),
        event_type: sandbox_proto::EventType::Agent as i32,
        sub_type: sub_type.to_string(),
        payload_json: "{}".to_string(),
        event_id: uuid::Uuid::new_v4().to_string(),
        sequence: 0,
        run_id: String::new(),
    }
}

/// An `agent.message` whose payload is `bytes` long, for byte-cap tests.
fn fat_agent_event(sub_type: &str, bytes: usize) -> PublishEventRequest {
    let mut req = agent_event(sub_type);
    req.payload_json = "x".repeat(bytes);
    req
}

fn task_turn_event(sub_type: &str) -> PublishEventRequest {
    PublishEventRequest {
        task_id: "task-1".to_string(),
        event_type: sandbox_proto::EventType::TaskTurn as i32,
        sub_type: sub_type.to_string(),
        payload_json: "{}".to_string(),
        event_id: uuid::Uuid::new_v4().to_string(),
        sequence: 0,
        run_id: String::new(),
    }
}

/// Controllable backend for outbox tests.
///
/// `fail_first`/`stalled` fail attempts with `UNAVAILABLE`; `reject` answers a
/// sub-type with a terminal status. `attempts` records every request seen,
/// failed included; `received` records only the accepted ones, in order.
struct MockBackend {
    fail_first: Mutex<u32>,
    stalled: AtomicBool,
    reject: Mutex<HashMap<String, Code>>,
    received: Arc<Mutex<Vec<PublishEventRequest>>>,
    attempts: Arc<Mutex<Vec<PublishEventRequest>>>,
}

impl MockBackend {
    fn new(fail_first: u32) -> (Arc<Self>, Arc<Mutex<Vec<PublishEventRequest>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let me = Arc::new(Self {
            fail_first: Mutex::new(fail_first),
            stalled: AtomicBool::new(false),
            reject: Mutex::new(HashMap::new()),
            received: Arc::clone(&received),
            attempts: Arc::new(Mutex::new(Vec::new())),
        });
        (me, received)
    }

    fn stall(&self) {
        self.stalled.store(true, Ordering::SeqCst);
    }

    fn resume(&self) {
        self.stalled.store(false, Ordering::SeqCst);
    }

    fn reject_sub_type(&self, sub_type: &str) {
        self.reject_sub_type_with(sub_type, Code::InvalidArgument);
    }

    fn reject_sub_type_with(&self, sub_type: &str, code: Code) {
        self.reject
            .lock()
            .expect("lock")
            .insert(sub_type.to_string(), code);
    }

    fn reject_sub_type_as_durable_admission_failure(&self, sub_type: &str) {
        self.reject_sub_type_with(sub_type, Code::Unavailable);
    }

    fn attempt_count(&self) -> usize {
        self.attempts.lock().expect("lock").len()
    }

    fn received_sub_types(&self) -> Vec<String> {
        self.received
            .lock()
            .expect("lock")
            .iter()
            .map(|r| r.sub_type.clone())
            .collect()
    }
}

#[async_trait]
impl BackendClient for MockBackend {
    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), Status> {
        self.attempts.lock().expect("lock").push(req.clone());

        let rejection = self
            .reject
            .lock()
            .expect("lock")
            .get(&req.sub_type)
            .copied();
        if let Some(code) = rejection {
            if code == Code::Unavailable {
                return Err(Status::unavailable(DURABLE_ADMISSION_RETRY_MESSAGE));
            }
            return Err(Status::new(code, "mock backend refuses this event"));
        }
        if self.stalled.load(Ordering::SeqCst) {
            return Err(Status::unavailable("mock backend stalled"));
        }
        {
            let mut remaining = self.fail_first.lock().expect("lock");
            if *remaining > 0 {
                *remaining -= 1;
                return Err(Status::unavailable("mock backend unavailable"));
            }
        }
        self.received.lock().expect("lock").push(req);
        Ok(())
    }

    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingAgentDelivery>, Status> {
        Ok(vec![])
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
        _answered_prompt: Option<&str>,
    ) -> Result<(), Status> {
        Ok(())
    }

    async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
        Ok(GetCredentialsResponse::default())
    }

    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), Status> {
        Ok(())
    }

    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
        Ok(String::new())
    }

    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<sandbox_proto::DownloadUploadResponse, Status> {
        Ok(sandbox_proto::DownloadUploadResponse::default())
    }

    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, Status> {
        Ok(None)
    }

    async fn upload_transcript(&self, _task_id: &str, _tar_data: Vec<u8>) -> Result<(), Status> {
        Ok(())
    }

    async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, Status> {
        Ok(vec![])
    }
}

async fn wait_until<F: Fn() -> bool>(cond: F) {
    for _ in 0..1000 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("condition not met within timeout");
}

#[test]
fn backoff_grows_then_caps() {
    assert_eq!(backoff_for(1), BASE_BACKOFF);
    assert_eq!(backoff_for(2), BASE_BACKOFF * 2);
    assert_eq!(backoff_for(3), BASE_BACKOFF * 4);
    // Large attempt counts saturate to the cap, never overflow.
    assert_eq!(backoff_for(30), MAX_BACKOFF);
    assert_eq!(backoff_for(u32::MAX), MAX_BACKOFF);
}

// ---------------------------------------------------------------------------
// Admission policy
// ---------------------------------------------------------------------------

#[test]
fn overflow_drops_oldest_non_lifecycle_keeps_lifecycle() {
    let mut buf = EventBuffer::new(3, usize::MAX);

    // Fill: two non-lifecycle, one lifecycle.
    assert!(
        buf.push(QueuedEvent::new(agent_event("message")))
            .evicted
            .is_empty()
    );
    assert!(
        buf.push(QueuedEvent::new(task_turn_event("active")))
            .evicted
            .is_empty()
    );
    assert!(
        buf.push(QueuedEvent::new(agent_event("message")))
            .evicted
            .is_empty()
    );
    assert_eq!(buf.len(), 3);

    // Over cap: oldest non-lifecycle (index 0) is evicted, not the taskTurn.
    let admission = buf.push(QueuedEvent::new(agent_event("message")));
    assert_eq!(admission.evicted.len(), 1);
    assert_eq!(admission.evicted[0].req.sub_type, "message");
    assert!(admission.rejected.is_none());
    assert_eq!(buf.len(), 3);

    // The surviving lifecycle event is still buffered.
    let remaining: Vec<_> = std::iter::from_fn(|| buf.pop_front())
        .map(|e| e.req.sub_type)
        .collect();
    assert!(remaining.contains(&"active".to_string()));
}

#[test]
fn lifecycle_saturation_refuses_the_newest_and_holds_the_cap() {
    let mut buf = EventBuffer::new(2, usize::MAX);

    assert!(
        buf.push(QueuedEvent::new(task_turn_event("active")))
            .rejected
            .is_none()
    );
    assert!(
        buf.push(QueuedEvent::new(agent_event("completed")))
            .rejected
            .is_none()
    );
    assert_eq!(buf.len(), 2);

    // Nothing evictable is left, so the class is capped by refusing the newest
    // rather than growing past the budget.
    let admission = buf.push(QueuedEvent::new(agent_event("error")));
    assert!(admission.evicted.is_empty());
    assert_eq!(
        admission.rejected.map(|e| e.req.sub_type),
        Some("error".to_string())
    );
    assert_eq!(buf.len(), 2);

    // The older prefix survives, in order — that is what makes later lifecycle
    // events interpretable.
    let remaining: Vec<_> = std::iter::from_fn(|| buf.pop_front())
        .map(|e| e.req.sub_type)
        .collect();
    assert_eq!(remaining, vec!["active", "completed"]);
}

#[test]
fn byte_cap_triggers_eviction_but_never_empties_the_buffer() {
    // Tiny byte cap, generous event cap: eviction driven purely by bytes.
    let mut buf = EventBuffer::new(1000, 40);
    // Each agent_event: payload "{}" (2) + sub_type "message" (7) + task "task-1" (6)
    // + a uuid event_id (36) = 51 bytes, already over the 40-byte cap, but a
    // single event is never dropped below itself — it's the only entry.
    let admission = buf.push(QueuedEvent::new(agent_event("message")));
    assert!(admission.evicted.is_empty());
    assert!(admission.rejected.is_none());
    assert_eq!(buf.len(), 1);
    // A second push must evict the first to try to get back under budget.
    let admission = buf.push(QueuedEvent::new(agent_event("message")));
    assert_eq!(admission.evicted.len(), 1);
    assert!(admission.rejected.is_none());
    assert_eq!(buf.len(), 1);
}

#[test]
fn a_lone_oversized_lifecycle_event_is_kept_not_refused() {
    let mut buf = EventBuffer::new(1000, 10);
    let admission = buf.push(QueuedEvent::new(task_turn_event("active")));
    assert!(admission.rejected.is_none());
    assert_eq!(buf.len(), 1);
}

// ---------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn drains_events_in_order_with_sequences() {
    let (backend, received) = MockBackend::new(0);
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

    for i in 0..5 {
        outbox.enqueue(agent_event(&format!("message-{i}")));
    }

    wait_until(|| received.lock().expect("lock").len() == 5).await;

    let got = received.lock().expect("lock");
    let sub_types: Vec<_> = got.iter().map(|r| r.sub_type.clone()).collect();
    assert_eq!(
        sub_types,
        vec![
            "message-0",
            "message-1",
            "message-2",
            "message-3",
            "message-4"
        ]
    );
    // Sequences are assigned in transmission order, contiguous from 1.
    let seqs: Vec<_> = got.iter().map(|r| r.sequence).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    assert_eq!(outbox.pending_count(), 0);
    assert_eq!(outbox.metrics().delivered, 5);

    cancel.cancel();
}

#[tokio::test(start_paused = true)]
async fn retries_reuse_a_stable_event_id_and_sequence() {
    // Fail the first 3 attempts; the drainer must retry the same event and
    // deliver it exactly once, preserving its identity. The backend's
    // deduplication and idempotent persistence both key on that identity
    // (the durable-ingress decision), so every attempt — not just the accepted one — has to
    // carry the same non-empty id and the same sequence.
    let (backend, received) = MockBackend::new(3);
    let attempts = Arc::clone(&backend.attempts);
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

    let ev = agent_event("completed");
    let event_id = ev.event_id.clone();
    outbox.enqueue(ev);

    wait_until(|| received.lock().expect("lock").len() == 1).await;

    let got = received.lock().expect("lock");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].event_id, event_id);
    assert_eq!(got[0].sequence, 1);
    assert_eq!(outbox.pending_count(), 0);

    let attempts = attempts.lock().expect("lock");
    assert_eq!(attempts.len(), 4, "3 failures then the delivery");
    for attempt in attempts.iter() {
        assert!(!attempt.event_id.is_empty(), "an event id is always set");
        assert_eq!(attempt.event_id, event_id);
        assert_eq!(attempt.sequence, 1);
    }
    // Each retryable failure is counted, so "retried" is distinguishable from
    // "dropped" and from "failed admission".
    assert_eq!(outbox.metrics().retried, 3);

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Bounds hold while the drainer is stalled
// ---------------------------------------------------------------------------

/// The acceptance criterion: capacity is enforced when the producer submits,
/// *including* while the drainer is wedged on an unreachable backend.
#[tokio::test(start_paused = true)]
async fn producer_burst_against_a_stalled_backend_respects_the_event_cap() {
    let (backend, _received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn_with_caps(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
        4,
        usize::MAX,
    );

    // Wedge the drainer on its first event so the burst below lands entirely on
    // the producer side of the queue.
    outbox.enqueue(agent_event("head"));
    wait_until(|| backend.attempt_count() >= 1).await;

    for i in 0..200 {
        outbox.enqueue(agent_event(&format!("message-{i}")));
        // The bound is enforced on every single submission, not eventually.
        assert!(
            outbox.pending_count() <= 5,
            "buffer ({}) grew past cap + the one in-flight event",
            outbox.pending_count()
        );
    }

    let metrics = outbox.metrics();
    assert_eq!(metrics.delivered, 0, "the backend never accepted anything");
    assert_eq!(metrics.dropped_overflow, 196, "200 sent, 4 retained");
    assert_eq!(metrics.undrained, 5);

    cancel.cancel();
}

#[tokio::test(start_paused = true)]
async fn producer_burst_against_a_stalled_backend_respects_the_byte_cap() {
    let (backend, _received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    // Room for ~4 fat events; the event cap is effectively off.
    const FAT: usize = 1024;
    let (outbox, _drainer) = EventOutbox::spawn_with_caps(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
        usize::MAX,
        4 * FAT,
    );

    outbox.enqueue(fat_agent_event("head", FAT));
    wait_until(|| backend.attempt_count() >= 1).await;

    for i in 0..100 {
        outbox.enqueue(fat_agent_event(&format!("message-{i}"), FAT));
        assert!(
            outbox.pending_count() <= 5,
            "byte budget let {} events accumulate",
            outbox.pending_count()
        );
    }

    assert!(outbox.metrics().dropped_overflow > 0);

    cancel.cancel();
}

/// Lifecycle events are never evicted, so they are the class that could grow
/// without bound. They do not: at saturation admission is refused, and the
/// refusal is counted separately from a capacity eviction.
#[tokio::test(start_paused = true)]
async fn lifecycle_only_saturation_refuses_admission_rather_than_growing() {
    let (backend, _received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn_with_caps(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
        3,
        usize::MAX,
    );

    outbox.enqueue(task_turn_event("active"));
    wait_until(|| backend.attempt_count() >= 1).await;

    for _ in 0..50 {
        outbox.enqueue(agent_event("completed"));
        assert!(
            outbox.pending_count() <= 4,
            "lifecycle class grew past the cap ({})",
            outbox.pending_count()
        );
    }

    let metrics = outbox.metrics();
    assert_eq!(metrics.dropped_overflow, 0, "nothing was evictable");
    assert_eq!(metrics.rejected_saturated, 47, "50 sent, 3 retained");
    assert_eq!(metrics.undrained, 4);

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Drop accounting and markers
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn overflow_emits_a_dropped_marker_before_the_next_event() {
    let (backend, received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn_with_caps(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
        2,
        usize::MAX,
    );

    outbox.enqueue(agent_event("head"));
    wait_until(|| backend.attempt_count() >= 1).await;
    for i in 0..10 {
        outbox.enqueue(agent_event(&format!("message-{i}")));
    }
    wait_until(|| outbox.metrics().dropped_overflow > 0).await;

    backend.resume();
    wait_until(|| {
        received
            .lock()
            .expect("lock")
            .iter()
            .any(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
    })
    .await;

    let got = received.lock().expect("lock");
    let marker_idx = got
        .iter()
        .position(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
        .expect("marker present");
    let marker = &got[marker_idx];
    assert!(marker.payload_json.contains("outbox_overflow"));
    // Deterministic ordering: the marker is announced before the next real
    // event, so the gap sits where it happened rather than at the tail.
    assert_eq!(
        got[marker_idx - 1].sub_type,
        "head",
        "the marker follows the event that was in flight when the drops began"
    );
    assert!(marker_idx + 1 < got.len(), "real events follow the marker");
    assert_ne!(got[marker_idx + 1].sub_type, EVENTS_DROPPED_SUB_TYPE);

    cancel.cancel();
}

/// A terminally-rejected event must not wedge the head of the queue. Before
/// this, `deliver` retried every `Err(status)` forever, so one malformed event
/// took out the whole task's stream.
#[tokio::test(start_paused = true)]
async fn a_terminal_rejection_is_dropped_and_the_stream_continues() {
    let (backend, received) = MockBackend::new(0);
    backend.reject_sub_type("poison");
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );

    outbox.enqueue(agent_event("poison"));
    outbox.enqueue(agent_event("after"));

    wait_until(|| backend.received_sub_types().contains(&"after".to_string())).await;

    let metrics = outbox.metrics();
    assert_eq!(metrics.dropped_terminal, 1);
    assert_eq!(metrics.retried, 0, "a terminal rejection is never retried");
    assert!(
        !received
            .lock()
            .expect("lock")
            .iter()
            .any(|r| r.sub_type == "poison")
    );

    // The gap is announced, not silently swallowed.
    wait_until(|| {
        backend
            .received_sub_types()
            .contains(&EVENTS_DROPPED_SUB_TYPE.to_string())
    })
    .await;
    let got = received.lock().expect("lock");
    let marker = got
        .iter()
        .find(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
        .expect("marker present");
    assert!(marker.payload_json.contains("backend_rejected"));

    cancel.cancel();
}

/// An over-limit payload can never succeed on resend.
///
/// Reachable in practice: neither end of the publish path caps gRPC size, so
/// the backend's tonic 4 MiB default can be crossed by a large `agent.message`.
#[tokio::test(start_paused = true)]
async fn an_over_limit_payload_is_dropped_rather_than_retried_forever() {
    let (backend, received) = MockBackend::new(0);
    backend.reject_sub_type_with("oversized", Code::OutOfRange);
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );

    outbox.enqueue(agent_event("oversized"));
    outbox.enqueue(agent_event("after"));

    wait_until(|| backend.received_sub_types().contains(&"after".to_string())).await;

    let metrics = outbox.metrics();
    assert_eq!(metrics.dropped_terminal, 1);
    assert_eq!(metrics.retried, 0, "an over-limit payload is never retried");
    assert!(
        !received
            .lock()
            .expect("lock")
            .iter()
            .any(|r| r.sub_type == "oversized")
    );

    cancel.cancel();
}

/// Quota and rate limiting share `RESOURCE_EXHAUSTED`, and nothing about it is
/// permanent, so it must stay on the retry path — the boundary that keeps the
/// fix above from over-reaching into codes backoff can actually clear.
#[tokio::test(start_paused = true)]
async fn resource_exhausted_is_retried_not_dropped() {
    let (backend, received) = MockBackend::new(0);
    backend.reject_sub_type_with("throttled", Code::ResourceExhausted);
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );

    outbox.enqueue(agent_event("throttled"));
    wait_until(|| outbox.metrics().retried >= 2).await;

    assert_eq!(
        outbox.metrics().dropped_terminal,
        0,
        "a throttled event is held for retry, not dropped"
    );
    assert!(received.lock().expect("lock").is_empty());

    cancel.cancel();
}

/// The production failure mode: the backend accepts but cannot commit to its
/// durable ingress path, returning `UNAVAILABLE`.
///
/// A bounded retry tolerates a transient blip without wedging later events.
#[tokio::test(start_paused = true)]
async fn repeated_durable_admission_failures_are_dropped_rather_than_wedging_the_stream() {
    let (backend, received) = MockBackend::new(0);
    backend.reject_sub_type_as_durable_admission_failure("poison");
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );

    outbox.enqueue(agent_event("poison"));
    outbox.enqueue(agent_event("after"));

    tokio::task::yield_now().await;
    for _ in 0..(MAX_DURABLE_ADMISSION_RETRIES + 2) {
        tokio::time::advance(MAX_BACKOFF).await;
        tokio::task::yield_now().await;
        if backend.received_sub_types().contains(&"after".to_string()) {
            break;
        }
    }
    wait_until(|| backend.received_sub_types().contains(&"after".to_string())).await;

    let metrics = outbox.metrics();
    assert_eq!(metrics.dropped_terminal, 1);
    assert_eq!(
        metrics.retried,
        u64::from(MAX_DURABLE_ADMISSION_RETRIES - 1),
        "the poison event gets a bounded retry budget before being dropped"
    );
    assert!(
        !received
            .lock()
            .expect("lock")
            .iter()
            .any(|r| r.sub_type == "poison")
    );

    wait_until(|| {
        backend
            .received_sub_types()
            .contains(&EVENTS_DROPPED_SUB_TYPE.to_string())
    })
    .await;
    let got = received.lock().expect("lock");
    let marker = got
        .iter()
        .find(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
        .expect("marker present");
    assert!(marker.payload_json.contains("backend_rejected"));

    cancel.cancel();
}

/// A refusal at lifecycle saturation is always a lifecycle event — the most
/// consequential loss of the four — so it must be as visible on the stream as a
/// capacity eviction, not counter-only.
#[tokio::test(start_paused = true)]
async fn lifecycle_saturation_emits_its_own_marker() {
    let (backend, received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn_with_caps(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
        2,
        usize::MAX,
    );

    outbox.enqueue(task_turn_event("active"));
    wait_until(|| backend.attempt_count() >= 1).await;
    for _ in 0..10 {
        outbox.enqueue(agent_event("completed"));
    }
    wait_until(|| outbox.metrics().rejected_saturated > 0).await;
    assert_eq!(
        outbox.metrics().dropped_overflow,
        0,
        "nothing was evictable; this is a refusal, not an eviction"
    );

    backend.resume();
    wait_until(|| {
        backend
            .received_sub_types()
            .contains(&EVENTS_DROPPED_SUB_TYPE.to_string())
    })
    .await;

    let got = received.lock().expect("lock");
    let marker = got
        .iter()
        .find(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
        .expect("marker present");
    assert!(
        marker.payload_json.contains("outbox_saturated"),
        "saturation marker carries its own reason: {}",
        marker.payload_json
    );

    cancel.cancel();
}

/// The producer-side drop class: an event whose type cannot go on the wire
/// never enters the buffer, but it is still a gap and is measured and
/// announced.
#[tokio::test(start_paused = true)]
async fn an_unrepresentable_event_is_counted_and_announced() {
    let (backend, received) = MockBackend::new(0);
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

    outbox.record_unrepresentable();

    wait_until(|| {
        received
            .lock()
            .expect("lock")
            .iter()
            .any(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
    })
    .await;

    assert_eq!(outbox.metrics().dropped_unrepresentable, 1);
    let got = received.lock().expect("lock");
    let marker = got
        .iter()
        .find(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
        .expect("marker present");
    assert!(marker.payload_json.contains("unrepresentable_event_type"));
    assert!(marker.sequence > 0, "markers ride the ordered stream");

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Shutdown
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn flush_returns_once_drained() {
    let (backend, received) = MockBackend::new(0);
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

    for i in 0..3 {
        outbox.enqueue(agent_event(&format!("message-{i}")));
    }
    outbox.flush(Duration::from_secs(5)).await;

    assert_eq!(outbox.pending_count(), 0);
    assert_eq!(received.lock().expect("lock").len(), 3);

    cancel.cancel();
}

/// A drop that has been recorded but not yet announced is outstanding work: the
/// buffer is empty, so an emptiness check alone would let shutdown proceed and
/// lose the marker that tells the reader a gap happened.
#[tokio::test(start_paused = true)]
async fn flush_waits_for_a_pending_drop_marker() {
    let (backend, received) = MockBackend::new(0);
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

    // Nothing is buffered — the marker is the only thing left to deliver.
    outbox.record_unrepresentable();
    assert_eq!(outbox.pending_count(), 0);

    outbox.flush(Duration::from_secs(5)).await;

    assert!(
        received
            .lock()
            .expect("lock")
            .iter()
            .any(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE),
        "flush returned before the marker reached the backend"
    );

    cancel.cancel();
}

/// ...and that wait is still bounded: marker debt that can never be settled
/// costs the flush budget, not the grace period.
#[tokio::test(start_paused = true)]
async fn flush_stays_bounded_when_a_drop_marker_cannot_be_delivered() {
    let (backend, _received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );

    outbox.record_unrepresentable();

    let started = tokio::time::Instant::now();
    outbox.flush(Duration::from_secs(2)).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_secs(2),
        "flush returned after {elapsed:?} — it did not count the marker as work"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "flush ran for {elapsed:?}, past its budget"
    );

    cancel.cancel();
}

/// Graceful flush stays bounded: a stalled backend must not hold the pod past
/// its termination grace period.
#[tokio::test(start_paused = true)]
async fn flush_is_bounded_when_the_backend_never_recovers() {
    let (backend, _received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );

    outbox.enqueue(agent_event("message"));

    let started = tokio::time::Instant::now();
    outbox.flush(Duration::from_secs(2)).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(3),
        "flush ran for {elapsed:?}, past its budget"
    );
    assert_eq!(outbox.pending_count(), 1, "the event is still queued");

    cancel.cancel();
}

/// Cancellation during a retry backoff terminates the drainer promptly, leaves
/// the undelivered event at the head, and is observable through the handle.
#[tokio::test(start_paused = true)]
async fn cancellation_during_retry_stops_the_drainer_and_keeps_the_event() {
    let (backend, _received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, drainer) = EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );

    outbox.enqueue(agent_event("completed"));
    wait_until(|| backend.attempt_count() >= 2).await;

    cancel.cancel();
    drainer.shutdown(Duration::from_secs(5)).await;

    // The event was neither delivered nor dropped: it stays undrained, and that
    // is what the metric says.
    let metrics = outbox.metrics();
    assert_eq!(metrics.undrained, 1);
    assert_eq!(metrics.delivered, 0);
    assert_eq!(metrics.dropped_overflow, 0);
    assert_eq!(metrics.dropped_terminal, 0);
}

/// Producers submitting while the drainer terminates must land in exactly one
/// bucket.
///
/// Pins the counter arithmetic; admission closes only once the drainer is gone,
/// and a raced buffer just reports undrained rather than failing the test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admission_racing_drainer_termination_accounts_for_every_event() {
    const PRODUCERS: usize = 4;
    const PER_PRODUCER: usize = 250;
    const AFTER: u64 = 100;

    for _round in 0..10 {
        let (backend, _received) = MockBackend::new(0);
        let cancel = CancellationToken::new();
        let (outbox, drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

        let producers: Vec<_> = (0..PRODUCERS)
            .map(|p| {
                let outbox = outbox.clone();
                tokio::spawn(async move {
                    for i in 0..PER_PRODUCER {
                        outbox.enqueue(agent_event(&format!("p{p}-{i}")));
                        if i % 16 == 0 {
                            tokio::task::yield_now().await;
                        }
                    }
                })
            })
            .collect();

        // Cancel mid-burst: this is the window where a producer can observe a
        // running drainer and find it terminated a moment later.
        tokio::time::sleep(Duration::from_millis(2)).await;
        cancel.cancel();

        for producer in producers {
            producer.await.expect("producer task");
        }
        drainer.shutdown(Duration::from_secs(5)).await;

        // Nothing vanished and nothing was counted twice.
        let settled = outbox.metrics();
        let accounted = settled.delivered
            + settled.dropped_overflow
            + settled.dropped_terminal
            + settled.rejected_saturated
            + settled.rejected_shutdown
            + settled.undrained as u64;
        assert_eq!(
            accounted,
            (PRODUCERS * PER_PRODUCER) as u64,
            "submitted events unaccounted for: {settled:?}"
        );

        // The drainer is gone, so admission is closed: nothing more can join the
        // buffer, and every further submission is refused under its own reason.
        for i in 0..AFTER {
            outbox.enqueue(agent_event(&format!("post-{i}")));
        }
        let after = outbox.metrics();
        assert_eq!(
            after.undrained, settled.undrained,
            "an event was buffered after the drainer terminated"
        );
        assert_eq!(after.rejected_shutdown, settled.rejected_shutdown + AFTER);
    }
}

/// An event evicted on the way in never entered the buffer, and says so. It is
/// the newest, so eviction reaches it only when the buffer holds nothing else
/// evictable — which a lifecycle-heavy backlog is exactly the shape of.
#[tokio::test(start_paused = true)]
async fn an_event_evicted_on_arrival_reports_that_it_was_not_taken() {
    let (backend, _received) = MockBackend::new(0);
    backend.stall();
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn_with_caps(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
        1,
        usize::MAX,
    );

    // One in flight against the stalled backend, one filling the buffer, both
    // lifecycle — so the message arriving next is the only evictable event.
    outbox.enqueue(task_turn_event("active"));
    wait_until(|| backend.attempt_count() >= 1).await;
    assert!(outbox.enqueue(task_turn_event("completed")));

    assert!(
        !outbox.enqueue(agent_event("message")),
        "the event was evicted to keep the caps, so nothing will send it"
    );
    assert_eq!(outbox.metrics().dropped_overflow, 1);

    cancel.cancel();
}

/// Once the drainer has terminated, admission is refused rather than silently
/// buffered into a queue with no consumer.
#[tokio::test(start_paused = true)]
async fn admission_is_refused_after_the_drainer_terminates() {
    let (backend, _received) = MockBackend::new(0);
    let cancel = CancellationToken::new();
    let (outbox, drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

    cancel.cancel();
    drainer.shutdown(Duration::from_secs(5)).await;

    assert!(
        !outbox.enqueue(agent_event("message")),
        "a refused submission says so, so a caller with something to undo can"
    );

    let metrics = outbox.metrics();
    assert_eq!(metrics.rejected_shutdown, 1);
    assert_eq!(metrics.undrained, 0);

    // A flush against a dead drainer returns immediately instead of burning the
    // whole timeout.
    outbox.flush(Duration::from_secs(30)).await;
}
