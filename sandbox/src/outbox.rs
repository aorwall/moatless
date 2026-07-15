//! At-least-once event outbox for the sandbox → backend event hop.
//!
//! `GrpcEventPublisher::publish_event` used to be fire-and-forget: a single
//! gRPC attempt whose failures were logged at `warn` and dropped. During any
//! backend unavailability (rollout, network blip) every `agent.message` /
//! `agent.completed` / `taskTurn.*` event for that window was permanently lost.
//! Everything downstream (Redis consumer groups, `task_events`, the reaper) is
//! at-least-once, but only for events that entered the pipeline — so a lost
//! `taskTurn/active` made a stuck turn invisible to the reaper.
//!
//! This module makes the hop at-least-once. `enqueue` is non-blocking and never
//! fails the caller; a single background drainer task sends events in order with
//! exponential backoff (capped at [`MAX_BACKOFF`]), preserving per-task ordering
//! and assigning a monotonically increasing `sequence` in transmission order so
//! the backend can spot duplicates/restarts.
//!
//! The queue is bounded ([`MAX_QUEUE_EVENTS`] / [`MAX_QUEUE_BYTES`]). On overflow
//! the oldest *non-lifecycle* events are dropped first — `taskTurn.*` and
//! terminal `agent.*` (`completed`, `error`, `stopped`) are always kept — and a synthetic
//! [`EVENTS_DROPPED_SUB_TYPE`] marker event is emitted so the gap stays visible.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sandbox_proto::PublishEventRequest;
use tokio::sync::{mpsc, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::backend_client::BackendClient;

/// Max number of events buffered before overflow eviction begins.
pub const MAX_QUEUE_EVENTS: usize = 10_000;
/// Max total payload bytes buffered before overflow eviction begins (~32 MiB).
pub const MAX_QUEUE_BYTES: usize = 32 * 1024 * 1024;

/// Backoff applied before the first retry; doubles each attempt up to the cap.
const BASE_BACKOFF: Duration = Duration::from_millis(500);
/// Ceiling on the exponential backoff between retries.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// `sub_type` of the synthetic marker emitted after an overflow drop.
pub const EVENTS_DROPPED_SUB_TYPE: &str = "events_dropped";

/// Compute the retry backoff for a given 1-based attempt number, capped at
/// [`MAX_BACKOFF`]. `attempt == 1` yields [`BASE_BACKOFF`].
fn backoff_for(attempt: u32) -> Duration {
    // Saturating shift keeps large attempt counts from overflowing before the
    // `.min` clamps them to the cap.
    let factor = 1u64
        .checked_shl(attempt.saturating_sub(1))
        .unwrap_or(u64::MAX);
    BASE_BACKOFF
        .saturating_mul(factor.min(u32::MAX as u64) as u32)
        .min(MAX_BACKOFF)
}

/// Whether an event must never be dropped on overflow.
///
/// Lifecycle events drive downstream reconciliation (the reaper keys off the
/// `taskTurn/active` row, terminal `agent` events close turns) so losing them
/// silently corrupts task state — they are retained even past the byte cap.
fn is_lifecycle(req: &PublishEventRequest) -> bool {
    use sandbox_proto::EventType;
    match EventType::try_from(req.event_type) {
        Ok(EventType::TaskTurn) => true,
        Ok(EventType::Agent) => {
            matches!(req.sub_type.as_str(), "completed" | "error" | "stopped")
        }
        _ => false,
    }
}

/// Rough in-memory footprint of a queued request, used for the byte budget.
fn request_size(req: &PublishEventRequest) -> usize {
    req.payload_json.len() + req.sub_type.len() + req.task_id.len() + req.event_id.len()
}

/// A queued event plus bookkeeping the drainer needs.
struct QueuedEvent {
    req: PublishEventRequest,
    /// Cached [`request_size`] so eviction can adjust the byte budget in O(1).
    size: usize,
    /// Sequence assigned on the first send attempt, reused across retries so a
    /// retried delivery carries the same `(event_id, sequence)` identity.
    sequence: Option<u64>,
}

impl QueuedEvent {
    fn new(req: PublishEventRequest) -> Self {
        let size = request_size(&req);
        Self {
            req,
            size,
            sequence: None,
        }
    }
}

/// Bounded FIFO buffer with the overflow-eviction policy.
///
/// Kept free of the drainer's async/atomic machinery so the overflow policy is
/// unit-testable in isolation. `push` returns the events evicted to stay within
/// the caps (oldest non-lifecycle first); the caller accounts for them.
struct EventBuffer {
    buf: VecDeque<QueuedEvent>,
    bytes: usize,
    max_events: usize,
    max_bytes: usize,
}

impl EventBuffer {
    fn new(max_events: usize, max_bytes: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            bytes: 0,
            max_events,
            max_bytes,
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.buf.len()
    }

    fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Append `ev`, then evict the oldest non-lifecycle events until back within
    /// the length and byte caps. Returns the evicted events (in eviction order).
    /// When only lifecycle events remain the caps are intentionally exceeded
    /// rather than dropping a lifecycle event.
    fn push(&mut self, ev: QueuedEvent) -> Vec<QueuedEvent> {
        self.bytes += ev.size;
        self.buf.push_back(ev);

        let mut dropped = Vec::new();
        // Never evict the last remaining event: a lone event larger than the
        // byte cap must still be deliverable rather than dropped into oblivion.
        while (self.buf.len() > self.max_events || self.bytes > self.max_bytes)
            && self.buf.len() > 1
        {
            let Some(idx) = self.buf.iter().position(|e| !is_lifecycle(&e.req)) else {
                // Only lifecycle events left — keep them all.
                break;
            };
            // `idx` came from `position()` on this same buffer, so `remove`
            // always yields `Some`; fall through defensively if it somehow can't.
            let Some(removed) = self.buf.remove(idx) else {
                break;
            };
            self.bytes -= removed.size;
            dropped.push(removed);
        }
        dropped
    }

    fn pop_front(&mut self) -> Option<QueuedEvent> {
        let ev = self.buf.pop_front()?;
        self.bytes -= ev.size;
        Some(ev)
    }

    fn push_front(&mut self, ev: QueuedEvent) {
        self.bytes += ev.size;
        self.buf.push_front(ev);
    }
}

/// Cloneable producer handle to the outbox.
///
/// Cheap to clone (all shared state is `Arc`-wrapped). Held by
/// [`crate::events::GrpcEventPublisher`]; `enqueue` is the only hot-path method.
#[derive(Clone)]
pub struct EventOutbox {
    tx: mpsc::UnboundedSender<QueuedEvent>,
    /// Total events dropped due to overflow (drives the marker + observability).
    dropped: Arc<AtomicU64>,
    /// Events buffered but not yet delivered or dropped (mpsc + drainer deque).
    /// Reaches 0 exactly when the outbox is fully drained.
    pending: Arc<AtomicUsize>,
    /// Wakes the drainer out of a backoff sleep so `flush` drains promptly.
    wake: Arc<Notify>,
}

impl EventOutbox {
    /// Build the outbox and spawn its single drainer task.
    ///
    /// The drainer runs until `cancel` fires (sandbox lifecycle end) or all
    /// producer handles drop. `task_id` labels the synthetic overflow marker.
    pub fn spawn(
        client: Arc<dyn BackendClient>,
        task_id: String,
        cancel: CancellationToken,
    ) -> Self {
        Self::spawn_with_caps(client, task_id, cancel, MAX_QUEUE_EVENTS, MAX_QUEUE_BYTES)
    }

    fn spawn_with_caps(
        client: Arc<dyn BackendClient>,
        task_id: String,
        cancel: CancellationToken,
        max_events: usize,
        max_bytes: usize,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let dropped = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(AtomicUsize::new(0));
        let wake = Arc::new(Notify::new());

        let drainer = Drainer {
            client,
            rx,
            buffer: EventBuffer::new(max_events, max_bytes),
            seq: 0,
            reported_drops: 0,
            task_id,
            dropped: Arc::clone(&dropped),
            pending: Arc::clone(&pending),
            wake: Arc::clone(&wake),
            cancel,
        };
        tokio::spawn(drainer.run());

        Self {
            tx,
            dropped,
            pending,
            wake,
        }
    }

    /// Enqueue an event for at-least-once delivery. Non-blocking; never fails
    /// the caller. If the drainer has already stopped (final shutdown) the event
    /// is dropped with a debug log.
    pub fn enqueue(&self, req: PublishEventRequest) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(QueuedEvent::new(req)).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            debug!("event outbox drainer stopped; event dropped");
        }
    }

    /// Number of events buffered but not yet delivered or dropped.
    pub fn pending_count(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// Total events dropped due to overflow across the outbox's lifetime.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::SeqCst)
    }

    /// Wait until the queue is fully drained or `timeout` elapses, whichever
    /// comes first. Called on graceful shutdown before the transcript flush.
    /// Best-effort: on timeout it logs and returns, leaving the drainer to keep
    /// retrying in the background until the process exits.
    pub async fn flush(&self, timeout: Duration) {
        let drained = tokio::time::timeout(timeout, async {
            loop {
                if self.pending.load(Ordering::SeqCst) == 0 {
                    return;
                }
                // Nudge the drainer out of any backoff sleep, then re-poll. A
                // short poll avoids the lost-wakeup race a bare Notify would have.
                self.wake.notify_waiters();
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;

        if drained.is_err() {
            warn!(
                pending = self.pending.load(Ordering::SeqCst),
                "event outbox flush timed out with events still queued"
            );
        }
    }
}

/// Outcome of a (retrying) delivery attempt.
enum Delivery {
    /// The event was acknowledged by the backend.
    Sent,
    /// The drainer was cancelled mid-delivery (shutdown).
    Cancelled,
}

/// The single background task that owns the buffer and drives delivery.
struct Drainer {
    client: Arc<dyn BackendClient>,
    rx: mpsc::UnboundedReceiver<QueuedEvent>,
    buffer: EventBuffer,
    /// Last sequence assigned; incremented in transmission order.
    seq: u64,
    /// Overflow drops already announced via a marker event.
    reported_drops: u64,
    task_id: String,
    dropped: Arc<AtomicU64>,
    pending: Arc<AtomicUsize>,
    wake: Arc<Notify>,
    cancel: CancellationToken,
}

impl Drainer {
    async fn run(mut self) {
        loop {
            self.absorb_ready();

            if self.buffer.is_empty() {
                tokio::select! {
                    () = self.cancel.cancelled() => break,
                    () = self.wake.notified() => continue,
                    item = self.rx.recv() => match item {
                        Some(ev) => self.accept(ev),
                        None => break, // all producer handles dropped
                    },
                }
                continue;
            }

            // Announce any overflow drops before the next real event so the gap
            // is visible in stream order. Coalesced: one marker per batch of drops.
            let total_dropped = self.dropped.load(Ordering::SeqCst);
            if total_dropped > self.reported_drops {
                let count = total_dropped - self.reported_drops;
                let marker = self.build_marker(count);
                match self.deliver(marker).await {
                    Delivery::Sent => self.reported_drops = total_dropped,
                    Delivery::Cancelled => break,
                }
            }

            let Some(mut ev) = self.buffer.pop_front() else {
                continue;
            };
            if ev.sequence.is_none() {
                self.seq += 1;
                ev.sequence = Some(self.seq);
            }
            ev.req.sequence = ev.sequence.unwrap_or(0);

            match self.deliver(ev.req.clone()).await {
                Delivery::Sent => {
                    self.pending.fetch_sub(1, Ordering::SeqCst);
                }
                Delivery::Cancelled => {
                    // Preserve ordering for any in-flight flush already waiting.
                    self.buffer.push_front(ev);
                    break;
                }
            }
        }
    }

    /// Move every currently-queued event from the mpsc into the buffer,
    /// applying the overflow policy as we go.
    fn absorb_ready(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.accept(ev);
        }
    }

    /// Push one event into the buffer and account for any overflow eviction.
    fn accept(&mut self, ev: QueuedEvent) {
        for removed in self.buffer.push(ev) {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            self.pending.fetch_sub(1, Ordering::SeqCst);
            warn!(
                sub_type = %removed.req.sub_type,
                "event outbox overflow: dropped oldest non-lifecycle event"
            );
        }
    }

    /// Send one request, retrying with exponential backoff until it is
    /// acknowledged or the drainer is cancelled. Keeps absorbing newly enqueued
    /// events between attempts so the mpsc doesn't grow unbounded during a stall.
    async fn deliver(&mut self, req: PublishEventRequest) -> Delivery {
        let mut attempt: u32 = 0;
        loop {
            if self.cancel.is_cancelled() {
                return Delivery::Cancelled;
            }
            self.absorb_ready();

            match self.client.publish_event(req.clone()).await {
                Ok(()) => return Delivery::Sent,
                Err(status) => {
                    attempt += 1;
                    let backoff = backoff_for(attempt);
                    warn!(
                        task_id = %req.task_id,
                        sub_type = %req.sub_type,
                        attempt,
                        backoff_ms = backoff.as_millis() as u64,
                        error = %status,
                        "event publish failed; retrying"
                    );
                    tokio::select! {
                        () = self.cancel.cancelled() => return Delivery::Cancelled,
                        () = self.wake.notified() => {}
                        () = tokio::time::sleep(backoff) => {}
                        item = self.rx.recv() => {
                            if let Some(ev) = item {
                                self.accept(ev);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Build the synthetic overflow marker. It rides the same ordered stream as
    /// real events (its own `event_id` + sequence) so the backend records the
    /// gap without special-casing.
    fn build_marker(&self, count: u64) -> PublishEventRequest {
        let payload = serde_json::json!({
            "reason": "outbox_overflow",
            "dropped_count": count,
        });
        let proto_event_type: sandbox_proto::EventType = moatless_events::EventType::Sandbox.into();
        PublishEventRequest {
            task_id: self.task_id.clone(),
            event_type: proto_event_type as i32,
            sub_type: EVENTS_DROPPED_SUB_TYPE.to_string(),
            payload_json: payload.to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use async_trait::async_trait;
    use sandbox_proto::{
        GetCredentialsResponse, InterruptedTurn, PendingMessage, PublishEventRequest,
    };
    use tonic::Status;

    fn agent_event(sub_type: &str) -> PublishEventRequest {
        let et: sandbox_proto::EventType = moatless_events::EventType::Agent.into();
        PublishEventRequest {
            task_id: "task-1".to_string(),
            event_type: et as i32,
            sub_type: sub_type.to_string(),
            payload_json: "{}".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
        }
    }

    fn task_turn_event(sub_type: &str) -> PublishEventRequest {
        let et: sandbox_proto::EventType = moatless_events::EventType::TaskTurn.into();
        PublishEventRequest {
            task_id: "task-1".to_string(),
            event_type: et as i32,
            sub_type: sub_type.to_string(),
            payload_json: "{}".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
        }
    }

    /// Mock backend client: fails the first `fail_first` publish attempts, then
    /// records every subsequently accepted request in order.
    struct MockBackend {
        fail_first: Mutex<u32>,
        received: Arc<Mutex<Vec<PublishEventRequest>>>,
    }

    impl MockBackend {
        fn new(fail_first: u32) -> (Arc<Self>, Arc<Mutex<Vec<PublishEventRequest>>>) {
            let received = Arc::new(Mutex::new(Vec::new()));
            let me = Arc::new(Self {
                fail_first: Mutex::new(fail_first),
                received: Arc::clone(&received),
            });
            (me, received)
        }
    }

    #[async_trait]
    impl BackendClient for MockBackend {
        async fn publish_event(&self, req: PublishEventRequest) -> Result<(), Status> {
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
        ) -> Result<Vec<PendingMessage>, Status> {
            Ok(vec![])
        }

        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _message_uuids: Vec<String>,
            _session_id: &str,
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

        async fn upload_transcript(
            &self,
            _task_id: &str,
            _tar_data: Vec<u8>,
        ) -> Result<(), Status> {
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

    #[test]
    fn overflow_drops_oldest_non_lifecycle_keeps_lifecycle() {
        let mut buf = EventBuffer::new(3, usize::MAX);

        // Fill: two non-lifecycle, one lifecycle.
        assert!(buf
            .push(QueuedEvent::new(agent_event("message")))
            .is_empty());
        assert!(buf
            .push(QueuedEvent::new(task_turn_event("active")))
            .is_empty());
        assert!(buf
            .push(QueuedEvent::new(agent_event("message")))
            .is_empty());
        assert_eq!(buf.len(), 3);

        // Over cap: oldest non-lifecycle (index 0) is evicted, not the taskTurn.
        let dropped = buf.push(QueuedEvent::new(agent_event("message")));
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].req.sub_type, "message");
        assert_eq!(buf.len(), 3);

        // The surviving lifecycle event is still buffered.
        let remaining: Vec<_> = std::iter::from_fn(|| buf.pop_front())
            .map(|e| e.req.sub_type)
            .collect();
        assert!(remaining.contains(&"active".to_string()));
    }

    #[test]
    fn overflow_keeps_lifecycle_even_past_cap() {
        let mut buf = EventBuffer::new(2, usize::MAX);
        // All lifecycle (incl. terminal `stopped`): nothing droppable, cap exceeded.
        assert!(buf
            .push(QueuedEvent::new(task_turn_event("active")))
            .is_empty());
        assert!(buf
            .push(QueuedEvent::new(agent_event("completed")))
            .is_empty());
        assert!(buf.push(QueuedEvent::new(agent_event("error"))).is_empty());
        let dropped = buf.push(QueuedEvent::new(agent_event("stopped")));
        assert!(dropped.is_empty());
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn byte_cap_triggers_eviction() {
        // Tiny byte cap, generous event cap: eviction driven purely by bytes.
        let mut buf = EventBuffer::new(1000, 40);
        // Each agent_event: payload "{}" (2) + sub_type "message" (7) + task "task-1" (6)
        // + a uuid event_id (36) = 51 bytes, already over the 40-byte cap, but a
        // single event is never dropped below itself — it's the only entry.
        assert!(buf
            .push(QueuedEvent::new(agent_event("message")))
            .is_empty());
        assert_eq!(buf.len(), 1);
        // A second push must evict the first to try to get back under budget.
        let dropped = buf.push(QueuedEvent::new(agent_event("message")));
        assert_eq!(dropped.len(), 1);
        assert_eq!(buf.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn drains_events_in_order_with_sequences() {
        let (backend, received) = MockBackend::new(0);
        let cancel = CancellationToken::new();
        let outbox = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

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

        cancel.cancel();
    }

    #[tokio::test(start_paused = true)]
    async fn retries_until_backend_recovers() {
        // Fail the first 3 attempts; the drainer must retry the same event and
        // deliver it exactly once, preserving its identity.
        let (backend, received) = MockBackend::new(3);
        let cancel = CancellationToken::new();
        let outbox = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

        let ev = agent_event("completed");
        let event_id = ev.event_id.clone();
        outbox.enqueue(ev);

        wait_until(|| received.lock().expect("lock").len() == 1).await;

        let got = received.lock().expect("lock");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].event_id, event_id);
        assert_eq!(got[0].sequence, 1);
        assert_eq!(outbox.pending_count(), 0);

        cancel.cancel();
    }

    #[tokio::test(start_paused = true)]
    async fn overflow_emits_dropped_marker() {
        let (backend, received) = MockBackend::new(0);
        let cancel = CancellationToken::new();
        // Cap of 2 events so a burst forces eviction. Keep the backend blocked
        // until after the burst so events pile up in the buffer.
        let outbox = EventOutbox::spawn_with_caps(
            backend,
            "task-1".to_string(),
            cancel.clone(),
            2,
            usize::MAX,
        );

        // Fire a burst of non-lifecycle events; some must be evicted.
        for i in 0..10 {
            outbox.enqueue(agent_event(&format!("message-{i}")));
        }

        wait_until(|| outbox.dropped_count() > 0).await;
        wait_until(|| {
            received
                .lock()
                .expect("lock")
                .iter()
                .any(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
        })
        .await;

        let got = received.lock().expect("lock");
        let marker = got
            .iter()
            .find(|r| r.sub_type == EVENTS_DROPPED_SUB_TYPE)
            .expect("marker present");
        assert!(marker.payload_json.contains("outbox_overflow"));

        cancel.cancel();
    }

    #[tokio::test(start_paused = true)]
    async fn flush_returns_once_drained() {
        let (backend, received) = MockBackend::new(0);
        let cancel = CancellationToken::new();
        let outbox = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());

        for i in 0..3 {
            outbox.enqueue(agent_event(&format!("message-{i}")));
        }
        outbox.flush(Duration::from_secs(5)).await;

        assert_eq!(outbox.pending_count(), 0);
        assert_eq!(received.lock().expect("lock").len(), 3);

        cancel.cancel();
    }
}
