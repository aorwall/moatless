# Backend Sandbox Facade

`crates/server/src/sandbox/mod.rs` is a **permanent facade**, not a module of code. The
sandbox subsystem was carved into three banded crates; this file only re-exports
them so higher bands still inside `moatless-backend` keep naming
`crate::sandbox::…`.

## Where the code lives

- **`moatless-sandbox-control`** (`crates/sandbox-control`, band 6) — the
  provider-agnostic half: durable lifecycle intent and orchestration, the gRPC
  ingress service, event dedup, routes/handlers, JWT, secrets, preflight,
  status reads, and the resource-quantity parsers. Its guide holds the
  load-bearing lifecycle invariants. **Start there for any change to sandbox
  intent, events, or the reconciler.**
- **`moatless-sandbox-k8s`** (`crates/sandbox-k8s`, band 7) — the Kubernetes
  runtime provider. Its guide holds the K8s naming/label/status-read rules.
- **`moatless-sandbox-docker`** (`crates/sandbox-docker`, band 7) — the Docker
  Engine runtime provider. Its guide holds the container-provider rules.

The re-exports are explicit and named (never a glob) so the crate census
resolves edges through the facade to definition sites. `k8s` and `docker` are
crate aliases (`pub use moatless_sandbox_<x> as <x>;`).

## What the backend still owns

- **Composition root.** `startup.rs`/`compose.rs`/`state.rs` build the concrete
  provider adapters (`K8sSandboxAdapter`, `DockerSandboxAdapter`) and the
  `Lifecycle`/`Reconciler` handles, and wire them into `AppState`. The crates
  are generic over the ports; only the backend names the concrete wiring.
- The unified task reaper lives in `moatless-tasks`
  (`crates/tasks/src/task/reaper.rs`); task-scoped stop/remove still routes
  through `Lifecycle` in the control crate.

Do not add sandbox logic to this file — it belongs in one of the three crates
above, behind the band it lives in.
