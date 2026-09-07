# moatless-sandbox-k8s

Every provider honors the contract in
[`crates/sandbox-runtime/AGENTS.md`](../sandbox-runtime/AGENTS.md). Read it
first. `src/lib.rs` states what this crate may name.

## What the Kubernetes runtime adds

- A lingering PVC is one of the resources reported as `RuntimeError::NotAbsent`.
- Read Kubernetes status direct from Deployments and Pods. Do not reintroduce
  Redis status caching.
- The pod watcher tracks a pod by `pod_name` as well as by `task_id`, so a
  superseded generation stays distinguishable from a replacement crash.
- `naming.rs` keeps the 63-character Kubernetes cap.
- `ResourceConfig` is also the storage config, and the per-repo override merges
  onto the `K8S_RESOURCE_*` defaults. Replace the whole block and a storage-only
  or memory-only override leaves the container BestEffort.

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
