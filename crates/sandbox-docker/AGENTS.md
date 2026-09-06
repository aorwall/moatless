# moatless-sandbox-docker

Every provider honors the contract in
[`crates/sandbox-runtime/AGENTS.md`](../sandbox-runtime/AGENTS.md). Read it
first. This guide adds only what the Docker runtime does on its own.

## What this crate may name

- Name `bollard` freely. The SDK is legal in a provider band, and the control
  band above depends only on the ports.
- Never name `moatless-sandbox-k8s`. It is a sibling in the same band.
- Never name a concrete `AppState`. The backend composition root builds
  `DockerSandboxAdapter` and wires it into `AppState`.

## What the Docker runtime adds

- `naming.rs` derives container names, network names and labels from `task_id`,
  so a caller addresses a container or a network without an extra lookup.
- The Docker connector reports a gRPC failure through `classify_grpc_error`. Do
  not stringify the failure.
