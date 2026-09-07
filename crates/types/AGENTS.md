# moatless-types

Admission and dependency constraints live in
[arch-rules](../../tooling/arch-rules/src/floor_type_admission.rs).

## Rules for adding or moving a type

- `string_enum` is private macro support shared by domain modules. It defines
  no domain of its own; keep its invocation name stable for architecture analysis.
- **The module is the domain, named from `CONTEXT.md`.** A reader who knows
  which domain a type belongs to then knows which file to open. Do not add a
  module named for the kind of thing it holds. `config`, `error`, `kind` and
  `delivery` are none of them a domain. The glossary's `_Avoid_` list carries
  bare "delivery": the term is Adapter delivery, and those types are adapter
  vocabulary.
- **An entity here is the domain model, never the row.** Every field holds the
  value the codebase reads. Use a `DateTime<Utc>` and not a text timestamp, an
  enum and not the string a column holds, a decoded blob and not a
  `serde_json::Value`. A persistence artifact does not belong on it: no `_raw`
  twin read through an accessor, and no nullable flag standing in for a `bool`.
- Decode a database row in the query's own crate. Preserve each decoder's
  failure behavior and logging when you move its domain type.
- **Renaming a module must not rename a schema.** No type here carries
  `#[schema(as = …)]`, so an OpenAPI schema name derives from the type name
  alone, and `packages/api-client/src/sandbox/model/` mirrors it in TypeScript.
  After any move, `task generate:spec` must leave `openapi-specs.json`
  byte-identical. An explicit `#[serde(rename)]` pin travels with its type.
- **`clippy.toml` names `access_control::ScopeContext::system` by full path.** An
  unresolvable `disallowed-methods` path fails silently, so moving or renaming
  that type disarms the access-control fence and reports no error. Verify by
  adding a temporary unannotated call and confirming that clippy refuses it.
- **A type may sit beside a name it reads unlike.** `sandbox::AgentStatus`
  belongs to the live-status cluster and not to `agent`. State the reason in the
  module doc, so the next reader does not "fix" it.
