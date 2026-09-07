# crates/loops

## What this crate owns

The `task_bindings` table and its `TaskBindingDao` store (`bindings/dao.rs`),
and the four Loop tables and their `LoopDao` store. No other crate declares
either. `bindings/ports.rs` and `loops/ports.rs` document which seam a crate
reads instead of naming a store.

An Adapter names `LoopReader` and never `LoopDao`. One exception stands:
`adapter-telegram/src/pairing/approval.rs` names `LoopDao`, because it reads and
writes a Loop's adapter config while it approves a pairing code. See
`docs/deferred-changes.md`.

SQL row decoding belongs to `loops/persistence.rs` and `bindings/persistence.rs`
beside those stores. The Adapter kernel owns only connection persistence.

## Naming the Adapter kernel

Name the kernel directly as `moatless_adapter_model::…`, from this crate and
from every provider crate. Do not re-export it through `crate::adapters`: a
type gets one path. The shared Adapter vocabulary one crate lower follows the
same rule. Name `AdapterKind` from `moatless_types::adapter`. Provider binding
contexts and destinations belong to `moatless_adapter_model::binding`; outbound
messages and post results belong to `moatless_adapter_model::outbound`.

A trait every provider implements, the registry, an ingress vocabulary, or a
read seam a provider names all belong in the kernel instead of `src/adapters/`.

[Adapter boundaries](../../tooling/arch-rules/src/adapter_boundaries.rs)
enforce these imports.

## Choosing between the two provider seams

Put durable provider work behind `adapters::delivery`, which is inbound and
at-least-once, so every handler must be idempotent. Put work that mirrors a
task's state back to the provider behind `adapters::events`, which is outbound
and best effort. `adapters/delivery/mod.rs` and `adapters/events.rs` state each
contract.

A delivery handler still emits its own `record_webhook`
(`moatless_adapter_model::metrics`). The delivery metric is low-cardinality at
`{adapter, kind, outcome}` and cannot show one broken connection;
`record_webhook` carries `event_type`, `loop_id` and `connection_id`.

Wrap `AdmitError::Store` at its raise site in `adapters/delivery/store.rs`, so
the `sqlx::Error` survives as a `StoreError` source and never enters the type
providers match on.

## Rules for a `TaskEventSubscriber`

`adapters/events.rs` states what a subscriber may do. Two rules are not there:

- **Own the whole reaction, including its ordering.** When a provider keeps
  per-task state, such as Linear's turn gate, the subscriber owns that state.
  Emit the event and change the state there, in the order the provider needs.
  Split the two across bands and the gate closes before the activity it gates is
  emitted.
- **Do not rely on running before another subscriber.** Registration order is no
  contract. A subscriber does run after the generic handler for its event, which
  is what makes a reaction keyed on persisted state safe.
