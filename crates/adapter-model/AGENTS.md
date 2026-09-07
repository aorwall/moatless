# crates/adapter-model

The Adapter contract. `src/lib.rs` names what each module holds, and each
module's own `//!` doc states its contract.
[arch-rules](../../tooling/arch-rules/src/base_surface.rs) enforces the
dependency bands and the feature gates.

## Where a type belongs

Shared domain rows and API presentation values live in `moatless-types`. Name
them from there. Provider implementer inputs and outputs live in this crate.
Loop, Subscription, Schedule, and Task binding SQL rows belong beside their
stores in `moatless-loops`. Decode an `adapter_kind` column with
`moatless_types::adapter::AdapterKind::parse_or_webhook`.

## Keeping the base surface driver-free

Run `cargo build -p moatless-adapter-model` to catch an import that workspace
feature unification hides. The manifest owns the feature-to-dependency mapping.

**A row shape is contract, and its decoder is a driver concern.**
`AdapterDelivery` exposes every field unconditionally, and derives
`sqlx::FromRow` only under `postgres`, with
`#[cfg_attr(feature = "postgres", derive(sqlx::FromRow))]`. A crate that binds
its fields compiles no driver.

**An error that either delivery seam hands a provider names no database
driver.** `AdmitError::Store` carries `StoreError`, so an `sqlx::Error` survives
only as the source.

## What the boundary checks enforce

[Adapter boundaries](../../tooling/arch-rules/src/adapter_boundaries.rs)
enforce provider dependencies and direct imports. [Trait
admission](../../tooling/arch-rules/src/single_implementation_trait.rs) checks
the exercised-fake requirement.
