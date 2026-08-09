# moatless-sandbox-control (band 6)

The provider-agnostic sandbox band: durable lifecycle intent and orchestration,
the gRPC ingress service, event dedup, HTTP routes/handlers, JWT, secrets,
preflight, status reads, and the resource-quantity parsers. The Kubernetes and
Docker runtimes (`moatless-sandbox-k8s`, `moatless-sandbox-docker`, band 7)
depend downward on this crate and implement its ports; this crate names neither
provider SDK (`kube`/`bollard`) — its error type boxes any provider error behind
`Box<dyn Error + Send + Sync>` so nothing here carries an SDK type.

## Ownership Map

- HTTP routes in `routes.rs` call handlers; handlers do not own provider logic.
- `lifecycle/` owns durable sandbox intent and all
  start/stop/restart/redeploy/remove orchestration.
- `SandboxRuntime` (`lifecycle/runtime.rs`) is the provider mutation port. The
  provider crates implement it natively and return `RuntimeOutcome`; provider
  adapters do not publish lifecycle events.
- The read-only seams are split by audience. The T3 contract ports
  (`SandboxObserver`, `SandboxConnector`, `SandboxIntentReader`,
  `SandboxStarter`, `SandboxStopper`) are declared once in
  `moatless-backend-core` (`sandbox` module) so `backend` and `crates/t3code`
  share one vocabulary; the provider adapters implement them and narrow backend
  types to the contract through the backend's `t3code::projection`.
  `status_reader.rs`'s `SandboxStatusReader` is the rich read (the 13-field
  `SandboxStatusFullResponse`, live status, and log streams) the HTTP routes
  need. `access.rs` holds only the `LogOptions` request struct shared by both.
- File handlers get a ready `SandboxClient` through `SandboxConnector`, then call
  file/search methods directly.
- The unified task reaper lives in `moatless-tasks`
  (`crates/tasks/src/task/reaper.rs`); task-scoped stop/remove still routes
  through `Lifecycle` here.

## Load-Bearing Rules

- `Lifecycle` is the sole writer of Sandbox intent. Do not add direct provider
  mutation paths from handlers, adapters, reapers, or watchers.
- Durable Sandbox state lives in the `sandboxes` table. No row means the Sandbox
  was never requested, is converged Stopped, and is wakeable.
- `delete_task` requires successful `lifecycle.remove` before deleting the task
  row (enforced in `moatless-tasks`).
- Completion facts commit before publication. `grpc_server.rs::publish_event`
  returns success only after the complete payload is durably admitted.
- `event_dedup.rs` records admission, not receipt. Failed admission must leave
  retries real, and concurrent duplicates should retry rather than be
  acknowledged.
- Operation-generation markers guard replacement operations. Marker clear must
  compare the acknowledged generation so late completions cannot erase newer
  operations.
- The reconciler is the desired-vs-actual recovery owner. It handles
  woken-ready pending messages, removed-intent orphans, abandoned edges, and
  matching replacement generations.
- `classify_grpc_error()` must retain the underlying `tonic::Status` as the
  error source while mapping client-visible status codes.
- Provision-plan construction and expected-image selection stay behind lifecycle
  and the concrete runtime adapters.
- Provision-plan construction must reject tasks whose sandbox identity cannot
  control the task before any runtime resource is created. The check belongs in
  this provider-agnostic band and must continue to match the gRPC
  `ScopeContext::can_control` gate.

## The provider contract (honored by band 7)

These are the parts of the port contract every provider must satisfy; the
runtime realizations live in the provider crates' own guides:

- A native provider operation must prove observed absence before reporting
  strict success, and surface lingering resources as `RuntimeError::NotAbsent`.
- Server containers are data-driven from `ServerConfig`; the port never
  hardcodes server names or ports outside explicit config parsing.
- Generated runtime names and labels stay deterministic from `task_id`.
