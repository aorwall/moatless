# crates/loops

The loops band: Adapter vocabulary, durable inbound delivery, bindings, and
loop sync. It sits below the backend and above the provider crates
(`adapter-linear`, `adapter-github-pr`, …), which is why both ends can name it.

## Ownership

- **Binding** — the `task_bindings` table, its `TaskBindingDao` store, and
  `PgTaskBindingDao`, all in `bindings/dao.rs`. This crate owns them; no other
  crate declares a Binding store.
- Two seams sit in `bindings/ports.rs` for a crate that does **not** drive the
  Binding lifecycle: `BindingProbe` (is this subject bound?) and, through
  `moatless_backend_core::binding::TaskBindings`, the Task band's downward edge.
  `BindingDirectory` implements both.
- **Loop** — the four Loop tables and their `LoopDao` store. `loops/ports.rs`
  publishes `LoopReader`, the Subscription reads an Adapter matches an inbound
  event against. `LoopDao` implements it directly: it is the only store over
  those tables, so a wrapper beside it would be a second surface.
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

`moatless-adapter-model` holds the whole Adapter contract: what an Adapter
offers (`capabilities` — the eleven traits a provider implements), what it
speaks (`models`, `apps`, `footer`, `signature`), what it can fail with
(`error` — both halves, `SharedAdapterError` and `AdapterError`, plus the
`IntoResponse` renderer over them), where its connections are stored (`dao`),
and the ingress counters (`metrics`). A provider crate implements an Adapter by
naming that crate and nothing above it.

Name it directly — `moatless_adapter_model::…` — from this crate and from every
provider crate. Do not re-export it through `crate::adapters`; one path to a
type is the point. The shared Adapter vocabulary that lives one crate lower —
`AdapterKind`, `BindingDestination`, `DestinationPostResult` in
`moatless-types` — follows the same rule: name it from `moatless_types::adapter`,
not through this facade.

What is left under `src/adapters/` is what the loops band adds *around* the
contract: the registry that maps a kind to a capability, the durable-delivery
seam, ingress, self-echo, subject events, and the shared connection/loop
handler support. If a change would put a trait every provider implements back in
here, it belongs in the kernel instead.

`scripts/check-crate-seams.sh` enforces this: a `pub use` of another crate's
item anywhere under `src/adapters/` fails unless it is listed in that script's
`ALLOWED_REEXPORTS` with the change that removes it. That list is now empty, and
it got there by shrinking — its one entry was `error.rs` forwarding
`SharedAdapterError` and `ProviderDecodeError` while `AdapterError` still lived
here, and the contract move deleted the module that was forwarding them.

Two features keep the kernel's base surface to the contract alone: `postgres`
adds the connection store and its row shapes, `http` adds the renderer and the
two crates it reads to build a body. Built with neither, `adapter-model` pulls
no database driver, no axum, and no `moatless-core` — check it with
`cargo build -p moatless-adapter-model` before assuming a new item can sit
ungated.

`AdapterConnectionDao` returns `StoreError`, and the Postgres store behind it
sits under the crate's `postgres` feature. A crate that only reads connections
needs no database driver.

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
