# Backend sandbox facade

`crates/server/src/sandbox/mod.rs` is a permanent facade. It re-exports four
banded crates, so a higher band still inside `moatless-backend` keeps naming
`crate::sandbox::…`. Add no sandbox logic to this file. Put it in one of the
crates below, behind the band that crate lives in.

`grpc_server` is the exception. It is a module of `moatless-backend`, and it
reaches back down into `event_dedup`, `jwt`, `metrics`, `secrets` and `storage`
through this facade.

## Where the code lives

The facade re-exports `moatless-sandbox-runtime`, `-control`, `-k8s` and
`-docker`. Each crate's own `AGENTS.md` holds its rules.

Start in `moatless-sandbox-control` for any change to sandbox intent, events,
or the reconciler. It holds the provider-agnostic half:

- durable lifecycle intent and orchestration;
- the gRPC ingress service and event dedup;
- routes and handlers;
- JWT, secrets and preflight;
- status reads and the resource-quantity parsers.

[BANDS](../../../../tooling/arch-rules/src/band_order.rs) owns the dependency
order between the four.

Keep every re-export explicit and named. Never write a glob: the crate census
resolves an edge through the facade to its definition site only when the name
is written out. `k8s` and `docker` are crate aliases, declared as
`pub use moatless_sandbox_<x> as <x>;`.

## What the backend still owns

- **The composition root.** `startup.rs`, `compose.rs` and `state.rs` build the
  concrete provider adapters (`K8sSandboxAdapter`, `DockerSandboxAdapter`) and
  the `Lifecycle` and `Reconciler` handles, then wire them into `AppState`. The
  four crates are generic over the ports. Only the backend names the concrete
  wiring.
- **The unified task reaper**, in `crates/server/src/reaper.rs`. Task-scoped stop
  and remove still route through `Lifecycle` in `moatless-sandbox-control`.
