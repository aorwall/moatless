# Backend Crate Split Plan

## Summary

Split the current `moatless-vibe-backend` crate into smaller Rust crates without
changing runtime behavior. The goal is to reduce compile surface area, make
domain ownership clearer, and let shared backend code be reused by the CLI,
integration tests, or future worker binaries without depending on the full Axum
server, Kubernetes provider, Redis consumer, and S3 stack.

This should be an incremental refactor. Each phase must keep the backend binary
working and pass the same backend checks before moving on.

## Current State

The backend currently exposes nearly every module from `backend/src/lib.rs`:

```rust
pub mod access_control;
pub mod adapters;
pub mod auth;
pub mod bindings;
pub mod command;
pub mod config;
pub mod db;
pub mod events;
pub mod message;
pub mod repository;
pub mod sandbox;
pub mod state;
pub mod task;
// ...
```

`AppState` centralizes infrastructure and repository trait objects in
`backend/src/state.rs`. This makes route handlers easy to wire, but it also
pulls many unrelated dependencies into every backend build:

- HTTP/API: `axum`, `utoipa`, `utoipa-axum`
- persistence: `sqlx`, PostgreSQL models/DAOs
- infrastructure: `kube`, `k8s-openapi`, `bollard`, `aws-sdk-s3`, `redis`
- auth/crypto: `jsonwebtoken`, `aes-gcm`, `hmac`, `sha2`
- transport: `tonic`, `prost`, `sandbox-proto`

Existing shared crates already follow the desired direction:

- `crates/api-schemas`: API DTOs shared with the CLI
- `crates/api-client`: CLI/client HTTP wrapper
- `crates/events`: canonical typed event payloads
- `crates/sandbox-proto`: gRPC protocol
- `crates/claude-agent-sdk`: agent process protocol

The split should extend this pattern, not introduce a second architecture.

## First Breakout Results (2026-07-15)

Phase 0 and the first Phase 1 extraction were completed with the pinned Rust
1.97.0 toolchain (`rustc 1.97.0`, `cargo 1.97.0`). Rustup initially recorded
the `cargo` and `clippy` components as installed even though their binaries were
missing; both components were reinstalled before recording build, lint, or test
results.

### Baseline and import observations

- The baseline backend tree had 54 direct normal dependencies and included the
  full Axum, SQLx, Kubernetes, AWS, Redis, crypto, and gRPC stacks.
- The captured `use crate::...` graph contained 742 import lines. The largest
  top-level edge groups were `adapters` (118), `state` (72), `auth` (66),
  `sandbox` (50), and `db` (43); 33 import lines targeted `scope`.
- `crate::scope` appeared anywhere in 39 production files. The compatibility
  path appeared in five backend test files:
  `repository_handler_test.rs`, `repository_dao_test.rs`, `task_dao_test.rs`,
  `agent_handlers_hooks_test.rs`, and `tests/common/mod.rs`.
- After extraction, the backend has 55 direct normal dependencies because it
  now depends on `moatless-backend-core`. The new crate's only direct normal
  dependency is `moatless-api-schemas`; `serde_json` is test-only.

### Build timings

Both timing pairs used the same pinned toolchain and commands: first
`cargo clean -p moatless-vibe-backend`, then two consecutive
`/usr/bin/time -p rtk cargo check -p moatless-vibe-backend` invocations.
These are single local samples with a warm dependency cache, so they are useful
as directional package-build observations rather than benchmarks.

| Backend check | Before | After |
|---------------|-------:|------:|
| Clean-package real time | 53.73s | 20.09s |
| No-op incremental real time | 0.60s | 0.24s |

### Boundary decisions

- `scope.rs` moved intact to `moatless-backend-core`, with `Scope` remaining
  canonical in `moatless-api-schemas`.
- Production consumers use `moatless_backend_core::scope`; a documented
  `moatless_vibe_backend::scope` re-export remains for one extraction phase and
  existing backend tests intentionally continue using it.
- `access_control.rs` remains in the backend because its database-backed checks
  depend on `AppState`, authentication, users, teams, and SQLx.
- `error.rs` remains in the backend because it is coupled to Axum and OpenAPI.
- No domain module, HTTP surface, database schema, serialization shape, or
  runtime behavior moved in this breakout.
- Crate named `moatless-backend-core` (layer-prefixed), not `moatless-core`.
  Scope policy is backend-only; a product-wide `core` name is reserved for
  primitives genuinely shared with the sandbox and CLI. See
  [Crate Naming Conventions](#crate-naming-conventions).

### Validation results

- `cargo clippy -p moatless-backend-core -- -D warnings`: passed.
- `cargo test -p moatless-backend-core`: 16 tests passed (12 moved policy/filter
  tests plus four new public-helper behavior tests).
- `cargo clippy -p moatless-vibe-backend -- -D warnings`: passed.
- `cargo test -p moatless-vibe-backend`: 860 tests passed. Together with the 12
  moved tests this preserves the 872-test baseline; the four added core tests
  raise the combined count to 876.
- `cargo tree -p moatless-backend-core --edges normal`: confirmed the intended
  direct dependency boundary.
- Production search for `crate::scope`: no matches. All five affected backend
  test files still use the compatibility path.
- `cargo check -p moatless-vibe-backend --tests --features db-tests`: blocked by
  26 pre-existing stale db-test compile errors unrelated to this extraction,
  including removed `channel` APIs, outdated config initializers, and old DAO
  signatures.
- `cargo fmt --all -- --check`: blocked by repository-wide Rust 1.97 formatting
  drift across many out-of-scope files. The check was non-mutating; applying it
  would create a broad formatting-only diff unrelated to this breakout.

### Follow-ups opened by this phase

- Remove the `moatless_vibe_backend::scope` compat re-export in the next
  extraction phase; five backend test files still use it
  (`repository_handler_test`, `repository_dao_test`, `task_dao_test`,
  `agent_handlers_hooks_test`, `tests/common/mod.rs`). Convert them to
  `moatless_backend_core::scope` and delete the re-export.
- The `db-tests` tier does not compile (26 pre-existing errors: removed
  `channel` APIs, outdated config initializers, old DAO signatures). Track and
  fix separately — until then that test tier is unguarded, which the next DAO
  crate extraction will hit blind.

## Goals

- Keep the deployable backend binary at `backend/`.
- Extract stable, reusable backend domain code into `crates/*`.
- Preserve Axum routes, OpenAPI generation, migrations, and runtime behavior.
- Make crate dependencies one-way and obvious.
- Avoid a large "big bang" move that blocks feature work.

## Non-Goals

- No database schema changes.
- No API contract changes.
- No frontend changes.
- No rewrite of `AppState` into a framework or DI container.
- No attempt to split every backend module immediately.

## Proposed Target Shape

```text
backend/
  src/
    main.rs              # binary entry point
    startup.rs           # Axum server wiring
    state.rs             # composition root
    routes only where tied to full AppState / Axum server

crates/
  backend-core/          # shared errors, scope, pure policy/types
  backend-db/            # sqlx DAO helpers and reusable repository traits
  backend-domain/        # low-infrastructure domains: tags, commands, feedback
  backend-auth/          # auth/session/token logic, after core split stabilizes
  backend-sandbox/       # sandbox providers, K8s/Docker clients, gRPC bridge
  backend-adapters/      # Slack/GitHub/Linear/Telegram adapters and bindings
```

This is a directional target. The first implementation should create only the
crate or crates needed for the current phase.

## Dependency Rules

Use these rules to prevent cycles:

```text
backend binary
  -> backend-adapters
  -> backend-sandbox
  -> backend-auth
  -> backend-domain
  -> backend-db
  -> backend-core
  -> api-schemas / events / sandbox-proto
```

Rules:

- Lower-level crates must not depend on `backend`.
- Domain crates must not depend on Axum unless they own route handlers.
- DAO/model crates may depend on `sqlx`, `chrono`, `serde`, and
  `moatless-api-schemas`, but not on `AppState`.
- Route handlers that need `AppState` can stay in `backend` until the domain
  has a smaller state interface.
- Infrastructure crates (`backend-sandbox`, adapter providers, S3, Redis) should
  be extracted after low-infrastructure domains prove the pattern.

## Crate Naming Conventions

Cargo's crate namespace is flat and global (every crate name lands in
`Cargo.lock`). Large Rust workspaces handle this with a **layer + grouped
prefix** scheme, and this split follows the same pattern.

Rules:

- **Name each crate for its layer, prefixed by the stack it belongs to.** The
  backend family is `backend-core` -> `backend-db` -> `backend-domain` ->
  `backend-auth` -> `backend-sandbox` -> `backend-adapters`. The prefix encodes
  which stack's floor this is and reads as an architecture in `cargo tree`.
- **Avoid unqualified `core` / `common` / `utils` crate names.** They have no
  boundary and become dumping grounds. A qualified name (`backend-core`) states
  the fence: not backend -> does not belong. This mirrors rust-analyzer, whose
  foundation crates are `base-db` / `ide-db`, never a bare `core`, and whose
  layers group under `hir-*` and `ide-*` prefixes.
- **Reserve `moatless-core` for genuine product-wide primitives** shared across
  backend *and* sandbox *and* CLI (shared IDs, error envelopes, common enums) —
  only if such a layer actually materializes. Backend-only policy is not it.
- **Truly cross-cutting contracts keep their existing what-it-is names**
  (`moatless-api-schemas`, `moatless-events`, `moatless-sandbox-proto`). These
  sit *below* `backend-core`, so `backend-core` depending on them is correct,
  not an inversion.
- **Prefer folder name == crate name.** Current crates diverge
  (`crates/backend-core` -> `moatless-backend-core`), matching existing house
  style (`crates/api-schemas` -> `moatless-api-schemas`). This is accepted for
  now; a future cleanup phase may align them, or drop the `moatless-` prefix
  from these unpublished internal crates.

Reference: rust-analyzer's `hir-*` / `ide-*` / `base-db` grouping is the
canonical large-workspace example of this convention.

## Extraction Order

### Phase 0: Measure and Draw Boundaries

Create a baseline before moving files:

```bash
cargo check -p moatless-vibe-backend
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend
```

Record:

- current backend compile time after a clean `cargo clean -p moatless-vibe-backend`
- module import edges from `use crate::...`
- tests that rely on `moatless_vibe_backend::*` re-exports

Suggested helper commands:

```bash
rg "use crate::" backend/src --glob '*.rs'
rg "moatless_vibe_backend::" backend/tests crates/integration-tests --glob '*.rs'
cargo tree -p moatless-vibe-backend --edges normal
```

### Phase 1: Extract Backend Core

Create `crates/backend-core` for low-level types that are widely referenced and
do not need Axum server composition:

- `scope`
- pure access-control policy/types extracted from `access_control`
- shared error response helpers that are not domain-specific
- common ID/team/user context types if they are not already in
  `moatless-api-schemas`

Do not move `backend/src/access_control.rs` wholesale in this phase. It
currently depends on `AppState`, `AuthUser`, auth/user/team models, and
database-backed user/team lookups. Moving the whole file would make
`backend-core` depend on high-level backend composition. Keep those helpers in
`backend` until auth/team persistence has narrow traits, or split only the pure
policy/type portions into `backend-core`.

Expected edits:

- Add the new crate to workspace `Cargo.toml`.
- Move source files with `git mv`.
- Replace `crate::scope` imports and any extracted pure access-control imports
  with `moatless_backend_core::...`.
- Keep compatibility re-exports in `backend/src/lib.rs` for one phase if this
  keeps tests small.

Validation:

```bash
cargo clippy -p moatless-backend-core -- -D warnings
cargo test -p moatless-backend-core
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend
```

### Phase 2: Extract a Small Domain Crate

Use a low-coupling domain to prove the pattern. Good candidates from the current
module graph:

- `feedback`
- `tag`
- `command`
- `task_template`

Recommended first candidate: `tag` or `feedback`. They have the standard
`dao.rs` / `error.rs` / `handlers.rs` / `models.rs` / `routes.rs` / `schemas.rs`
shape and limited cross-module usage.

Decision point:

- If moving routes creates too much `AppState` coupling, move only
  `models`, `schemas`, `dao`, and domain errors first.
- Leave `routes` and `handlers` in `backend` until they can depend on a narrow
  domain state trait instead of full `AppState`.

Validation:

```bash
cargo clippy -p moatless-backend-domain -- -D warnings
cargo test -p moatless-backend-domain
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend
```

### Phase 3: Extract Shared Persistence Shape

After one domain crate is working, factor repeated persistence patterns:

- `DbPool` alias and pool construction boundaries
- common SQLx error mapping helpers
- reusable repository trait conventions
- test database helpers, if they are not backend-binary-specific

Be conservative here. Do not force all DAOs into one generic abstraction. Most
repository traits should stay domain-owned.

### Phase 4: Extract Larger Domain Groups

Move related domains in dependency order:

1. `repository`, `plugin_repo`, `skill`, `loops`
2. `task`, `task_event`, `task_turn`, `message`
3. `auth`, `credentials`, `provider_settings`, `team`, `secret`

These are more coupled than the Phase 2 candidates. Expect to introduce narrow
interfaces before moving handlers or services:

- `TaskLookup`
- `MessageStore`
- `CredentialStore`
- `RepositoryConfigStore`
- `SandboxNotifier`

Keep concrete `Pg*Repository` types in their owning domain crate unless they
need backend-only infrastructure.

### Phase 5: Extract Infrastructure Crates

Move infrastructure after domain boundaries are stable:

- `sandbox`: K8s/Docker provider, gRPC client/server bridge, sandbox JWT,
  preflight, secrets resolution
- `adapters` and `bindings`: Slack/GitHub/Linear/Telegram adapter logic
- `sse`, `event_handler`, `triggers`: event delivery and background consumers

These modules currently have the highest coupling to `state`, auth,
repositories, and external dependencies. They should not be first.

## AppState Strategy

Treat `backend/src/state.rs` as the composition root during the split.

Short term:

- Keep `AppState` in `backend`.
- Let extracted crates define traits and concrete services.
- Wire those traits in `AppState::new`.
- Re-export moved modules temporarily only when it reduces test churn.

Medium term:

- Replace full `AppState` use in moved handlers with domain-specific state
  structs, for example:

```rust
pub struct TagState {
    pub tags: Arc<dyn TagRepository>,
}
```

Long term:

- Route registration can remain in `backend` if that keeps OpenAPI assembly
  simpler.
- Domain crates can expose `router(state)` only when the state type is narrow
  and does not pull the full backend dependency graph with it.

## Testing Strategy

For every phase:

- Run the new crate's clippy and tests.
- Run backend clippy and tests.
- Run targeted integration tests if the moved code touches public API behavior.

Minimum required checks after backend Rust changes:

```bash
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend
```

Additional checks by phase:

- DAO/model moves: run affected `backend/tests/*_dao_test.rs`.
- Handler/route moves: run affected `backend/tests/*_handler_test.rs`.
- Task/message/auth moves: run `task_crud`, `messages`, and auth-related
  integration tests via the compose runner when practical.
- Sandbox/adapters moves: run the relevant sandbox or adapter integration tier.

## Risks

| Risk | Mitigation |
|------|------------|
| Crate cycles between domain modules | Move lower-level traits first; keep handlers in `backend` until cycles are gone. |
| `AppState` drags the full backend into extracted crates | Use narrow state structs or traits owned by the domain crate. |
| OpenAPI derives require backend-only types | Keep route/OpenAPI assembly in `backend` until schemas are fully in shared crates. |
| Test churn from changed public paths | Use temporary compatibility re-exports, then remove them in a cleanup phase. |
| Compile time gets worse due to over-splitting | Measure clean and incremental compile time after each phase. |
| Infrastructure crates become dumping grounds | Split infrastructure only after domain crate boundaries are proven. |
| Unqualified `core`/`common` crate becomes a dumping ground | Name crates by layer with a stack prefix (`backend-*`); reserve unqualified `core` for real cross-stack primitives. |
| Folder and crate names drift apart | Keep the folder name aligned with the crate name where practical; batch any renames into a dedicated cleanup phase. |

## Acceptance Criteria

- At least one extracted backend crate exists under `crates/`.
- `moatless-vibe-backend` depends on the extracted crate instead of local module
  paths.
- No runtime behavior changes are included in the split PR.
- Backend clippy and tests pass.
- The plan is updated with actual compile-time observations and any boundary
  decisions learned during Phase 0/1.

## Recommended First PR

Start with a small, reviewable PR:

1. Add `crates/backend-core`.
2. Move `scope` plus any extracted pure access-control policy/types; keep
   database-backed `access_control` helpers in `backend`.
3. Update imports in backend modules.
4. Keep temporary re-exports in `backend/src/lib.rs`.
5. Run:

```bash
cargo clippy -p moatless-backend-core -- -D warnings
cargo test -p moatless-backend-core
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend
```

Only after that lands, use `tag` or `feedback` as the first domain extraction.
