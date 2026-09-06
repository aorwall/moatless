# moatless-sandbox-k8s

The Kubernetes runtime provider behind the `moatless-sandbox-control` ports. It
implements `SandboxRuntime`, `FleetObserver`, and the T3 read seams.

Every provider honors the contract in
[`crates/sandbox-runtime/AGENTS.md`](../sandbox-runtime/AGENTS.md). Read it
first. This guide adds only what the Kubernetes runtime does on its own.

## What this crate may name

- Name `kube` and `k8s-openapi` freely. Those SDKs are legal in a provider band,
  and the control band above depends only on the ports.
- Never name `moatless-sandbox-docker`. It is a sibling in the same band.
- Never name a concrete `AppState`. The backend composition root builds
  `K8sSandboxAdapter` and wires it into `AppState`.

## What the Kubernetes runtime adds

- Report a lingering PVC as `RuntimeError::NotAbsent`, together with the other
  lingering resources.
- Read Kubernetes status direct from Deployments and Pods. Do not reintroduce
  Redis status caching.
- The pod watcher tracks a pod by `pod_name` as well as by `task_id`, so a
  superseded generation stays distinguishable from a replacement crash.
- `naming.rs` derives every Kubernetes name and label from `task_id` and keeps
  the 63-character Kubernetes cap.
- `ResourceConfig` is also the storage config. Merge a per-repo override key by
  key onto the `K8S_RESOURCE_*` defaults. When you replace the whole block, a
  storage-only or memory-only override leaves the container BestEffort.

## What each Kubernetes label marks

The writers (`deployment.rs`, `pod_builder.rs`, `service.rs`, `pvc.rs`) hold
every label as a literal beside the resource it stamps. Two of them are lookup
contracts, not decoration:

- **`moatless.io/task-id`** is how almost every read finds a resource:
  `pod_reader`, `pvc`, `ingress`, `service`, `edge` and `deployment` all select
  on it. A resource created without it is invisible to cleanup and to status.
- **`app.kubernetes.io/managed-by=moatless`** is what `pod_watcher` watches. A
  pod without it never reaches the watcher.

`moatless_sandbox_runtime::runtime::OPERATION_GENERATION_LABEL` is the one label
this crate does not own. It carries lifecycle's operation identity, and
`fleet_observer` correlates watcher events back through it.
