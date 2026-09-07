# crates/loops

[BANDS](../../tooling/arch-rules/src/band_order.rs) owns the dependency order.

## What this crate owns

**Binding.** This crate owns the `task_bindings` table, its `TaskBindingDao`
store, and `PgTaskBindingDao`, all in `bindings/dao.rs`. No other crate
declares a Binding store.

A crate that does not drive the Binding lifecycle reads one of two seams.
`BindingProbe` answers "is this subject bound?" and the Adapter kernel
publishes it at `moatless_adapter_model::binding`. The Task band's downward
edge arrives through `moatless_tasks::task::bindings::TaskBindings`.
`BindingDirectory` in `bindings/ports.rs` implements both.

**Loop.** This crate owns the four Loop tables and their `LoopDao` store, the
only store over them. The Adapter kernel publishes `LoopReader`
(`moatless_adapter_model::loops`), the Subscription read an Adapter matches an
inbound event against, and `loops/ports.rs` implements it for `LoopDao`
directly. SQL row decoding belongs to `loops/persistence.rs` and
`bindings/persistence.rs` beside those stores; the Adapter kernel owns only
connection persistence.

An Adapter names `LoopReader` and never `LoopDao`. One exception stands:
`adapter-telegram/src/pairing.rs` names `LoopDao`, because it reads and writes
a Loop's adapter config while it approves a pairing code. See
`docs/deferred-changes.md`.

An Adapter that upserts Bindings as well as reading and writing them
(`adapter-github-pr`, `adapter-linear`) names `TaskBindingDao` directly. Do not
add a narrow read port for a crate that already holds the store: that leaves
two surfaces over one table.

## Naming the Adapter kernel

The whole Adapter contract lives in `moatless-adapter-model`.
[`crates/adapter-model/AGENTS.md`](../adapter-model/AGENTS.md) documents its
holdings, its `postgres`, `http` and `test-support` feature gates, and its
driver-free base surface. A provider implements an Adapter by naming that crate
and nothing above it.

Name the kernel directly as `moatless_adapter_model::…`, from this crate and
from every provider crate. Do not re-export it through `crate::adapters`: a
type gets one path. The shared Adapter vocabulary one crate lower follows the
same rule. Name `AdapterKind` from `moatless_types::adapter`. Provider binding
contexts and destinations belong to `moatless_adapter_model::binding`; outbound
messages and post results belong to `moatless_adapter_model::outbound`.

`src/adapters/` holds what the loops band adds *around* the contract:

- the durable-delivery store and worker;
- the inbound-message handler;
- self-echo;
- subject events;
- Adapter connection response projection.

A trait every provider implements, the registry, an ingress vocabulary, or a
read seam a provider names all belong in the kernel instead.

[Adapter boundaries](../../tooling/arch-rules/src/adapter_boundaries.rs)
enforce these imports.

## Choosing between the two provider seams

Two seams join a provider to the rest of the system. Neither is a match arm.
Adding a provider adds a module and one registration line, and edits no arm
that another provider is also editing.

| Seam | Direction | Contract |
|---|---|---|
| `adapters::delivery` | Inbound, durable | A webhook handler admits work under a `kind`, and the worker executes the handler registered for the same `kind`. Delivery is at-least-once, so every handler must be idempotent |
| `adapters::events` | Outbound, best effort | A provider subscribes a `TaskEventSubscriber` to the `EventKey`s it reacts to, and the backend's event-handler consumer fans each task event out to them |

Put durable provider work behind `delivery`. Put work that mirrors a task's
state back to the provider behind `events`.

A delivery handler still emits its own `record_webhook`
(`moatless_adapter_model::metrics`). The delivery metric is low-cardinality at
`{adapter, kind, outcome}` and cannot show one broken connection;
`record_webhook` carries `event_type`, `loop_id` and `connection_id`.

Wrap `AdmitError::Store` at its raise site in `adapters/delivery/store.rs`, so
the `sqlx::Error` survives as a `StoreError` source and never enters the type
providers match on. `crates/adapter-model/AGENTS.md` states the driver-free
error rule.

## Rules for a `TaskEventSubscriber`

- **Keep `on_task_event` cheap.** It runs inline on the consumer's dispatch, one
  event at a time. Decide there, then `tokio::spawn` the provider call.
- **Leave that spawn detached.** A progress notification lost to a restart is
  lost on purpose; the next event carries the task's state again. Work that must
  not be lost is a delivery and not a subscription.
- **Own the whole reaction, including its ordering.** When a provider keeps
  per-task state, such as Linear's turn gate, the subscriber owns that state.
  Emit the event and change the state there, in the order the provider needs.
  Split the two across bands and the gate closes before the activity it gates is
  emitted.
- **Do not rely on running before another subscriber.** Registration order is no
  contract. A subscriber does run after the generic handler for its event, which
  is what makes a reaction keyed on persisted state safe.
