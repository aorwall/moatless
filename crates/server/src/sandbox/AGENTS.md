# Backend sandbox facade

`crates/server/src/sandbox/mod.rs` states what the facade is. Add no sandbox
logic to it. Put it in one of the crates it re-exports, behind that crate's
band.

## Where the code lives

Start in `moatless-sandbox-control` for any change to sandbox intent, events, or
the reconciler. Each crate's own `AGENTS.md` holds its rules.

Keep every re-export explicit and named. Never write a glob: the crate census
resolves an edge through the facade to its definition site only when the name
is written out. `k8s` and `docker` are crate aliases, declared as
`pub use moatless_sandbox_<x> as <x>;`.

## What the backend still owns

- **The composition root.** Named modules under `compose/` build the
  concrete provider adapters (`K8sSandboxAdapter`, `DockerSandboxAdapter`) and
  the `Lifecycle` and `Reconciler` handles, then wire them into `AppState`. The
  four crates are generic over the ports. Only the backend names the concrete
  wiring.
- **The unified task reaper**, in `crates/server/src/reaper.rs`. Task-scoped stop
  and remove still route through `Lifecycle` in `moatless-sandbox-control`.
