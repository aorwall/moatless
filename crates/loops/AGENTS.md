# crates/loops

The loops band: Adapter vocabulary, durable inbound delivery, bindings, and
loop sync. It sits below the backend and above the provider crates
(`adapter-linear`, `adapter-github-pr`, …), which is why both ends can name it.

## Ownership

- **Binding** — the `task_bindings` table, its `TaskBindingDao` store, and
  `PgTaskBindingDao`, all in `bindings/dao.rs`. This crate owns them; no other
  crate declares a Binding store.
- Two seams a crate that does **not** drive the Binding lifecycle reads:
  `BindingProbe` (is this subject bound?), published by the Adapter kernel at
  `moatless_adapter_model::binding`, and, through
  `moatless_backend_core::binding::TaskBindings`, the Task band's downward edge.
  `BindingDirectory` in `bindings/ports.rs` implements both.
- **Loop** — the four Loop tables and their `LoopDao` store. The Adapter kernel
  publishes `LoopReader` (`moatless_adapter_model::loops`), the Subscription reads
  an Adapter matches an inbound event against; `loops/ports.rs` implements it for
  `LoopDao` directly. `LoopDao` is the only store over those tables, so a wrapper
  beside it would be a second surface.
- An Adapter names `LoopReader`, never `LoopDao`. The store has twenty-six
  inherent functions and the Adapters read five of them. One exception stands:
  `adapter-telegram/src/pairing.rs` names `LoopDao` because it reads and writes
  a Loop's adapter config as part of approving a pairing code. Whether that
  module survives is a behaviour question — see `docs/deferred-changes.md`.
- An Adapter that upserts Bindings as well as reading and writing them
  (`adapter-github-pr`, `adapter-linear`) names `TaskBindingDao` directly. Do not
  add a narrow read port for a crate that already holds the store — that leaves
  two surfaces over one table.

## The Adapter kernel is a crate, not a re-export

The whole Adapter contract lives in `moatless-adapter-model` — its holdings,
the `postgres`/`http`/`test-support` feature gates, and its driver-free base
surface are documented in `crates/adapter-model/AGENTS.md`. A provider
implements an Adapter by naming that crate and nothing above it.

Name it directly — `moatless_adapter_model::…` — from this crate and from every
provider crate. Do not re-export it through `crate::adapters`; one path to a
type is the point. The shared Adapter vocabulary that lives one crate lower —
`AdapterKind`, `BindingDestination`, `DestinationPostResult` in
`moatless-types` — follows the same rule: name it from `moatless_types::adapter`,
not through this facade.

What is left under `src/adapters/` is what the loops band adds *around* the
contract: the durable-delivery store and worker, the inbound-message handler,
self-echo, subject events, and the shared connection/loop handler support. If a
change would put a trait every provider implements, the registry, an ingress
vocabulary, or a read seam a provider names back in here, it belongs in the
kernel instead.

`scripts/check-crate-seams.sh` enforces this: invariant 3 fails a `pub use` of
another crate's item anywhere under `src/adapters/` unless it is listed in the
script's `ALLOWED_REEXPORTS` with the change that removes it — a list that is
empty and only shrinks — and invariant 4 fails a `moatless_loops` reference in
`adapter-slack` or `adapter-webhook`, the two provider crates the kernel freed.

## Two seams join a provider to the rest of the system

Neither is a match arm, and that is the point: adding a provider adds a module
and one registration line, and never edits an arm another provider is also
editing.

- `adapters::delivery` — **inbound, durable**. A webhook handler admits work
  under a `kind`; the worker executes the handler registered for the same
  `kind`. At-least-once, so every handler must be idempotent.
- `adapters::events` — **outbound, best effort**. A provider subscribes a
  `TaskEventSubscriber` to the `EventKey`s it reacts to; the backend's
  event-handler consumer fans each task event out to them.

Put durable provider work behind `delivery`. Put "mirror what is happening back
to the provider" behind `events`.

A delivery handler still emits its own `record_webhook`
(`moatless_adapter_model::metrics`). The delivery metric is deliberately
low-cardinality — `{adapter, kind, outcome}` — so it cannot show a single broken
connection; `record_webhook` carries `event_type`, `loop_id` and
`connection_id`, and is what makes one visible.

`AdmitError::Store` is wrapped at its raise site in `delivery/store.rs`, so the
`sqlx::Error` survives as a `StoreError` source without entering the type
providers match on. The driver-free error rule itself is stated in
`crates/adapter-model/AGENTS.md`.

## Rules for a `TaskEventSubscriber`

- **Keep `on_task_event` cheap.** It runs inline on the consumer's dispatch, one
  event at a time. Decide there, and `tokio::spawn` the provider call.
- **That spawn stays detached.** A progress notification lost to a restart is
  lost on purpose — the run is unaffected and the next event carries the task's
  state again — so there is nothing for a supervisor to own. Work that must not
  be lost is a delivery, not a subscription.
- **Own the whole reaction, including its ordering.** If a provider keeps
  per-task state (Linear's turn gate), the subscriber owns it and both the
  emission and the state change happen there, in the order the provider needs.
  Splitting the two across bands is how the gate came to close before the
  activity it was gating was emitted.
- **Do not rely on running before another subscriber.** Registration order is
  not a contract. A subscriber does run after the generic handler for its event,
  which is what makes a reaction keyed on persisted state safe.
