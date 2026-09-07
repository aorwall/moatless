# moatless-sandbox-runtime

The crate doc in `src/lib.rs` states what the contract is and what the
machinery holds, and each module's `//!` doc states what it owns.

`error::SandboxError::Lifecycle` boxes its source instead of naming
`LifecycleError`. The conversion from a lifecycle failure to a response error
lives beside `LifecycleError`, in the band that raises it.

Put a failure diagnostic on `RuntimeError`. Never put one on the success value
`RuntimeOutcome`. Both provider implementations delegate live status to
`status_reader::confirm`, and the reconciler and watcher that read the decision
vocabulary (`convergence`, `classify`, `marker`, `operation`) stay above it.

## What every runtime provider must honor

- Prove observed absence before you report strict success. Report a lingering
  resource as `RuntimeError::NotAbsent`.
- Return `RuntimeOutcome` from a native mutation. Do not publish a lifecycle
  event. `Lifecycle` in `moatless-sandbox-control` publishes them.
- Build server containers from `ServerConfig`. Do not hardcode a server name or
  a port outside explicit config parsing.
- Derive every generated name and label deterministically from `task_id`.
- Give every sandbox container and every server container CPU and memory
  requests and limits from `ResourceConfig`. Merge a per-repo override key by
  key onto the defaults. Do not replace the block.
- Keep the underlying `tonic::Status` as the error source in
  `classify_grpc_error` while you map client-visible status codes.
- Keep provision-plan construction and expected-image selection behind
  `Lifecycle` and the provider adapters.
- Name this crate and the domain floor `moatless-types`. Never name the wire
  contract `moatless-api-schemas`. A provisioning value a driver reads is domain
  data and lives in `moatless_types::workspace`: `RepoCheckout` and
  `WORKSPACE_REPOS_ENV` are the current examples. `crate_deps.rs` fails the
  api-schemas edge.
