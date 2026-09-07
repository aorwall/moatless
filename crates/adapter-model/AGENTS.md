# crates/adapter-model

The Adapter contract.
[arch-rules](../../tooling/arch-rules/src/base_surface.rs) enforces the
dependency bands and the feature gates.

## What the kernel holds

| Module | What it holds |
|---|---|
| `capabilities` | What an Adapter offers: the traits a provider implements |
| `apps`, `footer`, `signature`, `mentions` | What an Adapter speaks, `render_mentions` included |
| `persistence` | `AdapterConnectionRow` and its domain decode, gated on `postgres` |
| `access` | What an Adapter gates on: `SourceAccessPolicy`, `MessageTrigger` and `AccessDecision`, the vocabulary a chat provider consults before it processes an inbound message |
| `error` | What an Adapter can fail with: both halves, `SharedAdapterError` and `AdapterError`, plus the `IntoResponse` renderer over them |
| `dao` | Where an Adapter's connections are stored: `AdapterConnectionDao`, returning `StoreError` |
| `delivery` | How inbound work is handed over durably |
| `ingress`, `routing`, `metrics`, `registry` | How a delivery ends |
| `binding`, `loops` | `BindingContext`, `BindingDestination`, and the `BindingProbe` and `LoopReader` read seams |
| `outbound` | Provider reply, attachment, post-result, and task provenance contracts |

`delivery` holds:

- the request, `AdapterDeliveryRequest`;
- what admission did with it, `Admission` and `AdmitError`;
- the record a handler receives, `AdapterDelivery`;
- how a handler reports failure, `AdapterDeliveryFailure`;
- the handler registry, `AdapterDeliveryHandler` and `AdapterDeliveryHandlers`;
- the admission port a webhook handler crosses, `DeliveryAdmission`, which has
  one method, `admit`.

`ingress` holds `Outcome`, `IngressError`, `log_admission`, and
`admit_delivery` over the port. `routing` holds `resolve_chat_provider_event`.
`metrics` holds the ingress and outbound counters. `registry` maps a `kind` to
a capability.

Shared domain rows and API presentation values live in `moatless-types`. Name
them from there. Provider implementer inputs and outputs live in this crate.
Loop, Subscription, Schedule, and Task binding SQL rows belong beside their
stores in `moatless-loops`. Decode an `adapter_kind` column with
`moatless_types::adapter::AdapterKind::parse_or_webhook`.

`moatless-loops` implements `BindingProbe` and `LoopReader` beside the tables
they read. This crate only declares them.

The machinery that *runs* the delivery contract stays in
`moatless_loops::adapters::delivery`: the Postgres store, the supervised
worker, and `AdapterDeliveryPolicy`. This crate holds the contract alone.

## Keeping the base surface driver-free

Run `cargo build -p moatless-adapter-model` to catch an import that workspace
feature unification hides. The manifest owns the feature-to-dependency mapping.

**A row shape is contract, and its decoder is a driver concern.**
`AdapterDelivery` exposes every field unconditionally, and derives
`sqlx::FromRow` only under `postgres`, with
`#[cfg_attr(feature = "postgres", derive(sqlx::FromRow))]`. A crate that binds
its fields compiles no driver.

**An error that either delivery seam hands a provider names no database
driver.** `AdmitError::Store` carries `StoreError`; the loops store wraps it at
the raise site, so the `sqlx::Error` survives only as the source.

## What the boundary checks enforce

[Adapter boundaries](../../tooling/arch-rules/src/adapter_boundaries.rs)
enforce provider dependencies and direct imports. [Trait
admission](../../tooling/arch-rules/src/single_implementation_trait.rs) checks
the exercised-fake requirement.
