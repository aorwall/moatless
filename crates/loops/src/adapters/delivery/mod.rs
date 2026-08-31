//! Durable admission for every inbound Adapter delivery.
//!
//! What an inbound delivery sets off cannot honestly finish on the HTTP
//! response path: routing an inbound message can create a task and provision a
//! sandbox, and a sender that waits three seconds for a 200 will not wait for
//! that. Spawning it and returning 200 anyway makes the acknowledgment a lie —
//! the sender believes the delivery is accepted, and a deploy, an OOM kill, or
//! a panic loses it with no trace and no redelivery.
//!
//! This module is the one seam that makes such an acknowledgment true. An
//! Adapter delivery is written to `adapter_delivery` inside the request
//! handler, keyed by the sender's own delivery identifier; the supervised
//! `adapter-delivery-worker` claims it, executes it, and drives it to a
//! terminal state. Answering a sender with success means the delivery is
//! durably owned, not that a task was spawned.
//!
//! **One sender delivery is one *or more* records.** The identity is
//! `(adapter, kind, event_id)`, so a delivery that fans into several kinds —
//! GitHub activity on an open pull request is the ordinary case — becomes one
//! record per kind, each retrying and failing on its own budget. The sender's
//! own interface counts that same event as one; this is the stated cost of
//! naming the record for what it is for.
//!
//! # Admission and overload semantics
//!
//! **When the sender is acknowledged.** After [`AdapterDeliveryStore::admit`]
//! returns `Admission::Admitted` or `Admission::Duplicate` — that is, after
//! the record is committed. An [`AdmitError`] must be surfaced to the sender
//! as a retryable failure so it redelivers; it must never be swallowed into a
//! 200. The vocabulary those outcomes are spelled in belongs to the Adapter
//! kernel (`moatless_adapter_model::delivery`); this module is the machinery
//! that keeps its promise.
//!
//! **What is durable.** The whole `payload`, which must be everything the
//! handler needs to redo the delivery's work from scratch. Nothing in process
//! memory survives the crash this table defends against, so a payload that
//! references in-memory state is not durable.
//!
//! **Retries and deduplication.** Deduplication is a unique index on
//! `(adapter, kind, event_id)` where `event_id` is the sender's delivery
//! identifier: a redelivery is a duplicate, not a second record. Execution is
//! at-least-once, so handlers must be idempotent. A failed attempt is retried
//! with exponential backoff up to [`AdapterDeliveryPolicy::max_attempts`]; the
//! last attempt, or any failure the handler classifies as
//! `AdapterDeliveryFailure::Permanent`, makes the record terminal (`failed`)
//! with its cause retained in `last_error`, an `error` log, and a `terminal`
//! metric increment.
//!
//! **What happens at capacity.** Admission is bounded by
//! [`AdapterDeliveryPolicy::backlog_capacity`] unfinished records and refuses
//! with `AdmitError::AtCapacity` past it. Refusing is the point: the sender
//! retries a delivery it still owns, instead of the backend accepting a
//! delivery it has no capacity to execute. The bound is a stable ceiling rather
//! than an exact cutoff — concurrent admissions can overshoot it by at most the
//! number in flight, and never ratchet past that.
//! [`AdapterDeliveryPolicy::backlog_capacity`] states the arithmetic and why an
//! exact cutoff was not bought.
//!
//! **How shutdown drains.** On root cancellation the worker stops claiming new
//! batches and waits for the batch in flight — but only for as long as the
//! process itself waits. The root supervisor bounded-joins its tasks for
//! `SHUTDOWN_JOIN_DEADLINE` (20s) and then returns, naming whatever is still
//! running, while [`AdapterDeliveryPolicy::execution_timeout`] allows a single
//! execution 2 minutes. A deploy that catches a long-running batch therefore
//! exits with handlers mid-flight, and logs `adapter-delivery-worker` as a
//! straggler when it does; that warning is the expected shape of this
//! interaction, not a fault.
//!
//! Correctness does not rest on the drain. Records still `claimed` when the
//! process dies are recovered by the next run of the same owner, or by claim
//! lease expiry if that owner never returns. Shutdown delays an admitted
//! delivery; it never loses it — and that, rather than a clean drain, is the
//! guarantee.
//!
//! # What admitting a delivery costs its Adapter
//!
//! Every Adapter is on this seam: Slack, Telegram, and the generic Webhook
//! Adapter route inbound messages through
//! [`crate::adapters::inbound_delivery`], and GitHub and Linear register their
//! own kinds. Nothing inbound runs on a response path any more. The two
//! properties below are what admitting gives up in exchange for durability, and
//! every Adapter has to answer them rather than discover them:
//!
//! **Per-connection webhook outcome telemetry.** The metric here is only
//! `{adapter, kind, outcome}` and is deliberately low-cardinality, whereas
//! [`record_webhook`](moatless_adapter_model::metrics) carries `event_type`,
//! `loop_id`, and `connection_id` — which is what makes a single broken
//! Adapter connection visible. The seam's metric is not a replacement: a
//! handler must keep emitting its own webhook outcome metric, or per-connection
//! failure visibility is lost. The chat handler does, from inside `execute`,
//! with the labels carried in its payload for that sole purpose.
//!
//! **Execution start latency.** A spawn starts in microseconds. An admitted
//! delivery waits for the next poll, so the path trades a bounded scheduling
//! delay for durability. [`AdapterDeliveryPolicy::poll_interval`] is set for
//! interactive work rather than for batch throughput for exactly this reason.
//!
//! **In-process deduplication no Adapter needs any more.** Every Adapter once
//! kept a five-minute, per-process cache of handled event ids in front of
//! routing; all of them are gone. Keyed on the same identifier this table is
//! keyed on, such a cache sits *in front of* a fallible admission, so a
//! redelivery whose admission failed would be answered with a 200 and nothing
//! durable behind it — the precise acknowledgment lie this module exists to
//! remove. What the caches also covered, and the unique index does not, is
//! repeat *parse-time* side effects on a redelivery; that consequence is stated
//! where each Adapter parses.
//!
//! # What does not belong here
//!
//! Best-effort outbound activity notifications and API-key usage timestamps
//! stay off this path. They are allowed to be lost, and admitting them would
//! make every webhook slower and this table larger for no correctness gain.

mod metrics;
mod models;
mod store;
mod typed;
mod worker;

pub use metrics::describe_adapter_delivery_metrics;
pub use models::AdapterDeliveryPolicy;
pub use store::AdapterDeliveryStore;
pub use typed::{ConnectionDeliveryPayload, register_connection_delivery, register_typed_delivery};
pub use worker::spawn_adapter_delivery_worker;
