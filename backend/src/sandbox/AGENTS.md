# Backend Sandbox Module

Rust/Axum backend module that provisions, observes, and removes Kubernetes and
Docker sandboxes. It runs inside `moatless-vibe-backend`.

## Ownership Map

- HTTP routes in `routes.rs` call handlers; handlers do not own provider logic.
- `lifecycle/` owns durable sandbox intent and all start/stop/restart/redeploy/remove orchestration.
- `SandboxRuntime` is the provider mutation port. `k8s/provider.rs` and `docker/provider.rs` implement it natively and return `RuntimeOutcome`; provider adapters do not publish lifecycle events.
- The read-only seams are split by audience. The T3 contract ports (`SandboxObserver`, `SandboxConnector`, `SandboxIntentReader`, `SandboxStarter`, `SandboxStopper`) are declared once in `moatless-backend-core` (`sandbox` module) so `backend` and `crates/t3code` share one vocabulary; the provider adapters implement them and narrow backend types to the contract through `t3code::projection`. `status_reader.rs`'s `SandboxStatusReader` is the backend-local rich read (the 13-field `SandboxStatusFullResponse`, live status, and log streams) the HTTP routes need. `access.rs` holds only the `LogOptions` request struct shared by both.
- File handlers get a ready `SandboxClient` through `SandboxConnector`, then call file/search methods directly.
- The unified task reaper lives in `backend/src/task/reaper.rs`; task-scoped stop/remove still routes through lifecycle.

## Load-Bearing Rules

- `Lifecycle` is the sole writer of Sandbox intent. Do not add direct provider mutation paths from handlers, adapters, reapers, or watchers.
- Durable Sandbox state lives in the `sandboxes` table. No row means the Sandbox was never requested, is converged Stopped, and is wakeable.
- `delete_task` requires successful `lifecycle.remove` before deleting the task row.
- Completion facts commit before publication. `grpc_server.rs::publish_event` returns success only after the complete payload is durably admitted.
- `event_dedup.rs` records admission, not receipt. Failed admission must leave retries real, and concurrent duplicates should retry rather than be acknowledged.
- Operation-generation markers guard replacement operations. Marker clear must compare the acknowledged generation so late completions cannot erase newer operations.
- The reconciler is the desired-vs-actual recovery owner. It handles woken-ready pending messages, removed-intent orphans, abandoned edges, and matching replacement generations.
- Native provider operations must prove observed absence before strict success and surface lingering resources as `RuntimeError::NotAbsent`.
- Server containers are data-driven from `ServerConfig`; do not hardcode server names or ports outside explicit config parsing.
- Kubernetes status reads direct from Deployments and Pods. Do not reintroduce Redis status caching.
- The pod watcher tracks pods by `pod_name`, not only `task_id`, so superseded generations are distinguishable from replacement crashes.
- Keep generated Kubernetes/Docker names and labels deterministic from `task_id`; preserve the 63-character Kubernetes cap in `k8s/naming.rs`.
- `classify_grpc_error()` must retain the underlying `tonic::Status` as the error source while mapping client-visible status codes.
- Provision-plan construction and expected-image selection stay behind lifecycle and concrete runtime adapters.

## K8s Labels

- `moatless.io/task-id` identifies task-owned resources.
- `moatless.io/operation-generation` correlates runtime resources with lifecycle operations.
- `moatless.io/user-id` labels user secrets.
- `app.kubernetes.io/managed-by=moatless` identifies managed resources.
- `app.kubernetes.io/name=moatless-workspace`
- `app.kubernetes.io/component` is one of `sandbox`, `storage`, or `sandbox-service`.
