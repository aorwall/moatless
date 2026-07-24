# sandbox module

Rust (Axum) service managing the full lifecycle of Kubernetes- and Docker-based sandbox environments. Each sandbox is a multi-container pod/container identified by `task_id`, provisioned on demand. Runs as part of the `moatless-vibe-backend` binary.

## Architecture

### Lifecycle module (`lifecycle/`)
Deep module owning durable lifecycle intent and orchestration per Task, per
`specs/sandbox/04-lifecycle-architecture.md`. **Migration status: step 3 runtime
seams landed; step 4 shallow-path deletion remains.** The typed surface
(`LifecycleOutcome` with independent
intent/runtime effect axes, `ArchiveRetention`, classifiable
`LifecycleError`/`RuntimeError`, `OperationGeneration`/`CorrelationMarker`), the
`SandboxRuntime` seam (`RuntimeOperationContext` + `RuntimeOutcome`), the
`Lifecycle` orchestrator over ports (`LifecycleStore`, `AgentStopPort`,
`LifecycleEventSink`, `LifecycleLock`, `Clock`), and the load-bearing decision
logic as pure, unit-tested functions: `convergence` (reconciler desired/observed
map), `classify` (watcher four-case classification), `marker` (per-class
acknowledgement), `wake` (message-wake causal precedence), `events` (deterministic
`sandbox:{task}:{gen}:{fact}` ids + acceptance/completion/infrastructure
derivation), `attempt` (leader-local cap + backoff). Durability, event-dedup, and
strict-removal are load-bearing.

The `Lifecycle` handle is wired in production (`state::build_lifecycle`, shared by
`AppState`, the sandbox API, and the pod watcher) and is the sole writer of
Sandbox intent. `AppState` carries exactly one sandbox value — this handle, as
`state.sandbox`; runtime, observer, and connector handles are passed only to
their consumers during assembly. Every
start/stop/restart/redeploy/remove caller — task and
sandbox handlers, adapters, the reaper, message-send/notify, and the pod watcher's
crash remediation — routes through it, over the
`PgLifecycleStore`/`PgLifecycleLock` Postgres adapters, a native
`SandboxRuntime`, `GrpcAgentStop`, and the `OutboxEventSink`. Provision-plan
construction, expected-image resolution, strict runtime-resource absence, and
archive retention are behind lifecycle and the concrete runtime adapters.
`delete_task` requires a successful `lifecycle.remove` before
row deletion. Durable Sandbox state — desired state, stop reason, archive
retention, the correlation marker, the start/intent timestamps, and the per-Task
generation counter — lives in the **`sandboxes` table** (PK `task_id`, FK
`ON DELETE CASCADE`), born lazily through the intent upsert: no row means no
Sandbox was ever requested (converged Stopped, always wakeable). The row is
therefore the mechanism making lifecycle the sole intent writer by construction.
The status handlers and reaper read desired state through the store
(`Lifecycle::desired_state`); task responses use the lifecycle DAO's shared
Sandbox-intent projection, including the no-row Stopped-at-epoch semantic. The
transactional outbox is in place.

The pending-Message query/index and the desired-state reconciler are in place:
the reconciler is lease-elected on K8s / lease-free single-replica on Docker,
re-notifies a woken Ready sandbox of its undelivered pending messages, and
subsumes the `Removed`-intent orphan rule. `RuntimeOperationContext` reaches
every provider mutation; replacement pod templates and sandbox containers carry
`moatless.io/operation-generation`. K8s restart patches the Deployment template
under its existing `Recreate` strategy, while Docker restart strictly
inspect/remove/recreates the sandbox container because live labels are
immutable. Both watchers carry the observed generation through the four-case
classifier; Ready convergence uses the generation stamped on its runtime
resource. Docker recreates stopped containers for a new Start because labels
are immutable, filters subordinate server-container events, and defensively
rejects them before classification. The reconciler acknowledges matching
replacement generations and durably commits a deterministic `Error` fact
before clearing an abandoned edge. Suppression expires before abandonment,
and a matching replacement that is still starting keeps its marker. Every
marker clear compares the acknowledged generation, so a late completion cannot
erase a newer operation's marker. The reconciler is the only desired-vs-actual
recovery owner.
Runtime mutations use the native adapters directly through `SandboxRuntime`;
step 4 removes the remaining caller wrappers that already delegate to lifecycle.

### Runtime and access seams
`SandboxRuntime` is lifecycle's mutation seam: `ensure_running`, `stop`,
`restart`, `redeploy`, and `remove`. `K8sSandboxAdapter`
(`k8s/provider.rs`) and `DockerSandboxAdapter` (`docker/provider.rs`) implement
it natively and return `RuntimeOutcome`; adapters do not publish lifecycle
events. Successful storage/server degradations travel in outcome diagnostics
and lifecycle republishes them.

`access.rs` holds two narrow read/interior seams. `SandboxObserver` provides
live status and server logs. `SandboxConnector` is the
only task-addressed `SandboxClient` construction path, owning Kubernetes
service-name and Docker raw-URL addressing. The same concrete adapter
implements all three seams, but consumers receive only the capability they
need.
Startup constructs one shared `ProvisionDeps` repository bundle and gives it
to lifecycle plus the selected adapter, so plan construction and expected-image
reads share the same dependency graph.

### Request flow
```
HTTP → routes.rs (Axum OpenApiRouter)
     → handlers.rs      (lifecycle mutations + observer status)
     → file_handlers.rs (file ops: tree, read, write, list, search-by-name)
         → SandboxConnector.sandbox_client() → SandboxClient (gRPC direct)
     → Lifecycle → SandboxRuntime → k8s/* or docker/*
```
`SandboxApiState` contains only the observer, connector, lifecycle handle, and
task/team authorization repositories. File handlers ask the connector for a
ready client, then invoke file methods directly.
Both native adapters enforce observed absence before strict success and surface
lingering resources as `RuntimeError::NotAbsent`. `DeleteArchives` removes the
task's transcript and workspace prefixes through the configured S3 client in
both runtime modes, only after runtime absence succeeds.
Kubernetes resource modules share the bounded absence poller in `k8s/wait.rs`;
resource-specific code supplies only its authoritative lookup.

### K8s resources per sandbox
Provisioning creates (all labeled `moatless.io/task-id={task_id}`): a **Deployment** (enables stop = scale to 0 / start = scale to 1 without re-provisioning), its **Pod**, a ClusterIP **Service**, an optional **PVC**, and optional **IngressRoute** (Traefik CRD, one per server port, pattern `{task_id}--{port}.{proxy_domain}`). Deployment/Service names are deterministic: `moatless-ws-{sanitized_task_id}` (`k8s/naming.rs`, 63-char cap).

### Pod structure
- **`sandbox` container**: agent environment (image configurable per-repo), ports 8001 (HTTP) + gRPC, talks back to the backend via gRPC.
- **Server containers** (dynamic, data-driven from `ServerConfig`): each specifies its own `docker_image` (falls back to `sandbox_image`), `start_commands`, `env_vars`, port, capabilities. Name convention `{server.name}-server`; TCP readiness probe on the port.
- **Volumes**: `workspace-data` (PVC), `shared-data`/`build-cache` (emptyDir), `tmp` (memory-backed).

### Status resolution
`StatusResolver` (`k8s/status_resolver.rs`) reads status directly from the K8s API — no Redis cache: (1) Deployment replicas (0 = Stopped, missing = NotFound), (2) pod phase + sandbox container readiness, (3) server status by overlaying container probe states onto repo-config-defined servers (config-first, so servers show in the UI before pods exist).

### Pod watcher
`pod_watcher.rs` — leader-elected (K8s Lease) task that watches all sandbox pods, detects lifecycle transitions (OOM, crash loop, eviction, restart, ready, failed, deleted), classifies them against durable intent + correlation marker, and publishes only convergence/divergence events to Redis Streams. Tracks pods **by pod_name, not task_id**, so the superseded generation's deliberate stop during restart/redeploy is distinguishable from a replacement crash.

### Task reaper
The unified reaper lives in **`task/reaper.rs`, not this module**.
Leader-elected; each cycle (1) stops idle sandboxes after `ttl_minutes`, (2)
auto-closes Open tasks inactive past `inactive_ttl_days`, and (3) sweeps only
taskless labeled resources that lifecycle cannot express. Task-scoped
stop/remove goes through `state.sandbox`; its one interior read — the
idle-eligibility agent status — goes through the consumer-owned
`AgentStatusPort`, whose `ConnectorAgentStatus` implementation wraps
`SandboxConnector`.

### gRPC communication
`SandboxClient` (`client.rs`) calls sandbox pods: execute-agent, stop-agent, notify-messages, file ops, search-files. Lazy connection, 5s connect / 10s request timeout. `classify_grpc_error()` maps gRPC → `SandboxError`: Unavailable/DeadlineExceeded/Cancelled → NotReady/503, NotFound → 404, FailedPrecondition/InvalidArgument → 400, else → Internal/500. `notify.rs` is fire-and-forget message notification.

### Auto-provisioning
`auto_provision.rs` resolves a fresh provision plan for lifecycle
`ensure_running`/`redeploy` by reading task, workspace, repository, credentials,
secrets, plugins, and servers from the DB. Expected-image selection is exposed
through `Lifecycle`, not through an adapter.

### JWT auth
`jwt.rs` generates/verifies HS256 JWTs (90-day expiry, issuer `moatless-vibe-backend`) that sandbox pods use to authenticate back to the backend.

## K8s label convention
- `moatless.io/task-id` — task identifier (selector queries)
- `moatless.io/operation-generation` — lifecycle correlation identity on pod templates and Docker sandbox containers
- `moatless.io/user-id` — user identifier (on secrets)
- `app.kubernetes.io/managed-by=moatless` — identifies moatless-managed resources
- `app.kubernetes.io/name=moatless-workspace`
- `app.kubernetes.io/component` — `sandbox`, `storage`, `sandbox-service`

## Leader election
Three independent leader-elected subsystems use K8s Leases, all keyed on the `HOSTNAME` env var:
- `moatless-pod-watcher` — pod lifecycle watcher (15s default lease)
- `moatless-task-reaper` — unified reaper (impl in `task/reaper.rs`), 2× check-interval lease
- `moatless-sandbox-reconciler` — desired-state reconciler (impl in `sandbox/lifecycle/reconciler.rs`), 120s lease; Docker runs it lease-free (single replica)

## Design patterns
- **Data-driven servers**: no hardcoded server names; `ServerConfig` carries all per-server fields. Image resolution: `server.docker_image` → `sandbox_image`.
- **K8s API as source of truth**: status read direct from Deployments + Pods, no caching layer → no stale reads.
- **Config-first server status**: entries start from repository config (DB), then overlay K8s container statuses.
- **Idempotent K8s ops**: creates handle 409 Conflict, deletes handle 404 NotFound.
- **Deterministic naming**: all resource names derived from `task_id` via `naming.rs`, capped at 63 chars.
