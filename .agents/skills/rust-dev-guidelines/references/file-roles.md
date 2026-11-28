# Assign file responsibilities

Apply these contracts when writing or reviewing Rust in `crates/` and `tooling/`.
Files are optional. Their responsibilities are fixed when present.
Group files by the domain or capability that owns the behavior.

## Keep roots and entry points distinct

| File | Owns | Keep elsewhere |
|---|---|---|
| `lib.rs` | Crate documentation, crate attributes, top-level module declarations and intentional public re-exports. | Type definitions, operation bodies, dependency construction and process startup. |
| `mod.rs` | One directory module's documentation, child declarations and selected re-exports. | Type definitions, operation bodies, business logic and runtime wiring. |
| `main.rs` | Executable entry, runtime selection, argument parsing, application invocation and process exit. | Resource initialization, router construction, worker supervision and domain workflows. |
| `app.rs` | Application startup order, starting servers and workers, waiting for termination and coordinated shutdown. | Domain decisions and concrete dependency construction. |
| `compose.rs` | Construct concrete dependencies and connect them through existing interfaces. | Request handling, business decisions and lifecycle wait loops. |

The `main.rs` role applies to every executable target root, including `src/bin/`
and custom Cargo target paths. The `lib.rs` role also applies to a custom library root.
Every module root that declares production children follows the `mod.rs` role,
including the `foo.rs` plus `foo/` layout and inline production namespaces.
Declaring only test children does not turn an implementation file into a namespace root.
Generated protocol inclusion belongs with declarations; do not hand-edit generated output.
Test-module declarations and crate-level lint attributes may remain in roots.
Put handwritten tests in the owning implementation module or a declared test module.
Cargo integration-test `main.rs` files are test roots, not application entry points.

An application needs `app.rs` only when it owns a process lifecycle.
When composition grows into `compose/`, its `mod.rs` declares and re-exports.
Put construction functions in its named child modules.
Re-export supported paths by name after moving implementations (`pub use child::Item`);
do not widen visibility to simplify imports. Sibling glob re-exports (`pub use child::*`)
into one parent make `arch-report` intractable (`docs/deferred-changes.md`).
Use a distinct capability name such as `docker_compose.rs` for Docker Compose operations;
`compose.rs` denotes dependency assembly.

## Separate HTTP from domain behavior

| File | Owns | Keep elsewhere |
|---|---|---|
| `routes.rs` | Register paths, HTTP methods, handlers and transport middleware; return a router. | Business logic, handler bodies, SQL, credential resolution and workflow orchestration. |
| `handlers.rs` | Decode transport input, obtain caller identity, invoke an owning operation and map its result to HTTP. | Business validation, resource-access policy, persistence implementation and workflow decisions. |
| `service.rs` or a named operation file | Domain policy and the operation's decisions, ordering and side effects. | HTTP response construction and route registration. |
| `dao.rs` | The owning persistence interface, PostgreSQL implementation and database operations. | HTTP adaptation and unrelated domain workflows. |

`routes.rs` contains no business logic, including closures supplied to route builders.
Register authentication middleware there; implement authentication in its owning module.
Handlers may invoke authorization, but the domain owns the policy and its enforcement.
Put domain rules behind operations used by every entry point, including CLI, RPC and Adapter calls.
Move domain behavior out even when only one handler currently calls it.
A simple read may call an injected DAO directly; do not add a pass-through service.
Direct pool access in a handler is an exception recorded in the nearest `AGENTS.md`.
During a review, name that exception separately from compliance with the default contract.

Prefer an operation name such as `creation.rs`, `send.rs` or `closer.rs` when it identifies the responsibility precisely.
Do not create a service, trait or layer solely to fill a file template.
Keep database transaction ownership intact when dividing an implementation.

## Name model and contract files precisely

| Location | Responsibility |
|---|---|
| Domain `models.rs` | Local domain or persistence shapes and explicit re-exports. Distinguish row types from domain types by name. |
| Domain `schemas.rs` | HTTP request/response definitions or re-exports from the owning schema crate. |
| `workspace-types/src/{workspace,repository,plugin}.rs` | Workspace domain values. |
| `workspace-types/src/schemas/` | Workspace request/response contracts, behind `schemas`. |
| `workspace-ports/src/` | Workspace capabilities and supporting contract types. |
| `types/src/<domain>.rs` | Pure shared domain values admitted by the repository's type rules. |
| `api-schemas/src/<domain>.rs` | Shared API request/response contracts. |
| `ports/src/<domain>/` | Cross-crate capability declarations and their supporting contract types. |
| Domain `ports.rs` | Implement the published capabilities over the owning domain. |
| Module-local `ports.rs` | Declare capabilities consumed by that module when it owns a separate seam, such as T3 projections. |

A `ports.rs` module doc states whether it declares or implements contracts.
When one file does both, separate the roles into named children and keep supported paths.
Schema modules may also adapt domain values into their wire contracts.
An `error.rs` owns error types and conversions for its enclosing concept.
Retain error causes; apply HTTP status mapping and redaction at the transport boundary.
An HTTP-facing domain may colocate its `IntoResponse` adapter in `error.rs`;
domain operations return typed errors, and pure shared error types stay transport-free.

## Review contents and ownership

1. Read the crate manifest, module declarations, local guidance and relevant deferred changes.
2. Inventory every source module. Inspect definitions, callers and tests to establish what each module actually does.
3. Compare each role with its contents. Record mixed responsibilities and behavior that belongs to another owner.
4. For each finding, name the symbol, source line, callers, owning module, proposed move and behavior to preserve.
5. Check public paths, dependency bands, architecture rules and validation requirements before recommending the move.

Check lifetime as well as file names. Work used after startup belongs to its runtime concept, not under `init/`.
For example, workspace sync serves startup, Turn completion, periodic work, pre-stop requests and shutdown.
Its callers should reach one workspace-sync owner with shared state.

Report responsibility violations separately from optional readability changes and known exceptions.
File length is evidence of reading cost, not proof that ownership is wrong.
Splitting a file does not resolve misplaced behavior or an overexposed interface.
Do not mark a crate compliant from its directory tree alone.

## Choose a home for a deduplicated shared type

Stop at the first question that answers yes; read as independent criteria,
question 4 would take a response body from question 1.

1. A request or response body? `workspace-types::schemas` for Workspace,
   repository and plugin contracts, `api-schemas` for other domains, and
   `types::http` for the common error body.
2. Satisfied by a pluggable implementer? Beside its trait, in `adapter-model`
   or `sandbox-runtime`.
3. A cross-crate contract? `workspace-ports` or `ports/src/<domain>/`, as
   `tooling/arch-rules/src/port_admission.rs` admits.
4. Pure domain data? `workspace-types` or `types/src/<domain>.rs`, as
   `floor_type_admission.rs` and `closed_dependency_list.rs` admit.
5. Needed before any domain exists? `core`.

## Clear a duplicate-type-name violation

The `duplicate-type-name` rule (`arch-rules`) counts a type as defined in a
crate when the crate declares it, and does not count a re-export.

Consolidate first: delete every copy but one, then re-export the survivor from
each crate that declared its own. A re-export added while both declarations
still stand leaves two definitions and still fires the rule.

A foreign-typed conversion cannot travel with the moved type. Once the DTO
lives in `api-schemas`, do not write `impl From<sandbox_proto::X> for Foo`: a
crate naming both types and owning neither breaks the orphan rule, and the
crate defining `sandbox_proto::X` sits below `api-schemas` and cannot name
`Foo`. Write a free function instead, in the crate that needs the conversion.

## Clear a file-size violation

`tooling/arch-rules/src/file_size.rs` holds a Rust source file under 1600
lines. The files over the line the day it landed are in `arch-baseline.json`,
and the baseline only shrinks.

Split in this order, and stop as soon as the file is under:

1. Move an inline `#[cfg(test)] mod tests { … }` tail to `foo/tests.rs`, leaving
   `#[cfg(test)] mod tests;` behind. A parent whose only `mod` is that one does
   not become a declaration-only root.
2. Split the extracted test file again under `foo/tests/`, one child per
   subject, with shared fixtures in a sibling named for what it builds.
3. Split the production half into `foo.rs` plus `foo/`. The root then follows
   the `mod.rs` role above: declarations and re-exports only.

An inherent `impl Type` block may be split across children of one crate; a
trait `impl Trait for Type` block may not. Where a trait impl holds the bulk,
keep it in a child of its own and give it one delegating arm per method, as
`crates/server/src/sandbox/grpc_server/dispatch.rs` does.

A baselined file has no ceiling: the violation is keyed on the path, so the
entry clears only when the file drops under the limit. Do not read the
baseline as a per-file cap.

## Enforce the structural boundary

`cargo test -p moatless-arch-rules` checks declaration-only roots, route
registration, dependencies toward transport, SQL placement, process exit and
composition boundaries. These structural rules have no baseline entries (unlike
`file_size`); keep them at zero.
The checker includes executable target roots and the excluded Codex SDK source.

Read `tooling/arch-rules/src/composition_root.rs` before proposing new
construction paths. A passing structural check does not prove that a handler
contains no business policy: inspect its decisions and effects using the review
steps above. Report required checker changes with a move. Never add baseline
debt to conceal a violation; document any one-to-one relocation of existing debt.
