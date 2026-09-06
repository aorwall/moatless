# crates

Guidance that spans the banded workspace crates. Read the nearest crate's own
`AGENTS.md` too. This file covers only what crosses them.

Dependency bands are defined in
[BANDS](../tooling/arch-rules/src/band_order.rs).

## Where a deduplicated shared type belongs

A type defined in more than one shared-vocabulary crate needs one home. Work
down this list and stop at the first question that answers yes. A later
question never overrides an earlier one.

1. Is it a request body or a response body itself? → `api-schemas`. The body is
   the wire contract, so it lives with the crate that owns the contract.
2. Must a pluggable implementer satisfy it? → `adapter-model` or
   `sandbox-runtime`. A type an implementer returns or accepts lives with the
   trait it serves, so every implementer reads one shape.
3. Is it a cross-crate contract? → Consult [port admission](../tooling/arch-rules/src/port_admission.rs).
   Keep a supporting contract type with its trait.
4. Is it pure domain data? → Consult [type admission](../tooling/arch-rules/src/floor_type_admission.rs)
   and the [closed dependency list](../tooling/arch-rules/src/closed_dependency_list.rs).
5. Does the process need it before any domain exists? → `core`. That crate holds
   process-level config and the wiring the runtime reads at startup.

Read the five as independent criteria and the procedure picks the wrong home. A
response body is also domain data that several crates read, so question 4 would
take it from question 1.

## Clearing a duplicate-type-name violation

The `duplicate-type-name` rule (`arch-rules`) counts a type as defined in a
crate when the crate declares it. It does not count a re-export of another
crate's type, so a cross-crate `pub use moatless_api_schemas::Foo` collides
with nobody.

Consolidate first: delete every copy but one. Then re-export the survivor from
each crate that declared its own. A re-export added while both declarations
still stand leaves two definitions and still fires the rule.

A foreign-typed conversion cannot travel with the moved type. Once the DTO
lives in `api-schemas`, an `impl From<sandbox_proto::X> for Foo` breaks the
orphan rule in a crate that defines neither side. `sandbox-runtime` names both
`Foo` and `sandbox_proto::X` and owns neither, so `From`, `Foo` and the proto
type are all foreign there. Write it as a free function in that crate. The
crate that defines `sandbox_proto::X` could write the impl, because the proto
type is local to it. That crate sits below `api-schemas` and cannot name `Foo`.
