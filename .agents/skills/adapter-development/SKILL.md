---
name: adapter-development
description: "Build or change Rust adapters in crates/adapter-*. Use for webhook ingress, durable delivery, outbound replies, reactions, binding status, or adding an adapter."
---

# Adapter development

Read [where a type belongs](../rust-dev-guidelines/references/file-roles.md#choose-a-home-for-a-deduplicated-shared-type),
the nearest crate's AGENTS.md, and [the domain glossary](../../../CONTEXT.md) before choosing a module.

## Ownership

| Responsibility | Owner |
|---|---|
| Sender-specific behavior | `crates/adapter-{github-pr,slack,telegram,linear,webhook}/src` |
| Capability traits, registry, durable-delivery contracts | `crates/adapter-model/src` |
| Delivery worker, binding routing, subscriptions | `crates/loops/src/adapters` and `crates/loops/src/bindings` |
| Provider composition | [adapter_installations.rs](../../../crates/server/src/adapter_installations.rs) |
| HTTP mounts and provider facades | `crates/server/src/adapters` |
| Adapter vocabulary | [AdapterKind](../../../crates/types/src/adapter.rs) |

`crates/adapter-github-pr/src` is a worked module layout. Keep construction in
`adapter.rs`, HTTP extraction in `handlers.rs`, admission in `ingress.rs`,
normalization in `webhook.rs`, and durable-kind registration in `delivery.rs`.
Put each workflow in its owning module. `auth.rs` selects credentials by use case;
`subject.rs` owns persisted subject parsing; `api.rs` owns Sender HTTP calls.
Implement capabilities directly on the adapter. Do not add forwarding methods to
`adapter.rs` or impose empty modules on a provider that does not need them.

When touched behavior belongs in another module, move it with the change and
explain the move in the PR body. Follow the root single-commit policy.
For a new adapter, follow [wiring](references/new-adapter-wiring.md).

## Delivery invariants

- Admit durable work before reporting success. Preserve each ingress outcome's
  HTTP mapping: malformed requests, invalid signatures, and missing delivery
  identities have different policies. Test the owning ingress implementation.
- Kind strings and persisted external IDs are compatibility contracts. Deduplicate
  per adapter, kind, and Sender delivery identity. One kind owns one independent
  effect; fan-out must not depend on execution order.
- Add payload fields with `#[serde(default)]` so queued records survive upgrades.
  Store IDs and reload configuration; do not persist credentials in deliveries.
- A deleted connection is permanent failure. A deleted Subscription gates new
  subscription-created Tasks, but does not stop existing Bindings receiving events.
- Normalization returns values; Sender-to-user lookup is its permitted I/O.
  Apply effects in ingress or durable handlers. Keep audience selection in the
  shared subject-event router and preserve `external_id_aliases` for stored IDs.
- Keep best-effort activity notifications off the durable seam.
- Several buffered events reach the Task as one turn carrying one
  `external_event_id`, the newest. An adapter whose replies must reach a
  specific event renders that event's own id into each part's body.
- A system scope bypass needs a local lint allowance with the reason required by
  the owning crate's AGENTS.md.

## Verification

Test normalization tables, signature and acknowledgment policy, duplicate admission,
queued payload round trips, current-state reload, permanent versus retryable errors,
and the exact advertised capability set. Inject Sender API base URLs and use a local
stub; shared helpers live in [test_support.rs](../../../crates/adapter-model/src/test_support.rs).
Assert API-call count/order when it affects behavior or rate limits.

Run the root Rust verification commands and tests scoped to each changed crate.
Database cases require that crate's `db-tests` feature and a test database; a plain
unit-test pass does not attest to them. CI lints test targets and discovers db-tests
packages. Use [Rust guidance](../rust-dev-guidelines/SKILL.md) for the build matrix.
