# crates

What crosses the banded workspace crates. Dependency bands are defined in
[BANDS](../tooling/arch-rules/src/band_order.rs).

## Where a deduplicated shared type belongs

Work down this list and stop at the first question that answers yes. Reading the
five as independent criteria picks the wrong home: a response body is also domain
data that several crates read, so question 4 would take it from question 1.

1. Is it a request body or a response body itself? → `api-schemas`.
2. Must a pluggable implementer satisfy it? → `adapter-model` or
   `sandbox-runtime`, with the trait it serves.
3. Is it a cross-crate contract? → Consult [port admission](../tooling/arch-rules/src/port_admission.rs).
   Keep a supporting contract type with its trait.
4. Is it pure domain data? → Consult [type admission](../tooling/arch-rules/src/floor_type_admission.rs)
   and the [closed dependency list](../tooling/arch-rules/src/closed_dependency_list.rs).
5. Does the process need it before any domain exists? → `core`.

## Clearing a duplicate-type-name violation

The `duplicate-type-name` rule (`arch-rules`) counts a type as defined in a
crate when the crate declares it, and does not count a re-export.

Consolidate first: delete every copy but one, then re-export the survivor from
each crate that declared its own. A re-export added while both declarations
still stand leaves two definitions and still fires the rule.

A foreign-typed conversion cannot travel with the moved type. Once the DTO lives
in `api-schemas`, write `impl From<sandbox_proto::X> for Foo` as a free function
in the crate that needs it: a crate naming both and owning neither breaks the
orphan rule, and the crate defining `sandbox_proto::X` sits below `api-schemas`
and cannot name `Foo`.

## Clearing a file-size violation

[File size](../tooling/arch-rules/src/file_size.rs) holds a Rust source file
under 1600 lines. The files over the line the day it landed are in
`arch-baseline.json`, and the baseline only shrinks.

Split in this order, and stop as soon as the file is under:

1. Move an inline `#[cfg(test)] mod tests { … }` tail to `foo/tests.rs`, leaving
   `#[cfg(test)] mod tests;` behind. A parent whose only `mod` is that one does
   not become a declaration-only root.
2. Split the extracted test file again under `foo/tests/`, one child per
   subject, with shared fixtures in a sibling named for what it builds.
3. Split the production half into `foo.rs` plus `foo/`. The root then follows
   the `mod.rs` role in `rust-dev-guidelines`: declarations and re-exports only.

An inherent `impl Type` block may be split across children of one crate; a
trait `impl Trait for Type` block may not. Where a trait impl holds the bulk,
keep it in a child of its own and give it one delegating arm per method, as
`crates/server/src/sandbox/grpc_server/dispatch.rs` does.

A baselined file has no ceiling: the violation is keyed on the path, so the
entry clears only when the file drops under the limit. Do not read the baseline
as a per-file cap.
