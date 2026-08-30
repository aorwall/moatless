# moatless-sandbox-runtime (band 5)

What a sandbox runtime implements, and the machinery every runtime reuses.

Two things live here, and the split between them is the whole point of the
crate. The **contract** is what a container runtime must satisfy:
`runtime::SandboxRuntime` for mutation, `fleet::FleetObserver` for observation,
and the values both sides speak in — `RuntimeOutcome`, `OperationGeneration`,
`SandboxObservedStatus`, `ProvisionRequest`, `RepoConfig`, `ServerStatus`. The
**machinery** is everything a runtime needs in order to answer: the gRPC client
and its error classification, the status reads, the storage plan, the response
schemas, the resource parsers, and the ingress counters.

## The rule

**The orchestrator that drives a runtime must stay unreachable from here.**
`Lifecycle`, the reconciler, the watcher, the durable intent behind them, and
the HTTP surface over them live in `moatless-sandbox-control`, a band above.
Nothing in this crate may name that crate — Cargo enforces it, since the
dependency runs the other way, and the census scores the edge.

A concrete consequence: `error::SandboxError::Lifecycle` boxes its source rather
than naming `LifecycleError`. The conversion from a lifecycle failure to a
response error lives beside `LifecycleError`, in the band that raises it.

## Ownership

- `runtime.rs` owns the mutation seam and the error a mutation fails with:
  `RuntimeError` and the `ErrorClass` that drives the reconciler's retry policy.
  A failure diagnostic belongs on `RuntimeError`, never on the success
  `RuntimeOutcome`.
- `fleet.rs` owns the observation seam. One batch read per tick
  (`observe_fleet`), plus the authoritative per-Task read (`observe`) the
  reconciler confirms a divergence with. `status_reader::confirm` is the shared
  live-status helper both provider implementations delegate to.
- `convergence`, `classify`, `marker` and `types` are the decision vocabulary a
  runtime and the orchestrator both read. They decide nothing on their own —
  the reconciler and the watcher that call them stay above.
- `client.rs` re-exports `SandboxConnector` and `classify_grpc_error` from
  `sandbox-proto`, beside the `SandboxClient` they hand out.

## What a runtime must honor

- A native provider operation must prove observed absence before reporting
  strict success, and surface lingering resources as `RuntimeError::NotAbsent`.
- Server containers are data-driven from `ServerConfig`; the contract never
  hardcodes server names or ports outside explicit config parsing.
- Generated runtime names and labels stay deterministic from `task_id`.
- `classify_grpc_error` must retain the underlying `tonic::Status` as the error
  source while mapping client-visible status codes.
