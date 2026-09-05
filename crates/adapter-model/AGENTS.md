# crates/adapter-model

The Adapter contract. Dependency bands and feature gates are enforced by
[arch-rules](../../tooling/arch-rules/src/base_surface.rs).

## What the kernel holds

- **What an Adapter offers** — `capabilities`, the traits a provider implements.
- **What it speaks** — `apps`, `footer`, `signature`, `mentions`
  (`render_mentions`).
- **The shapes it persists** — `persistence`: the `*Row` structs and their
  `From<…Row>` decode into the domain types, `postgres`-gated. The domain types
  themselves live in `moatless-types`; name them from there, not through this
  crate, and decode an `adapter_kind` column with
  `moatless_types::adapter::AdapterKind::parse_or_webhook`.
- **The access it gates on** — `access`: `SourceAccessPolicy`, `MessageTrigger`,
  `AccessDecision`, the vocabulary a chat provider consults before it processes
  an inbound message.
- **What it can fail with** — `error`: both halves, `SharedAdapterError` and
  `AdapterError`, plus the `IntoResponse` renderer over them.
- **Where its connections are stored** — `dao` (`AdapterConnectionDao`,
  returning `StoreError`).
- **How inbound work is handed over durably** — `delivery`: the request
  (`AdapterDeliveryRequest`), what admission did with it (`Admission`,
  `AdmitError`), the record a handler receives (`AdapterDelivery`), how it
  reports failure (`AdapterDeliveryFailure`), the handler registry
  (`AdapterDeliveryHandler`, `AdapterDeliveryHandlers`), and the admission port
  a webhook handler crosses (`DeliveryAdmission`, one method — `admit`).
- **How a delivery ends** — `ingress` (`Outcome`, `IngressError`,
  `log_admission`, and `admit_delivery` over the port) and `routing`
  (`resolve_chat_provider_event`), plus the ingress/outbound counters
  (`metrics`), and the `registry` that maps a `kind` to a capability.
- **The two read seams a provider names but does not own** — `BindingProbe`
  (`binding`) and `LoopReader` (`loops`). Declared here; `moatless-loops`
  implements them beside the tables they read.

The machinery that *runs* the delivery contract — the Postgres store, the
supervised worker, `AdapterDeliveryPolicy` — stays in
`moatless_loops::adapters::delivery`. The contract descends; the machine that
keeps it does not.

## Feature boundaries

Run `cargo build -p moatless-adapter-model` to catch imports hidden by workspace
feature unification. The manifest owns the feature-to-dependency mapping.

**A row shape is contract; its decoder is a driver concern.** `AdapterDelivery`
exposes every field unconditionally and derives `sqlx::FromRow` only under
`postgres` (`#[cfg_attr(feature = "postgres", derive(sqlx::FromRow))]`). A crate
that binds its fields compiles no driver.

**An error either delivery seam hands a provider names no database driver.**
`AdmitError::Store` carries `StoreError`, wrapped at the raise site in the loops
store, so the `sqlx::Error` survives as the source without appearing in the type
provider crates match on.

## Boundaries

[Adapter boundaries](../../tooling/arch-rules/src/adapter_boundaries.rs) enforce
provider dependencies and direct imports. The exercised-fake requirement is
checked by [trait admission](../../tooling/arch-rules/src/single_implementation_trait.rs).
