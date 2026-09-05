# crates

Guidance spanning the banded workspace crates. Read the nearest crate's own
`AGENTS.md` too; this file covers only what crosses them.

Dependency bands are defined in [BANDS](../tooling/arch-rules/src/band_order.rs).

## Where a deduplicated shared type belongs

A type defined in more than one shared-vocabulary crate needs one home. Take the
first question that answers yes. The order is load-bearing and the first yes
wins — a later question never overrides an earlier one.

1. Is it a request or response body itself? → `api-schemas`. The body is the
   wire contract, so it lives with the crate that owns the contract.
2. Must a pluggable implementer satisfy it? → `adapter-model` / `sandbox-runtime`.
   A type an implementer returns or accepts lives with the trait it serves, so
   every implementer speaks one shape.
3. Is it a cross-crate contract? → Consult [port admission](../tooling/arch-rules/src/port_admission.rs).
   Keep supporting contract types with their traits.
4. Is it pure domain data? → Consult [type admission](../tooling/arch-rules/src/floor_type_admission.rs)
   and the [closed dependency list](../tooling/arch-rules/src/closed_dependency_list.rs).
5. Does the process need it before any domain exists? → `core`. Process-level
   config and wiring the runtime reads at startup.

Read as five independent criteria the procedure picks the wrong home: a response
body is also domain data several crates read, so question 4 would take it from
question 1. The first yes is the answer.

## Clearing a duplicate-type-name violation

The `duplicate-type-name` rule (`arch-rules`) counts a type as defined in a crate
when the crate declares it, not when it re-exports another crate's. A cross-crate
`pub use moatless_api_schemas::Foo` is not a definition and collides with nobody.

That clears a collision only because the duplicate declaration was deleted.
Adding the re-export while both declarations still stand leaves two definitions
and still fires the rule. Consolidate — delete every copy but one — then
re-export the survivor from the crates that named their own.

A foreign-typed conversion cannot travel with the moved type. Once the DTO lives
in `api-schemas`, an `impl From<sandbox_proto::X> for Foo` written in a crate
that defines neither side breaks the orphan rule — `sandbox-runtime` names both
`Foo` and `sandbox_proto::X` but owns neither, so `From`, `Foo`, and the proto
type are all foreign there. Write it as a free function in that crate. (The
crate that defines `sandbox_proto::X` could write the impl, since the proto type
is local to it, but that crate is below `api-schemas` and cannot name `Foo`.)
