# Enforce the Rust workspace architecture

> **Status:** implemented
> **Updated:** 2026-09-05
> **Vocabulary:** `CONTEXT.md`
> **Predecessor:** `.plans/backend-ports-and-crates.md`

## Scope

Move decidable architecture constraints into `tooling/arch-rules`, consolidate
Cargo inheritance, and put shared contracts and infrastructure in their owning
crates. Preserve runtime behavior, serialized values, OpenAPI schemas, database
migrations, dependency versions, and binary names.

The `architecture` CI job runs on every PR and push covered by `ci.yml`, without
consulting the changed-package selector. It runs
`cargo nextest run --locked -p moatless-arch-rules`.

## Workspace layout

| Directory | Package | Change |
|---|---|---|
| `crates/server` | `moatless-backend` | Remove the 24 top-level re-export facades and move the backend directory |
| `crates/sandbox` | `moatless-sandbox` | Move the sandbox directory |
| `crates/ports` | `moatless-ports` | Rename backend-core and retain contracts |
| `tooling/arch-rules` | `moatless-arch-rules` | Move architecture tooling outside application crates |

The server's `migrations` symlink still targets `../core/migrations`.
`CLAUDE.md` symlinks travel with their `AGENTS.md` files.
Docker contexts, workflow paths, task commands, scripts, and development guidance
must resolve these directories. Backend and sandbox package/binary names stay stable.

Every workspace member inherits `version`, `edition`, `authors`, and workspace
lints. Workspace path dependencies and external dependencies used by at least
three members are centralized. Features remain explicit at each consumer.
Generated protobuf code has a scoped Clippy exemption; handwritten code inherits
the workspace checks.

## Contract and infrastructure placement

| Item | Owner |
|---|---|
| Change-feed machinery and database enum helpers | `moatless_core::{change_feed, db_enum}` |
| Store error | `moatless_types::store::StoreError`, imported directly |
| Running state, UI-RPC values, access-control helpers | `moatless_types` |
| Activity values and pure helpers | `moatless_types::activity` |
| Activity SQL decoding | Private `StoredActivity` row wrapper beside queries in `moatless_tasks::task_activities::dao` |
| `ActivitySource` | `moatless_ports::activity`, with a reasoned admission exception |
| `TaskBindings` | `moatless_tasks::task::bindings` |
| Message wake-failure helper | `moatless_tasks::message::wake`, used by tasks and loops |
| Reserved-port and absent-sandbox value helpers | `moatless_types::sandbox` |
| Codex payload vocabulary and `PluginDelivery` | `moatless_types`; API schemas retain their public re-export paths |
| Sandbox JWT signing and verification | `moatless_core::sandbox_jwt` |
| Observation publishing, event identities, `PublishDecision` | `moatless_sandbox_runtime::observation_events` |
| `classify_observation` | `moatless_sandbox_control::lifecycle::watcher` |

Ports contain traits **and supporting contract types**, including transitive
signature vocabulary. They do not hold unrelated exported data or policy helpers.
Removing database machinery also removes the ports crate's `database` feature,
SQL driver, and hashing dependencies.

The watcher classifier calls `Lifecycle`, so it cannot move into runtime without
changing the lifecycle boundary. Drivers retain exact manifest exceptions for
their dependency on sandbox-control, covering lifecycle and provisioning.
No replacement single-implementation trait is introduced for that edge.

Activity decoding preserves the existing conversions and failure behavior.
The private wrapper keeps the SQL trait implementation with a locally owned type.
The message helper belongs to tasks because both tasks and loops call it.

## Rule inputs and identities

`Workspace` reads Cargo metadata and raw manifests. A cached source index walks
Cargo targets, module declarations, inline and file modules, explicit paths,
imports, aliases, and re-exports. Test-only items are separate from production
consumers. Supported contract macros contribute generated enum/mock identities.
The index is syntactic; it does not replace Rust type checking or expand arbitrary
procedural macros.

Violation keys identify declarations and their participants, without line numbers.
The baseline must lose entries when violations disappear. Existing debt outside
the planned moves is baselined when each checker first lands. Deliberate exceptions
are recorded at the source rather than mixed into that debt.

| Rule | Authority |
|---|---|
| `band-order` | `band_order::BANDS`; every member assigned, normal edges strictly downward |
| `dependency-privacy` | Complementary privacy bans, including edges that point down a band |
| `manifest-inheritance` | Raw member manifests |
| `closed-dependency-list` | Allowed value dependencies of types |
| `base-surface` | Adapter transport dependencies optional, directly owned by one feature, unreachable from defaults |
| `floor-type-admission` | Three production consumer crates, or two in different bands; declared dependency boundary |
| `port-admission` | Two independent non-implementing consumers; supporting types allowed |
| `single-implementation-trait` | Multiple production implementations or an exercised test double |
| `row-stays-with-query` | Derived and manual `FromRow` implementations require a direct SQL dependency |
| `composition-root` | State assembly and concrete DAO construction |
| `table-ownership` | SQL literals and table ownership reconciled with migration history |
| `store-privacy` | Cross-crate DAO access and direct SQL-driver ownership |
| `adapter-boundaries` | Binding/Adapter module separation and provider import restrictions |
| `agents-symlink` | Existing sibling `CLAUDE.md` symlink to `AGENTS.md` |
| `arch-allow-names-a-rule` | Registered, reasoned exceptions at applicable declarations |
| `duplicate-type-name` | Public definitions, excluding foreign re-exports |
| `generic-module-name` | Domain-based module naming |

The five shell checks and their three baselines are replaced by these rules and
`arch-baseline.json`. Each rule has an enforcing test. A registry test rejects
unregistered findings, and the baseline test rejects stale entries.

T3's projection exemption permits reads; foreign writes remain findings. Its
existing direct workspace-script update is baselined and recorded in
`docs/deferred-changes.md`. Existing AppState and Binding/Adapter debt retains
its original scope, with declaration-level identities.

## Exceptions

Dependency exceptions are exact source-package/target-package edges:

```toml
[[package.metadata.arch.allow]]
rule = "band-order"
dependency = "moatless-sandbox-control"
reason = "The driver invokes lifecycle transitions and builds provisioning plans."
```

Rust-item exceptions use one doc line per rule:

```rust
/// arch-allow(port-admission): Keeps the T3 read projection independent of task services.
pub trait ActivitySource { /* contract */ }
```

A dependency exception cannot be placed on a Rust item. Unknown rules, empty
reasons, undeclared dependency targets, and inapplicable item tags fail validation.
Codex payload components and Activity status carry type-admission exceptions so
the shared wire vocabulary can travel together without splitting its schema.

## Verification

- Architecture fixtures cover valid and invalid edges, aliases, optional and
  target dependencies, dev/build exclusions, exact exceptions, module visibility,
  test-only sources, mocks, re-exports, SQL ownership, and composition roots.
- Run `cargo test -p moatless-arch-rules` against the reviewed baseline.
- Run the workspace Clippy floor and the required core, ports, and backend feature
  tiers from root `AGENTS.md`, verbatim.
- Run crate-scoped tests for the affected members; never `cargo test --workspace`.
- Regenerate OpenAPI and compare it byte-for-byte with the committed schema.
- Check Docker/task/workflow paths, migrations and instruction symlinks, and the
  ports dependency tree. No SQL driver may appear in the ports tree.
- Review renames separately from content changes and reject unrelated formatting.

No deployment, schema-domain split, migration, or lifecycle redesign is part of
this change. The global `ARCH_STRICT=1` switch remains optional until all existing
architecture debt is removed.

Verification completed: all 27 workspace members passed crate-scoped tests;
architecture passed 56 unit tests and 19 integration tests. The Clippy floor and
all five required feature tiers passed. Frontend type-checking, Biome, the 42 plan
assertions, and Docker build-definition checks passed. OpenAPI remained
byte-identical. Taskfile build commands were checked with a dry run; full Docker
image builds and deployed E2E tests were not run.
