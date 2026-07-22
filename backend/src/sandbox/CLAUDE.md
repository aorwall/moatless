# sandbox module

Rust (Axum) service managing the full lifecycle of Kubernetes- and Docker-based sandbox environments. Each sandbox is a multi-container pod/container identified by `task_id`, provisioned on demand. Runs as part of the `moatless-vibe-backend` binary.

## Architecture

### Lifecycle module (`lifecycle/`)
Deep module owning durable lifecycle intent and orchestration per Task, per
`specs/sandbox/04-lifecycle-architecture.md`. **Migration status: step 2 caller
migration landed.** The typed surface (`LifecycleOutcome` with independent
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
Sandbox intent: every start/stop/restart/redeploy/remove caller — task and
sandbox handlers, adapters, the reaper, message-send/notify, and the pod watcher's
crash remediation — routes through it, over the `PgLifecycleStore`/`PgLifecycleLock`
Postgres adapters, the `RuntimeBridge` over `SandboxProvider`, `GrpcAgentStop`, and
the `OutboxEventSink`. `delete_task` requires a successful `lifecycle.remove` before
row deletion. Durable Sandbox state — desired state, stop reason, archive
retention, the correlation marker, the start/intent timestamps, and the per-Task
generation counter — lives in the **`sandboxes` table** (PK `task_id`, FK
`ON DELETE CASCADE`), born lazily through the intent upsert: no row means no
Sandbox was ever requested (converged Stopped, always wakeable). The row is
therefore the mechanism making lifecycle the sole intent writer by construction.
The status handlers and reaper read desired state through the store
(`Lifecycle::desired_state`); task responses read the row via
`TaskRepository::get_sandbox_intents`. The transactional outbox is in place.

The pending-Message query/index and the desired-state reconciler are in place:
the reconciler is lease-elected on K8s / lease-free single-replica on Docker,
re-notifies a woken Ready sandbox of its undelivered pending messages, and
subsumes the `Removed`-intent orphan rule. Remaining step-2 work: thread
`RuntimeOperationContext` generation into the provider mutations so pods/containers
carry generation labels (and restrict Docker observation to the sandbox
container); the stale edge-marker (restart/redeploy) handling (both need
generation identity); and deleting the reaper's retired `run_reconcile_pass` (no
later than step 4). Steps 3–4 (rename `SandboxProvider` → `SandboxRuntime`, delete
shallow paths) remain pending.

### Provider abstraction
`SandboxProvider` trait (`provider.rs`) is the provider-agnostic interface for lifecycle operations (provision, start, stop, restart, cleanup, status, server logs, file search, `sandbox_client()`). Implementations: `K8sSandboxProvider` (`k8s/provider.rs`) and `DockerSandboxProvider` (`docker/provider.rs`). Migration step 3 renames this to `SandboxRuntime` (`provision` → `ensure_running`) and splits out the read-side `SandboxAccess` seam.

### Request flow
```
HTTP → routes.rs (Axum OpenApiRouter)
     → handlers.rs      (lifecycle: provision, stop, start, restart, cleanup, status, search)
     → file_handlers.rs (file ops: tree, read, write, list, search-by-name)
         → provider.sandbox_client() → SandboxClient (gRPC direct)
     → SandboxProvider impl → k8s/* or docker/*
```
File handlers call `provider.sandbox_client()` for a ready `SandboxClient`, then invoke file methods directly — no provider-level duplication.

### K8s resources per sandbox
Provisioning creates (all labeled `moatless.io/task-id={task_id}`): a **Deployment** (enables stop = scale to 0 / start = scale to 1 without re-provisioning), its **Pod**, a ClusterIP **Service**, an optional **PVC**, and optional **IngressRoute** (Traefik CRD, one per server port, pattern `{task_id}--{port}.{proxy_domain}`). Deployment/Service names are deterministic: `moatless-ws-{sanitized_task_id}` (`k8s/naming.rs`, 63-char cap).

### Pod structure
- **`sandbox` container**: agent environment (image configurable per-repo), ports 8001 (HTTP) + gRPC, talks back to the backend via gRPC.
- **Server containers** (dynamic, data-driven from `ServerConfig`): each specifies its own `docker_image` (falls back to `sandbox_image`), `start_commands`, `env_vars`, port, capabilities. Name convention `{server.name}-server`; TCP readiness probe on the port.
- **Volumes**: `workspace-data` (PVC), `shared-data`/`build-cache` (emptyDir), `tmp` (memory-backed).

### Status resolution
`StatusResolver` (`k8s/status_resolver.rs`) reads status directly from the K8s API — no Redis cache: (1) Deployment replicas (0 = Stopped, missing = NotFound), (2) pod phase + sandbox container readiness, (3) server status by overlaying container probe states onto repo-config-defined servers (config-first, so servers show in the UI before pods exist).

### Pod watcher
`pod_watcher.rs` — leader-elected (K8s Lease) task that watches all sandbox pods, detects lifecycle transitions (OOM, crash loop, eviction, restart, ready, failed, deleted), and publishes events to Redis Streams for SSE/event-handler consumption. Tracks pods **by pod_name, not task_id**, so rolling updates don't confuse old/new pods.

### Task reaper
The unified reaper lives in **`task/reaper.rs`, not this module**. Leader-elected; each cycle (1) scales idle Deployments to 0 after `ttl_minutes`, and (2) auto-closes Open tasks inactive past `inactive_ttl_days` via the same `cleanup` + `update_task(Closed)` + `TaskSubType::Closed` sequence as the manual close handler.

### gRPC communication
`SandboxClient` (`client.rs`) calls sandbox pods: execute-agent, stop-agent, notify-messages, file ops, search-files. Lazy connection, 5s connect / 10s request timeout. `classify_grpc_error()` maps gRPC → `SandboxError`: Unavailable/DeadlineExceeded/Cancelled → NotReady/503, NotFound → 404, FailedPrecondition/InvalidArgument → 400, else → Internal/500. `notify.rs` is fire-and-forget message notification.

### Auto-provisioning
`k8s/auto_provision.rs` lets `start()` transparently provision a never-provisioned workspace by reading task + repository from the DB and resolving credentials (provider_tokens table → env var fallback). Only injects default servers when the DB `servers` column is empty (back-compat for old repos).

### JWT auth
`jwt.rs` generates/verifies HS256 JWTs (90-day expiry, issuer `moatless-vibe-backend`) that sandbox pods use to authenticate back to the backend.

## K8s label convention
- `moatless.io/task-id` — task identifier (selector queries)
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
