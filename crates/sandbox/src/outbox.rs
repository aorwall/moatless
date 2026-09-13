//! At-least-once send buffer for the sandbox → backend event hop.
//!
//! In-memory, ordered, non-durable across process death: losing it loses
//! events. [`EventOutbox::enqueue`] is non-blocking, never fails the caller, and
//! is where capacity is enforced, so a stalled drainer cannot grow the queue. A
//! single drainer sends in FIFO order with capped exponential backoff, assigning
//! a monotonic `sequence` in transmission order.
//!
//! Every lost event is counted and announced on the stream as an
//! [`EVENTS_DROPPED_SUB_TYPE`] marker. Markers are drainer-generated and never
//! enter the buffer, so a saturated buffer can still announce its saturation.
//!
//! At capacity the outbox evicts the oldest evictable event and counts
//! `dropped_overflow`. When only lifecycle events (`taskTurn.*` and terminal
//! `agent.*`) remain, it refuses the incoming event and counts
//! `rejected_saturated` instead. It always keeps one event, even when that
//! event alone exceeds the byte cap. Lifecycle events are preferred but share
//! the queue budget with every other event, so the outbox refuses the newest
//! and keeps the oldest — the drainer is FIFO, and an older event is the
//! prefix that keeps the later ones interpretable.
//!
//! Drop an event the backend *permanently* refuses; do not retry it. That
//! event is at-most-once, because one in-order drainer would otherwise stall
//! on it and take the task's whole remaining stream with it
//! (`dropped_terminal`). `dropped_unrepresentable` is a bug in this process,
//! not a load condition.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use sandbox_proto::PublishEventRequest;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::{Code, Status};
use tracing::{error, info, warn};

use crate::backend_client::BackendClient;

/// Max number of events buffered before the admission policy applies.
pub const MAX_QUEUE_EVENTS: usize = 10_000;
/// Max total payload bytes buffered before the admission policy applies (~32 MiB).
pub const MAX_QUEUE_BYTES: usize = 32 * 1024 * 1024;

/// Backoff applied before the first retry; doubles each attempt up to the cap.
const BASE_BACKOFF: Duration = Duration::from_millis(500);
/// Ceiling on the exponential backoff between retries.
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Backend admission failures that repeat this many times for the exact same
/// event are treated as poison rather than allowed to wedge the whole stream.
const MAX_DURABLE_ADMISSION_RETRIES: u32 = 12;
const DURABLE_ADMISSION_RETRY_MESSAGE: &str =
    "event not durably admitted; retry with the same event_id";

/// `sub_type` of the synthetic marker emitted after a drop.
pub const EVENTS_DROPPED_SUB_TYPE: &str = "events_dropped";

/// Why an event never reached the backend.
///
/// These are not interchangeable: two are load conditions, one is a
/// wire-contract violation, one is a bug in this process. Each has its own
/// counter in [`OutboxMetrics`] and its own marker reason, so an operator
/// reading either surface can tell which happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropReason {
    /// Capacity-driven: the buffer was full and this was the oldest evictable
    /// event. Load, not error.
    Overflow,
    /// Capacity-driven, but nothing was evictable: the buffer was full of
    /// lifecycle events and this submission was refused. Always a lifecycle
    /// event, and therefore the most consequential of the four.
    RefusedSaturated,
    /// The backend refused the event permanently (`INVALID_ARGUMENT`). Retrying
    /// is what would be wrong — see [`Drainer::deliver`].
    TerminalRejection,
    /// The producer could not put the event on the wire at all (an event type
    /// the sandbox protocol cannot express). A bug in this process.
    UnrepresentableType,
}

impl DropReason {
    /// Fixed order, so marker emission is deterministic when more than one
    /// reason has unannounced drops. Must stay in [`DropReason::index`] order.
    const ALL: [Self; 4] = [
        Self::Overflow,
        Self::RefusedSaturated,
        Self::TerminalRejection,
        Self::UnrepresentableType,
    ];

    /// Position in [`DropReason::ALL`], used to index the per-reason ledgers.
    const fn index(self) -> usize {
        match self {
            Self::Overflow => 0,
            Self::RefusedSaturated => 1,
            Self::TerminalRejection => 2,
            Self::UnrepresentableType => 3,
        }
    }

    /// The `reason` field of the synthetic marker's payload.
    const fn marker_reason(self) -> &'static str {
        match self {
            Self::Overflow => "outbox_overflow",
            Self::RefusedSaturated => "outbox_saturated",
            Self::TerminalRejection => "backend_rejected",
            Self::UnrepresentableType => "unrepresentable_event_type",
        }
    }
}

/// A point-in-time reading of the send buffer's counters.
///
/// The four things that have to stay distinguishable (and the ticket's
/// measurement criterion) map onto these fields as: *dropped* —
/// `dropped_overflow` + `dropped_terminal` + `dropped_unrepresentable`;
/// *backpressured or failed* — `rejected_saturated` + `rejected_shutdown`;
/// *retried* — `retried`; *undrained* — `undrained`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboxMetrics {
    /// Events evicted because the buffer was at capacity.
    pub dropped_overflow: u64,
    /// Events the backend refused permanently.
    pub dropped_terminal: u64,
    /// Events the producer could not represent on the wire.
    pub dropped_unrepresentable: u64,
    /// Admissions refused because capacity was exhausted by lifecycle events.
    pub rejected_saturated: u64,
    /// Admissions refused because the drainer had already terminated.
    pub rejected_shutdown: u64,
    /// Retryable delivery failures — one per re-attempt, not per event.
    pub retried: u64,
    /// Events acknowledged by the backend.
    pub delivered: u64,
    /// Events buffered or in flight, i.e. neither delivered nor dropped.
    pub undrained: usize,
}

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

/// Whether an event is in the priority class that survives eviction.
///
/// Lifecycle events drive downstream reconciliation (the reaper keys off the
/// `taskTurn/active` row, terminal `agent` events close turns) so losing one
/// silently corrupts task state. They are evicted last — but they are still
/// capped; see the module docs for what happens when they alone fill the buffer.
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
    /// Cached [`is_lifecycle`]. The eviction scan tests every buffered event
    /// under the producer's lock, so recomputing it would mean a protobuf enum
    /// conversion plus a string match per element, per eviction.
    lifecycle: bool,
    /// Sequence assigned on the first send attempt, reused across retries so a
    /// retried delivery carries the same `(event_id, sequence)` identity.
    sequence: Option<u64>,
}

impl QueuedEvent {
    fn new(req: PublishEventRequest) -> Self {
        let size = request_size(&req);
        let lifecycle = is_lifecycle(&req);
        Self {
            req,
            size,
            lifecycle,
            sequence: None,
        }
    }
}

/// What the admission policy did with one submitted event.
struct Admission {
    /// Events evicted to make room, in eviction order (oldest non-lifecycle
    /// first). Always [`DropReason::Overflow`].
    evicted: Vec<QueuedEvent>,
    /// Set when the submitted event itself was refused: capacity was exhausted
    /// and nothing evictable was left. Always a lifecycle event.
    rejected: Option<QueuedEvent>,
    /// Whether the submitted event is in the buffer now. It is the newest, so
    /// eviction reaches it only when it is the sole evictable event there.
    admitted: bool,
}

/// Bounded FIFO buffer with the admission policy.
///
/// Kept free of the drainer's async machinery so the policy is unit-testable in
/// isolation; the caller accounts for whatever [`EventBuffer::push`] hands back.
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

    fn len(&self) -> usize {
        self.buf.len()
    }

    fn over_capacity(&self) -> bool {
        self.buf.len() > self.max_events || self.bytes > self.max_bytes
    }

    /// Append `ev` and re-establish the caps.
    ///
    /// Eviction takes the oldest non-lifecycle event first and repeats until the
    /// buffer is back within budget. If nothing evictable is left the newest
    /// event is refused instead, so no priority class grows without bound. A
    /// buffer holding a single event is never emptied: a lone event larger than
    /// the byte cap must still be deliverable rather than dropped into oblivion.
    fn push(&mut self, ev: QueuedEvent) -> Admission {
        self.bytes += ev.size;
        self.buf.push_back(ev);

        let mut evicted = Vec::new();
        let mut admitted = true;
        while self.over_capacity() && self.buf.len() > 1 {
            let Some(idx) = self.buf.iter().position(|e| !e.lifecycle) else {
                // Only lifecycle events left; fall through to refusal.
                break;
            };
            let newest = self.buf.len() - 1;
            // `idx` came from `position()` on this same buffer, so `remove`
            // always yields `Some`; fall through defensively if it somehow can't.
            let Some(removed) = self.buf.remove(idx) else {
                break;
            };
            admitted &= idx != newest;
            self.bytes -= removed.size;
            evicted.push(removed);
        }

        let rejected = if self.over_capacity() && self.buf.len() > 1 {
            match self.buf.pop_back() {
                Some(refused) => {
                    self.bytes -= refused.size;
                    Some(refused)
                }
                None => None,
            }
        } else {
            None
        };

        Admission {
            evicted,
            admitted: admitted && rejected.is_none(),
            rejected,
        }
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

/// One [`DropReason`]'s ledger: what was lost, and how much of that loss the
/// drainer has already announced on the stream.
///
/// `recorded` is written by whoever loses the event (producer or drainer);
/// `announced` is written only by the drainer, so it needs no read-modify-write.
/// The difference is the marker debt [`EventOutbox::flush`] has to wait out —
/// without it, a graceful shutdown could return with a gap still unannounced.
#[derive(Default)]
struct DropLedger {
    recorded: AtomicU64,
    announced: AtomicU64,
}

impl DropLedger {
    fn unannounced(&self) -> u64 {
        self.recorded
            .load(Ordering::SeqCst)
            .saturating_sub(self.announced.load(Ordering::SeqCst))
    }
}

/// Lifetime counters behind [`OutboxMetrics`].
#[derive(Default)]
struct Counters {
    drops: [DropLedger; DropReason::ALL.len()],
    rejected_shutdown: AtomicU64,
    retried: AtomicU64,
    delivered: AtomicU64,
}

impl Counters {
    fn drops(&self, reason: DropReason) -> &DropLedger {
        &self.drops[reason.index()]
    }

    /// Total drops the drainer still owes the stream a marker for.
    fn unannounced_drops(&self) -> u64 {
        self.drops.iter().map(DropLedger::unannounced).sum()
    }
}

/// Everything admission has to decide against, behind one lock.
///
/// `drainer_stopped` lives here rather than in an atomic beside the buffer
/// because the two have to be read and written *together*: a producer that
/// tested a separate flag and then took the buffer lock could be descheduled in
/// between, let the drainer terminate, and push an event nothing will ever send.
struct AdmissionState {
    buffer: EventBuffer,
    /// The one event the drainer has popped but not yet resolved. Counted as
    /// undrained; kept outside the buffer so delivery holds no lock.
    in_flight: usize,
    /// Set once the drainer has terminated. Admission closes with it, in the
    /// same critical section, so there is no window in which an event can be
    /// buffered with no consumer left to drain it.
    drainer_stopped: bool,
}

impl AdmissionState {
    fn undrained(&self) -> usize {
        self.buffer.len() + self.in_flight
    }
}

/// State shared by every producer handle and the single drainer.
struct Shared {
    state: Mutex<AdmissionState>,
    counters: Counters,
    /// Producer → drainer: there is work (or an unannounced drop) to pick up.
    work: Notify,
    /// `flush` → drainer: stop waiting out a retry backoff and try now.
    flush_nudge: Notify,
}

impl Shared {
    /// The admission guard. Poison recovery is deliberate: nothing inside the
    /// critical section can panic short of allocation failure, and forfeiting
    /// every queued event over a poisoned lock would be strictly worse than
    /// continuing.
    fn state(&self) -> MutexGuard<'_, AdmissionState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn undrained(&self) -> usize {
        self.state().undrained()
    }

    /// Pop the next event for delivery, marking it in flight under the same
    /// lock so [`AdmissionState::undrained`] never transiently under-reports.
    fn take_next(&self) -> Option<QueuedEvent> {
        let mut state = self.state();
        let ev = state.buffer.pop_front()?;
        state.in_flight = 1;
        Some(ev)
    }

    /// Put an undelivered in-flight event back at the head (shutdown path).
    fn return_in_flight(&self, ev: QueuedEvent) {
        let mut state = self.state();
        state.buffer.push_front(ev);
        state.in_flight = 0;
    }

    fn clear_in_flight(&self) {
        self.state().in_flight = 0;
    }

    /// Close admission. Taken under the admission lock, so once this returns no
    /// producer can add an event the drainer will never see: a submission is
    /// either already buffered (and reported as undrained) or refused.
    fn stop_admission(&self) {
        self.state().drainer_stopped = true;
    }

    fn record_drop(&self, reason: DropReason) {
        self.counters
            .drops(reason)
            .recorded
            .fetch_add(1, Ordering::SeqCst);
    }

    fn metrics(&self) -> OutboxMetrics {
        let c = &self.counters;
        let recorded = |reason: DropReason| c.drops(reason).recorded.load(Ordering::SeqCst);
        OutboxMetrics {
            dropped_overflow: recorded(DropReason::Overflow),
            dropped_terminal: recorded(DropReason::TerminalRejection),
            dropped_unrepresentable: recorded(DropReason::UnrepresentableType),
            rejected_saturated: recorded(DropReason::RefusedSaturated),
            rejected_shutdown: c.rejected_shutdown.load(Ordering::SeqCst),
            retried: c.retried.load(Ordering::SeqCst),
            delivered: c.delivered.load(Ordering::SeqCst),
            undrained: self.undrained(),
        }
    }
}

/// Cloneable producer handle to the send buffer.
///
/// Cheap to clone (all shared state is `Arc`-wrapped). Held by
/// [`crate::events::GrpcEventPublisher`]; `enqueue` is the only hot-path method.
#[derive(Clone)]
pub struct EventOutbox {
    shared: Arc<Shared>,
}

impl EventOutbox {
    /// Build the send buffer and spawn its single drainer task.
    ///
    /// Returns the producer handle and the owned [`OutboxDrainerHandle`]; the
    /// drainer runs until `cancel` fires. `task_id` labels the synthetic drop
    /// marker.
    pub fn spawn(
        client: Arc<dyn BackendClient>,
        task_id: String,
        cancel: CancellationToken,
    ) -> (Self, OutboxDrainerHandle) {
        Self::spawn_with_caps(client, task_id, cancel, MAX_QUEUE_EVENTS, MAX_QUEUE_BYTES)
    }

    fn spawn_with_caps(
        client: Arc<dyn BackendClient>,
        task_id: String,
        cancel: CancellationToken,
        max_events: usize,
        max_bytes: usize,
    ) -> (Self, OutboxDrainerHandle) {
        let shared = Arc::new(Shared {
            state: Mutex::new(AdmissionState {
                buffer: EventBuffer::new(max_events, max_bytes),
                in_flight: 0,
                drainer_stopped: false,
            }),
            counters: Counters::default(),
            work: Notify::new(),
            flush_nudge: Notify::new(),
        });

        let drainer = Drainer {
            client,
            shared: Arc::clone(&shared),
            seq: 0,
            task_id,
            cancel,
        };
        let join = tokio::spawn(drainer.run());

        (Self { shared }, OutboxDrainerHandle { join })
    }

    /// Submit an event for at-least-once delivery. Non-blocking; never fails the
    /// caller.
    ///
    /// Capacity is enforced here, synchronously, so a stalled drainer cannot let
    /// the queue grow past its caps. Whether the drainer is still running is
    /// decided in the *same* critical section, so a submission is never buffered
    /// into a queue with no consumer. Whatever the policy loses is counted under
    /// [`OutboxMetrics::dropped_overflow`], [`OutboxMetrics::rejected_saturated`]
    /// or [`OutboxMetrics::rejected_shutdown`] and, for a drop, announced in
    /// stream order.
    ///
    /// Returns whether *this* event entered the buffer. A caller that records
    /// having published something needs that answer: the admission policy
    /// refuses a submission outright when the buffer is saturated or the drainer
    /// has stopped, and a caller told nothing would go on believing an event it
    /// never sent is on its way.
    pub fn enqueue(&self, req: PublishEventRequest) -> bool {
        // Scoped so the guard is released before any logging. `Err` hands the
        // request back out so the refusal can be logged outside the lock.
        let outcome = {
            let mut state = self.shared.state();
            if state.drainer_stopped {
                Err(req)
            } else {
                Ok(state.buffer.push(QueuedEvent::new(req)))
            }
        };

        let admission = match outcome {
            Ok(admission) => admission,
            Err(refused) => {
                self.shared
                    .counters
                    .rejected_shutdown
                    .fetch_add(1, Ordering::SeqCst);
                warn!(
                    sub_type = %refused.sub_type,
                    "event send buffer drainer stopped; event refused"
                );
                return false;
            }
        };
        let admitted = admission.admitted;

        for removed in &admission.evicted {
            self.shared.record_drop(DropReason::Overflow);
            warn!(
                sub_type = %removed.req.sub_type,
                "event send buffer at capacity: evicted oldest non-lifecycle event"
            );
        }
        if let Some(refused) = &admission.rejected {
            // Refusal is the most consequential loss of the four — by the
            // eviction policy's own invariant this is always a lifecycle event —
            // so it goes through the same seam as an eviction and gets its own
            // marker rather than being visible only in a local counter.
            self.shared.record_drop(DropReason::RefusedSaturated);
            error!(
                sub_type = %refused.req.sub_type,
                "event send buffer saturated by lifecycle events: refused the newest"
            );
        }

        self.shared.work.notify_one();
        admitted
    }

    /// Account for an event the producer could not put on the wire at all.
    ///
    /// The event never entered the buffer, so this is the producer's only way to
    /// reach the drop-accounting seam. It is a bug in this process, not a load
    /// condition — it is counted under
    /// [`OutboxMetrics::dropped_unrepresentable`] and announced like any other
    /// gap.
    pub fn record_unrepresentable(&self) {
        self.shared.record_drop(DropReason::UnrepresentableType);
        self.shared.work.notify_one();
    }

    /// Events buffered or in flight — neither delivered nor dropped.
    pub fn pending_count(&self) -> usize {
        self.shared.undrained()
    }

    /// A point-in-time reading of every counter.
    pub fn metrics(&self) -> OutboxMetrics {
        self.shared.metrics()
    }

    /// Wait until the buffer is fully drained or `timeout` elapses, whichever
    /// comes first. Called on graceful shutdown before the transcript flush.
    ///
    /// Bounded and best-effort by construction: on timeout it reports what is
    /// still undrained and returns, leaving the drainer to keep retrying in the
    /// background until the process exits. It also returns early once the
    /// drainer has stopped, because nothing left will move then.
    pub async fn flush(&self, timeout: Duration) {
        let outcome = tokio::time::timeout(timeout, async {
            loop {
                let (undrained, stopped) = {
                    let state = self.shared.state();
                    (state.undrained(), state.drainer_stopped)
                };
                // A dead drainer will never move anything, so waiting is pure
                // delay. This is separate from the work check below, not part
                // of it: it is "nothing can happen", not "nothing is pending".
                if stopped {
                    return;
                }
                // Drops are carried by counter deltas, not by buffered events,
                // so an empty buffer is not an empty queue: a terminal rejection
                // or an unrepresentable type leaves a marker the drainer still
                // owes the stream. Returning here would cancel the drainer with
                // that gap unannounced.
                if undrained == 0 && self.shared.counters.unannounced_drops() == 0 {
                    return;
                }
                // Nudge the drainer out of any backoff sleep, then re-poll. A
                // short poll avoids the lost-wakeup race a bare Notify would have.
                self.shared.flush_nudge.notify_waiters();
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;

        let undrained = self.shared.undrained();
        let unannounced_drops = self.shared.counters.unannounced_drops();
        if undrained > 0 || unannounced_drops > 0 {
            warn!(
                undrained,
                unannounced_drops,
                timed_out = outcome.is_err(),
                "event send buffer flush finished with work still queued"
            );
        }
    }
}

/// Owned handle to the spawned drainer task.
///
/// # Ownership and criticality
///
/// **Criticality: critical.** The drainer is the only thing that turns a
/// buffered event into a delivered one. A process whose drainer has terminated
/// keeps producing events that can never be sent — which is why
/// [`EventOutbox::enqueue`] refuses admission once
/// [`OutboxDrainerHandle::is_finished`] would report `true`, counting the
/// refusals under [`OutboxMetrics::rejected_shutdown`] instead of silently
/// buffering them.
///
/// The handle is returned to the composition root rather than discarded, and is
/// `#[must_use]` so dropping it is a deliberate act. The drainer participates in
/// cancellation through the token passed to [`EventOutbox::spawn`], and
/// [`Self::shutdown`] joins it within a bounded deadline.
/// [`Self::into_join_handle`] hands the raw handle to a supervisor that wants to
/// own termination policy itself.
#[must_use = "the drainer handle owns critical work; dropping it detaches the task"]
pub struct OutboxDrainerHandle {
    join: JoinHandle<()>,
}

impl OutboxDrainerHandle {
    /// Whether the task has terminated — the signal for a supervisor that polls
    /// rather than joins.
    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    /// Surrender the raw join handle to a supervisor.
    pub fn into_join_handle(self) -> JoinHandle<()> {
        self.join
    }

    /// Wait up to `deadline` for the (already-cancelled) drainer to finish,
    /// reporting a straggler rather than blocking shutdown indefinitely.
    pub async fn shutdown(self, deadline: Duration) {
        match tokio::time::timeout(deadline, self.join).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(error = %e, "event send buffer drainer terminated abnormally"),
            Err(_elapsed) => warn!(
                deadline_ms = deadline.as_millis() as u64,
                "event send buffer drainer did not stop within the shutdown deadline"
            ),
        }
    }
}

/// Outcome of a (retrying) delivery attempt.
enum Delivery {
    /// The event was acknowledged by the backend.
    Sent,
    /// The backend refused the event permanently; resending cannot succeed.
    Rejected(Status),
    /// The drainer was cancelled mid-delivery (shutdown).
    Cancelled,
}

/// The single background task that drains the buffer.
struct Drainer {
    client: Arc<dyn BackendClient>,
    shared: Arc<Shared>,
    /// Last sequence assigned; incremented in transmission order.
    seq: u64,
    task_id: String,
    cancel: CancellationToken,
}

impl Drainer {
    async fn run(mut self) {
        loop {
            if self.cancel.is_cancelled() {
                break;
            }

            // Announce any drops before the next real event so each gap is
            // visible at the point in the stream where it happened.
            if !self.announce_drops().await {
                break;
            }

            let Some(mut ev) = self.shared.take_next() else {
                tokio::select! {
                    () = self.cancel.cancelled() => break,
                    () = self.shared.work.notified() => {}
                }
                continue;
            };

            if ev.sequence.is_none() {
                self.seq += 1;
                ev.sequence = Some(self.seq);
            }
            ev.req.sequence = ev.sequence.unwrap_or(0);

            match self.deliver(&ev.req).await {
                Delivery::Sent => {
                    self.shared.clear_in_flight();
                    self.shared
                        .counters
                        .delivered
                        .fetch_add(1, Ordering::SeqCst);
                }
                Delivery::Rejected(status) => {
                    self.shared.clear_in_flight();
                    self.shared.record_drop(DropReason::TerminalRejection);
                    error!(
                        task_id = %ev.req.task_id,
                        event_id = %ev.req.event_id,
                        sub_type = %ev.req.sub_type,
                        error = %status,
                        "backend permanently rejected an event; dropping it and moving on"
                    );
                }
                Delivery::Cancelled => {
                    // Preserve ordering for any in-flight flush already waiting.
                    self.shared.return_in_flight(ev);
                    break;
                }
            }
        }

        // Close admission before reporting, so the counts below cannot be
        // invalidated by a producer that slipped in after the loop exited.
        self.shared.stop_admission();

        // The drainer's last act is the only unconditional report of what this
        // buffer lost over the pod's lifetime, so it carries every counter
        // rather than just the depth.
        let m = self.shared.metrics();
        info!(
            task_id = %self.task_id,
            delivered = m.delivered,
            undrained = m.undrained,
            retried = m.retried,
            dropped_overflow = m.dropped_overflow,
            dropped_terminal = m.dropped_terminal,
            dropped_unrepresentable = m.dropped_unrepresentable,
            rejected_saturated = m.rejected_saturated,
            rejected_shutdown = m.rejected_shutdown,
            "event send buffer drainer stopped"
        );
    }

    /// Emit one marker per [`DropReason`] with unannounced drops, in
    /// [`DropReason::ALL`] order. Returns `false` if the drainer was cancelled.
    async fn announce_drops(&mut self) -> bool {
        // A local handle so the ledger borrow below does not tie up `self`,
        // which `deliver` needs mutably.
        let shared = Arc::clone(&self.shared);

        for reason in DropReason::ALL {
            let ledger = shared.counters.drops(reason);
            // Read `recorded` once: anything recorded after this point stays
            // unannounced and is picked up by the next pass, which is what keeps
            // `flush`'s marker-debt check and this loop reading the same ledger.
            let recorded = ledger.recorded.load(Ordering::SeqCst);
            let announced = ledger.announced.load(Ordering::SeqCst);
            let Some(count) = recorded.checked_sub(announced).filter(|c| *c > 0) else {
                continue;
            };

            let mut marker = self.build_marker(reason, count);
            // The marker rides the same ordered stream as real events, so it
            // takes the next sequence rather than a placeholder the backend
            // would read as a restart.
            self.seq += 1;
            marker.sequence = self.seq;

            match self.deliver(&marker).await {
                Delivery::Sent => ledger.announced.store(recorded, Ordering::SeqCst),
                Delivery::Rejected(status) => {
                    // The marker is backend-authored in all but name — a UUID, a
                    // known event type, serialised JSON — so a terminal rejection
                    // means version skew. Record it as announced anyway: counting
                    // the marker's own loss as a drop would generate a new marker
                    // that would be rejected in turn, forever.
                    error!(
                        task_id = %self.task_id,
                        reason = reason.marker_reason(),
                        error = %status,
                        "backend permanently rejected a dropped-events marker; gap left unannounced"
                    );
                    ledger.announced.store(recorded, Ordering::SeqCst);
                }
                Delivery::Cancelled => return false,
            }
        }
        true
    }

    /// Send one request, retrying with exponential backoff until it is
    /// acknowledged, permanently refused, or the drainer is cancelled.
    ///
    /// Two codes are terminal, both for the same reason: the request cannot
    /// succeed no matter how often it is resent, so retrying would wedge this
    /// drainer's head forever and take the task's entire event stream with it.
    /// Both are dropped and counted under [`DropReason::TerminalRejection`],
    /// which announces the gap with a marker rather than losing it silently.
    ///
    /// - `INVALID_ARGUMENT` — the request violates the wire contract (unknown
    ///   `event_type`, empty `event_id`, unparseable `payload_json`; see the
    ///   backend's `AdmissionError`).
    /// - `OUT_OF_RANGE` — the encoded message exceeds a gRPC size limit. tonic
    ///   raises this from the codec on both sides, and neither end of this path
    ///   configures a limit, so the backend's decode ceiling is tonic's 4 MiB
    ///   default. An `agent.message` carrying a large tool result can cross it,
    ///   and the payload is fixed by the time it reaches here — re-encoding the
    ///   same bytes produces the identical rejection every time.
    /// - repeated durable-admission `UNAVAILABLE` — the backend deliberately
    ///   redacts commit failures behind one retryable status. A short outage
    ///   should converge, but the same event failing through the whole retry
    ///   budget is more likely poison content than recoverable downtime. Drop it
    ///   with a marker so later messages can still reach the task.
    ///
    /// `RESOURCE_EXHAUSTED` is deliberately *not* terminal: it is the standard
    /// code for quota and rate limiting, which is exactly what backoff is for.
    async fn deliver(&mut self, req: &PublishEventRequest) -> Delivery {
        let mut attempt: u32 = 0;
        loop {
            if self.cancel.is_cancelled() {
                return Delivery::Cancelled;
            }

            match self.client.publish_event(req.clone()).await {
                Ok(()) => return Delivery::Sent,
                Err(status)
                    if matches!(status.code(), Code::InvalidArgument | Code::OutOfRange) =>
                {
                    return Delivery::Rejected(status);
                }
                Err(status) => {
                    attempt += 1;
                    if is_repeated_durable_admission_failure(&status, attempt) {
                        error!(
                            task_id = %req.task_id,
                            event_id = %req.event_id,
                            sub_type = %req.sub_type,
                            attempt,
                            error = %status,
                            "backend durable admission kept failing; dropping event and moving on"
                        );
                        return Delivery::Rejected(status);
                    }
                    self.shared.counters.retried.fetch_add(1, Ordering::SeqCst);
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
                        () = self.shared.flush_nudge.notified() => {}
                        () = tokio::time::sleep(backoff) => {}
                    }
                }
            }
        }
    }

    /// Build the synthetic drop marker. It rides the same ordered stream as real
    /// events (its own `event_id` + sequence) so the backend records the gap
    /// without special-casing.
    fn build_marker(&self, reason: DropReason, count: u64) -> PublishEventRequest {
        let payload = serde_json::json!({
            "reason": reason.marker_reason(),
            "dropped_count": count,
        });
        PublishEventRequest {
            task_id: self.task_id.clone(),
            event_type: sandbox_proto::EventType::Sandbox as i32,
            sub_type: EVENTS_DROPPED_SUB_TYPE.to_string(),
            payload_json: payload.to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
            run_id: String::new(),
        }
    }
}

fn is_repeated_durable_admission_failure(status: &Status, attempt: u32) -> bool {
    status.code() == Code::Unavailable
        && status.message() == DURABLE_ADMISSION_RETRY_MESSAGE
        && attempt >= MAX_DURABLE_ADMISSION_RETRIES
}

#[cfg(test)]
mod tests;
