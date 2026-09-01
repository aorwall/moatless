# crates/adapter-model

The Adapter kernel: the whole Adapter contract in one crate. It sits above
`moatless-types`, `moatless-core`, `moatless-api-schemas` and
`moatless-backend-core`, and below the loops band and every provider crate
(`adapter-slack`, `adapter-webhook`, `adapter-telegram`, `adapter-github-pr`,
`adapter-linear`). A provider implements an Adapter by naming this crate and
nothing above it; the loops band that orchestrates providers names the same
items from the same paths.

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

## Features keep the base surface to the contract

- `postgres = ["dep:sqlx"]` — the connection store and the row shapes it
  decodes.
- `http = ["dep:axum", "dep:moatless-api-schemas", "dep:moatless-core"]` — the
  `IntoResponse` rendering of `AdapterError`.
- `test-support = ["http", "dep:tokio"]` — the error/renderer test helpers, the
  loopback provider stubs, and `FakeAdmission`.

Built with neither `postgres` nor `http`, the kernel pulls no database driver,
no axum, and no `moatless-core`. `tracing`, `regex` and `moatless-backend-core`
are unconditional. A `--workspace` clippy run unifies features and hides an
ungated import, so check the floor with a dedicated per-crate build —
`cargo build -p moatless-adapter-model` — before assuming a new item can sit
ungated, and `cargo tree -p moatless-adapter-model --no-default-features
-e normal` for the driver/axum count.

**A row shape is contract; its decoder is a driver concern.** `AdapterDelivery`
exposes every field unconditionally and derives `sqlx::FromRow` only under
`postgres` (`#[cfg_attr(feature = "postgres", derive(sqlx::FromRow))]`). A crate
that binds its fields compiles no driver.

**An error either delivery seam hands a provider names no database driver.**
`AdmitError::Store` carries `StoreError`, wrapped at the raise site in the loops
store, so the `sqlx::Error` survives as the source without appearing in the type
provider crates match on.

## One path to every type

Name kernel items directly — `moatless_adapter_model::…` — from the loops band
and from every provider crate. Do not re-export them through
`crate::adapters`; one path to a type is the point, enforced by
`scripts/check-crate-seams.sh` invariant 3.

## The provider crates that read no loops band

`adapter-slack` and `adapter-webhook` implement the contract by naming this
crate alone; neither declares `moatless-loops`. `adapter-telegram` keeps one
edge — `pairing.rs`, which reads and writes a Loop's config through `LoopDao`
(a behaviour question recorded in `docs/deferred-changes.md`).
`scripts/check-crate-seams.sh` invariant 4 fails the build if either freed crate
names `moatless_loops` anywhere under `src/`, or if telegram names it outside
that one file.

## A trait here earns its keep with a second implementation

The repo forbids a trait with a single implementation. `DeliveryAdmission` has
two: `AdapterDeliveryStore` (the real durable store, in `moatless-loops`) and
`FakeAdmission` (in `test_support`), and the second is what lets a webhook
handler assert every admission outcome — admitted, duplicate, at-capacity,
store-failed — without a database. Do not add a kernel trait without a real
second implementer or a test that drives a fake one.
