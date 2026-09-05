# moatless-types

Shared type definitions. Pure data, no I/O.

Admission and dependency constraints live in [arch-rules](../../tooling/arch-rules/src/floor_type_admission.rs).

## Load-Bearing Rules

- **The module is the domain, named from `CONTEXT.md`.** A reader who knows
  which domain a type belongs to knows which file to open. Do not add a module
  named for the kind of thing it holds — `config`, `error`, `kind`, `delivery`
  — because none of those is a domain. Bare "delivery" is on the glossary's
  `_Avoid_` list; the term is Adapter delivery, and those types are adapter
  vocabulary.
- **An entity here is the domain model, never the row.** Every field is the
  value the codebase reads: a `DateTime<Utc>`, not a text timestamp; an enum,
  not the string a column holds; a decoded blob, not `serde_json::Value`. A
  persistence artifact — a `_raw` twin read through an accessor, a nullable
  flag standing in for a `bool` — does not belong on it.
- Decode database rows in the query's crate. Preserve each decoder's failure
  behavior and logging when moving its domain type.
- **Renaming a module must not rename a schema.** No type here carries
  `#[schema(as = …)]`, so an OpenAPI schema name derives from the type name
  alone, and `packages/api-client/src/sandbox/model/` mirrors it in TypeScript.
  After any move, `task generate:spec` must leave `openapi-specs.json`
  byte-identical. Explicit `#[serde(rename)]` pins travel with their type.
- **`clippy.toml` names `access_control::ScopeContext::system` by full path.**
  An unresolvable `disallowed-methods` path fails silently, so moving or
  renaming that type disarms the access-control fence with no error. Verify by
  adding a temporary unannotated call and confirming clippy refuses it.
- **A type may sit beside a name it reads unlike.** `sandbox::AgentStatus`
  belongs to the live-status cluster, not to `agent`. State the reason in the
  module doc so the next reader does not "fix" it.
