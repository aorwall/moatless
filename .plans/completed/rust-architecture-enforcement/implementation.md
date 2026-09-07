# Rust architecture enforcement

The approved scope is 88 required findings across 28 crate audits. The six
recorded exceptions and one optional finding remain outside implementation.
`status.json` tracks each finding. Main revision
`e91747e78a216191ef71a90d4bdded05d32446c3` is merged into this branch.

## File contracts

The Rust skill's `references/file-roles.md` owns the rules. Files are optional,
but their roles are fixed: `lib.rs` exports a crate, namespace roots declare
children, `main.rs` enters and exits the process, `app.rs` owns lifecycle, and
named composition modules construct dependencies. Routes register HTTP;
handlers adapt transport; operations own policy and side effects; persistence
modules own SQL and transaction boundaries.

The checker enforces declaration-only roots, route registration, dependency
direction toward transport, SQL placement, process exit and composition.
Its source inventory includes custom Cargo targets and the excluded Codex SDK.
Fixtures cover cfg negation, mixed production/test alternatives, inline
namespaces, re-export facades and indirect references. Handler policy still
requires a responsibility review; syntax alone cannot establish its owner.

## Crate evidence

Paths below are relative to `crates/`, except architecture tooling.

| Crate | Implemented ownership |
|---|---|
| adapter-github-pr | `operations` owns mutations and PR queries; HTTP and T3 call that owner. Delivery implementation has named children. |
| adapter-linear | Operations own issue mutations, task attachment and subscription reactions. Installation owns Linear OAuth and stable connection writes. |
| adapter-model | Adapter binding and outbound contracts live here; connection persistence and metric recording have named owners. |
| adapter-slack | Thread-history presentation belongs to the delivery operation; the API client performs provider requests. |
| adapter-telegram | Pairing persistence and approval are separate owners; assembly supplies the required capabilities. |
| adapter-webhook | Ingress owns subscription policy and durable admission; request contracts come from api-schemas. |
| api-client | Library declarations expose the client implementation and generated contracts. |
| api-schemas | Canonical sandbox and webhook wire contracts preserve served fields and request defaults. |
| claude-agent-sdk | Library and transport namespaces expose named implementation owners while preserving the SDK surface. |
| cli | Entry, application dispatch and command operations have separate owners; namespace roots declare children. |
| codex-sdk | Named runtime and subprocess owners preserve startup deadlines, maximum-turn admission and bounded shutdown. |
| core | `db/access_control` owns SQL predicates; change-feed position policy is separate from DAO queries. Upload errors belong to the backend. Metrics state is private. |
| identity | Authentication assembly, state, account operations, password operations, OAuth login and DAOs are separate. Team operations enforce policy. Adapter-app services validate and seal before persistence. |
| loops | Binding and Loop operations own mutation, delivery and reaction policy. SQL has DAO owners. Loop spawn configuration belongs to Loop models. |
| ports | Capability declarations and supporting contracts have named modules with explicit re-exports. |
| s3 | Object and presigning operations have named owners and causal typed errors; transport redaction belongs to callers. |
| sandbox | Application lifecycle and construction are separate. Shared workspace sync serves initialization, Turn completion, periodic work, pre-stop and shutdown. Task context, terminal and agent runtime have explicit owners. |
| sandbox-control | Server configuration operations enforce access and restart policy; routes register and health handlers adapt transport. Lifecycle locking uses its database owner. |
| sandbox-docker | Named driver operations and construction modules replace mixed roots. |
| sandbox-k8s | Driver roots declare and re-export named implementation modules. |
| sandbox-proto | Protocol roots declare generated contracts; handwritten client behavior has a named owner. |
| sandbox-runtime | Re-exports canonical API contracts; runtime-owned free functions convert protobuf values. Provisioning names domain values. |
| server | `compose/` assembles concrete dependencies; `app` supervises lifecycle. Adapter commands, repository sync, task-connection enrichment, upload reads and access administration have operation owners. Usage SQL belongs to its DAO. |
| t3code | Framing, sessions, dispatch, authentication, subscriptions and projection dependencies have named owners. Script writes delegate to workspaces; read-only projections retain their recorded exception. |
| tasks | Recovery policy belongs to `task_turn/recovery`; fork, outbox, relation and external-event SQL have persistence owners. Message summaries are shared independently of HTTP. |
| types | Pure shared values remain here; PostgreSQL predicates, provider-token SQL, Adapter contracts and Loop-only configuration move to their owners. Enum generation has a private module. |
| workspaces | Repository operations own reusable provider work. Script replacement enforces access and preserves hidden environment values. Workspace placement replacement uses one locked transaction. |
| tooling/arch-rules | Six structural checks, shared source classification and target inventory; declaration-only library root and named runner/report owners. |

## Preserved and strengthened behavior

- Password login keeps unknown-email verification, throttle ordering and
  best-effort failure recording. Password changes require cookie authentication
  and the current password; changes and resets revoke sessions.
- Account creation commits user, provider identity and password together.
  A competing request now receives the intended 409 instead of a unique-index 500.
- Workspace placement replacement locks the workspace and commits placement,
  primary selection and derived kind together. Failure rolls back the operation.
- Sandbox initialization still reports permanent failure as exit 78 and transient
  failure as exit 1. The executable owns exit; the application returns typed failure.
- Workspace synchronization retains shared state across every lifecycle caller.
- HTTP, gRPC and SDK contracts retain their supported entry points. Internal Rust
  callers use the new owners, with explicit compatibility re-exports where needed.

The excluded Codex SDK still requires the external checkout named in its
manifest. Its lockfile selects upstream-compatible `rmcp` and `rmcp-macros`
1.7.0, because the locked 1.8.0 packages were unavailable during verification.
The SDK passes 32 unit tests and one doctest against that checkout. Runtime
shutdown is bounded to five seconds; uncooperative upstream blocking work
cannot be forcibly interrupted by Tokio.

## Architecture baseline

The baseline shrinks from 153 to 111 entries. None of the six new rules has an
entry. `baseline-evidence.json` records fixed identities and one-to-one
relocations of existing debt. The remaining 111 entries belong to older rules;
this change does not claim that all pre-existing architecture debt is resolved.

## Verification

`status.json` records the per-crate results and limitations:

- 3,615 default owning-crate tests; 33 standalone Codex SDK tests.
- 68 architecture fixtures and 24 workspace checks.
- 509 backend database tests, plus three workspace transaction regressions.
  The throughput benchmark passes in isolation; its first run competed with
  two full database suites and failed its relative-throughput threshold.
- Required workspace and feature-specific Clippy configurations pass.
- OpenAPI regeneration changes six documentation strings, with identical routes
  and wire schemas. Orval regeneration, TypeScript type-check and Biome pass.

Live external SDK/provider E2E tests remain credential-gated. No deployment or
production data was required for verification.
