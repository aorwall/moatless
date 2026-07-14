# Sandbox Lifecycle & Handling

How the backend creates, watches, protects, stops, recovers, and destroys
Kubernetes sandboxes — as the system behaves **today**, after the reliability
work tracked in `docs/plans/sandbox-reliability-fixes.md` (PRs #285–#301,
#306, #307).

This is a companion to [Sandbox Architecture](./sandbox-architecture.md)
(what runs *inside* the sandbox pod) and
[Architecture Overview](./architecture-overview.md) (the whole system). This
document covers the *control plane*: the lifecycle machinery in
`backend/src/sandbox/` and `backend/src/task/reaper.rs`.

## Table of Contents

1. [Lifecycle at a Glance](#lifecycle-at-a-glance)
2. [Provisioning](#provisioning)
3. [Status Resolution](#status-resolution)
4. [Pod Watcher](#pod-watcher)
5. [Idle Handling: the Task Reaper](#idle-handling-the-task-reaper)
6. [Failure Handling & Recovery](#failure-handling--recovery)
7. [Stopping, Closing, Cleanup](#stopping-closing-cleanup)
8. [Credential Handling](#credential-handling)
9. [Build Caches & Toolchain](#build-caches--toolchain)
10. [Configuration Reference](#configuration-reference)

---

## Lifecycle at a Glance

Each task owns at most one sandbox: a Kubernetes **Deployment** (plus
Service, PVC, Secret, IngressRoutes) named `moatless-ws-{task_id}` in the
sandbox namespace. The Deployment's `replicas` field is the on/off switch —
stop is *scale to 0*, start is *scale to 1* — so a stopped sandbox keeps its
PVC and restarts cheaply.

```
                 provision                    pod ready
  NotCreated ──────────────▶ Initializing ──────────────▶ Ready
      ▲                          ▲    │                     │
      │                          │    │ crash/OOM           │ idle TTL /
      │        start             │    ▼ (K8s restarts)      │ manual stop
      │   (scale to 1, or        │  Initializing            ▼
      │    auto-provision)       └───────────────────── Stopped (replicas=0)
      │                                                     │
      │                 cleanup (task closed/deleted)       │
      └─────────────────────────────────────────────────────┘
```

Three leader-elected background subsystems drive everything that isn't a
direct API call:

| Subsystem | Lease | Job |
|---|---|---|
| **Pod watcher** (`sandbox/k8s/pod_watcher.rs`) | `moatless-pod-watcher` | Watch all sandbox pods, detect lifecycle transitions, publish events |
| **Task reaper** (`task/reaper.rs`) | `moatless-task-reaper` | Stop idle sandboxes, close inactive tasks, close orphaned turns, reconcile desired-vs-actual, GC orphaned resources |
| **Codex refresher** (`credentials/codex/`) | per-scope locks | Keep stored Codex OAuth blobs fresh |

## Provisioning

`POST /api/sandbox/v1/tasks/{task_id}/provision` (or task creation with
`start: true`) creates, per task:

1. **Deployment** — wraps the pod spec; owner of the on/off semantics.
2. **Pod** (via Deployment) — `sandbox` container (Rust runtime + agent CLI,
   ports 8001 HTTP / 50052 gRPC) plus one container per configured server
   (`{name}-server`, data-driven from the repository's `servers` config).
3. **Service** (ClusterIP) — sandbox HTTP/gRPC + all server ports.
4. **PVC** — persistent `/opt/moatless/workspace` (default 3Gi, `ebs` mode).
5. **Secret** — git credentials, backend JWT, agent credentials.
6. **IngressRoute** (Traefik CRD) — one per server port:
   `{task_id}--{port}.{proxy_domain}`.

All resources carry `moatless.io/task-id` labels and **ownerReferences**
pointing at the Deployment, so `cleanup` cascades and the orphan-GC pass can
find leftovers.

Notable behaviors:

- **Auto-provisioning** (`k8s/auto_provision.rs`): `start` on a
  never-provisioned or cleaned-up task rebuilds the full provision plan from
  the DB (task + repository + credentials), so "start" always works.
- **Per-repo resource overrides**: the repository config's `resources` field
  overrides the global CPU/memory requests/limits. Defaults are deliberately
  Burstable: requests 1 CPU / 2Gi, limits 4 CPU / **16Gi** (raised from 8Gi
  after parallel-subagent OOMs, PR #306). Heavy repos (e.g. moatless-vibe
  itself) set their own.
- **Sandbox tokens**: pods authenticate back to the backend with an HS256
  JWT (user token 7d TTL, bootstrap token 30min).

## Status Resolution

`StatusResolver` (`k8s/status_resolver.rs`) computes status **directly from
the K8s API on every read** — there is no status cache to go stale:

1. Deployment missing → `NotFound`; `replicas == 0` → `Stopped`.
2. Otherwise read the pod: phase + sandbox-container readiness →
   `Initializing` / `Ready` / `Error`.
3. Server statuses start from repository config (so servers appear in the UI
   before any pod exists) and overlay the actual container probe states.

The event stream (below) exists for *push* updates; status reads never
depend on it.

## Pod Watcher

A leader-elected watch over all pods labeled `moatless.io/task-id`. State is
tracked **per pod name**, not per task, so rolling updates don't confuse an
old pod's death with the new pod's birth.

### Transitions detected

| Sub-type | Meaning | Published sandbox status |
|---|---|---|
| `pod_scheduled` | Pod placed on a node | Initializing |
| `pod_ready` | Sandbox container ready | Ready |
| `container_restarted` | Container restart observed | Initializing |
| `container_oom_killed` | cgroup OOM kill (exit 137) | Initializing |
| `container_terminated` | Container exited | Initializing |
| `container_crash_loop` | CrashLoopBackOff | Initializing |
| `pod_recovering` | Pod recovering after failure | Initializing |
| `node_reclaimed` | Node removed (spot reclaim / drain), rescheduling | Initializing |
| `pod_evicted` | Kubelet eviction / involuntary removal | Error |
| `pod_failed` | Pod-level failure | Error |
| `pod_succeeded` / `sandbox_completed` | Clean exit | Stopped |
| `pod_deleted` | Pod object gone | NotCreated |

Container-level crashes stay `Initializing` on purpose — K8s restarts them;
only pod-level failures are treated as dead.

### Reliability behaviors

- **Duplicate suppression** (PR #296): for the high-frequency sub-types
  (`container_restarted`, `container_crash_loop`, `container_terminated`)
  the watcher remembers the last *published* sandbox status per pod and
  skips re-publishing the same status. A crash-looping container previously
  emitted an identical event every backoff cycle, flooding SSE and the task
  timeline. Metrics and warn-logs still record every edge — only the
  *publish* is deduplicated — and `container_oom_killed` is always loud.
- **Spot-node removal is not fatal** (PR #306): when a pod disappears with
  `pod_status_reason` in `DeletionByPodGC` / `NodeLost` / `NodeShutdown` /
  `Terminated`, or exits cleanly while the Deployment still desires a
  replica, the watcher classifies it as an involuntary eviction that K8s
  will reschedule instead of `pod_failed`.
- **Node reclaim surfaces its root cause** (`node_reclaimed`): a node-removal
  reschedule is now published as an explicit `node_reclaimed` event
  (Initializing tone) carrying `node_name` + `disruption_reason`, rather than
  staying silent — so the timeline explains *why* the sandbox restarted
  ("Node reclaimed", cause `DeletionByPodGC`) instead of showing a bare stop or
  a scary `pod_failed`. Classification matches the node-removal reason on
  **both** the pod-level `status.reason` and the structured `DisruptionTarget`
  condition (`disruption_reason`); the reason frequently lands on the condition
  first while `status.reason` is still empty, which previously slipped through
  as a fatal `pod_failed`. The node-removal set also includes
  `DeletionByTaintManager` (NoExecute-taint drain during a spot reclaim /
  autoscaler consolidation).
- **Intentional-stop awareness**: a stop initiated through the API (scale to
  0) is distinguished from an eviction so the UI doesn't render a
  user-requested stop as a crash. The idle reaper additionally tags its
  stops with `lifecycle_event: "idle_reaped"` so the UI can say
  "Stopped (idle)".

Events flow: watcher → Redis Streams → SSE (frontend) + event handler
(DB `task_events`, channel adapters).

## Idle Handling: the Task Reaper

One leader-elected loop (default every 300s) runs several passes per cycle.
This is the subsystem most changed by the reliability train — the old
behavior of "stop anything without a recent *message*" reaped working agents
mid-build.

### 1. The activity floor

For every running sandbox and open task, the reaper computes the latest
activity as the **newest** of:

1. Latest conversation message (`messages`)
2. **Latest agent activity event** — tool calls *and* streamed tool
   progress. A multi-minute build inside one turn emits no messages, so the
   sandbox publishes a rate-limited (60s) `agent/activity` liveness event on
   `ToolProgress` (PR #293, `sandbox/src/agent/handle/claude.rs`).
3. `sandbox_started_at`
4. `created_at` (fallback for tasks with no activity at all)

For the idle-*stop* decision the floor is additionally folded with the
latest agent **lifecycle** event: a `started` (running) agent pins the floor
to *now* (never reaped while running); a `completed`/`stopped`/`error` agent
counts its TTL from when it stopped, not from the last message.

### 2. Idle stop decision

Per-task `ttl_minutes` (default `SANDBOX_IDLE_TIMEOUT_MINUTES`, 15; `0` or
negative = never reap). When the floor is older than the TTL, **the reaper
does not trust its own bookkeeping** — it asks the sandbox directly
(PRs #293 + #298):

1. **Live agent check**: one gRPC `GetAgentStatus` per reap candidate. If
   the agent reports anything other than `idle`/`stopped`, skip the stop.
   This is defense in depth against a stale floor (dropped events, watcher
   failover, delivery lag). An unreadable status (gRPC error, no client)
   falls through — otherwise a sandbox with a broken gRPC path could never
   be reaped.
2. **Busy-process check**: the same response carries `busy_processes` — the
   sandbox walks its own process tree (`/proc/<pid>/task/*/children` from
   the container's PID 1) and reports whether any live non-zombie
   descendants exist. Background jobs launched by the agent
   (`run_in_background` builds, dev servers being watched) outlive the
   per-turn CLI subprocess and reparent to the sandbox process, so this
   catches work no event stream sees. A busy sandbox defers the stop —
   but only up to `SANDBOX_BUSY_EXTENSION_MAX_HOURS` (default 4h) past the
   activity floor, so a leaked daemon can't hold a sandbox open forever.
   In tmux mode the signal is hard-wired `false` (the persistent CLI would
   read always-busy), and old sandbox images simply lack the proto field,
   which decodes as `false` — old behavior.

The stop itself is graceful: `StopAgent` gRPC first (so the pod exits
promptly instead of riding termination grace into a SIGKILL/137), then scale
the Deployment to 0, then publish `sandbox/stopped` with
`lifecycle_event: "idle_reaped"`.

### 3. Inactive-task close

Open tasks idle past `inactive_ttl_days` (default 7; `0` = never) are
auto-closed: same `cleanup` + `Closed` status + `task/closed` event sequence
as the manual close handler. Close supersedes stop (cleanup tears down the
Deployment either way).

### 4. Orphaned-turn closer

Turn state is committed transactionally at event ingress (`task_turns`
table), so a turn stuck `active` > 5 minutes is a rare lost-terminal-event
case. The reaper force-closes it only after confirming it's genuinely
orphaned: the Deployment is gone, or the agent itself reports terminal.
Tasks currently being re-provisioned by the reconcile pass are left alone.

### 5. Reconcile pass (config-gated, default OFF)

Desired-vs-actual remediation, all classes opt-in:

- `RECONCILE_REPROVISION_ENABLED`: re-provision tasks whose turn is stuck
  active but whose sandbox resources vanished (bounded by
  `RECONCILE_MAX_REPROVISION_ATTEMPTS`, default 3, per leader lifetime).
- `RECONCILE_PENDING_TIMEOUT_ENABLED`: scale to 0 pods stuck `Pending`
  longer than `RECONCILE_PENDING_TIMEOUT_MINUTES` (default 15) —
  unschedulable pods otherwise wait forever.

### 6. Orphan GC

Deletes sandbox K8s resources whose task no longer exists or was closed more
than `SANDBOX_ORPHAN_GC_RETENTION_HOURS` (default 24) ago. Complements the
ownerReferences cascade for resources that predate it or escaped it.

## Failure Handling & Recovery

- **Container crashes / OOM**: K8s restarts the container in place; the
  watcher publishes `Initializing` (deduplicated across repeats). After
  `SANDBOX_MAX_CRASH_RESTART_COUNT` (default 5) restarts the sandbox is
  declared crash-looping.
- **OOM specifics**: `container_oom_killed` is never deduplicated. The
  container memory *limit* default is 16Gi precisely because concurrent
  subagents plus build children blew through 8Gi (PR #306); requests stay at
  2Gi to preserve node packing.
- **Spot reclaim / node drain**: classified as involuntary eviction with
  expected recovery, not `pod_failed` (see Pod Watcher above). The
  Deployment reschedules the pod; the PVC re-attaches.
- **Stuck provisioning**: `K8S_POD_STARTUP_TIMEOUT_SECS` (120) bounds the
  provision call; the pending-pod reconcile class (opt-in) is the backstop
  for pods that stay unschedulable afterwards.
- **Backend deploys**: sandbox pods are independent Deployments; a backend
  rollout changes leaders (Lease-based) but doesn't touch running sandboxes.

## Stopping, Closing, Cleanup

| Operation | What happens | Reversible? |
|---|---|---|
| **Stop** | Graceful `StopAgent`, scale Deployment to 0. PVC, Service, Secret, IngressRoutes remain. | Yes — `start` scales back to 1 |
| **Start** | Scale to 1; auto-provision from DB if resources are missing | — |
| **Restart** | Delete pods; the Deployment recreates them | — |
| **Cleanup / Close / Delete** | Delete *all* task resources (Deployment, Service, PVC, Secret, IngressRoutes — the Service deletion was a missing cascade step fixed in PR #287) | No |

**Closed is terminal.** Sending a message to a Closed task returns **409
Conflict** (PR #295) instead of silently reviving nothing — the sandbox is
gone and the task will not reopen. External adapters (GitHub PR comments,
Linear, Slack, scheduled triggers) still drop the message silently, since a
409 means nothing to a webhook. Tasks in `Error` status remain sendable.

## Credential Handling

Sandboxes fetch credentials over gRPC (`GetCredentials`, JWT-authed) rather
than baking them into the pod. Two flows matter for lifecycle:

- **GitHub**: token refreshed via the GitHub App service; `/me` reports
  `providers[{provider: "github", connected}]` and the UI shows a Reconnect
  banner when refresh fails.
- **Codex** (PR #301): the refresher keeps `CODEX_AUTH_JSON` fresh with
  `user > team > global` scope precedence, skipping scopes with an open
  codex task (the sandbox watcher rotates the single-use refresh token in
  that case). On *permanent* refresh failure it stamps
  `metadata.refresh_failure`, which now surfaces everywhere:
  - `GET /settings/codex/config` → `needsReconnect: true` → amber
    "Reconnect needed" alert in settings;
  - `/me` providers array gains a `codex` entry (only for users who
    connected Codex) → "Reconnect Codex" TopBar button;
  - **task creation pre-flight**: creating a codex task with no usable
    credential at any scope fails fast with 400 instead of provisioning a
    sandbox that dies on "Sign-in expired" minutes later. Transient refresh
    errors fail open.

## Build Caches & Toolchain

- Volumes per pod: `workspace-data` (PVC, survives stop/start),
  `shared-data` (emptyDir), `build-cache` (emptyDir, sized by
  `K8S_SANDBOX_BUILD_CACHE_SIZE`, default 1536Mi — lost on pod restart),
  `tmp` (memory-backed).
- The sandbox images pin their Rust toolchain to the workspace's
  `rust-toolchain.toml` (1.94.0); a CI `toolchain-drift` job fails any PR
  where `docker/Dockerfile.sandbox-base` and `rust-toolchain.toml` disagree
  (PR #300), so an in-sandbox `cargo build` never triggers a surprise
  rustup download.
- Two opt-in flags (both default off, PR #300):
  - `K8S_SANDBOX_PERSIST_TOOLCHAIN` — mount PVC subPaths at
    `/opt/cargo/registry` and `/opt/cargo/git` so crate downloads survive
    restarts. Deliberately **not** `/opt/cargo` wholesale: K8s subPath
    mounts don't seed from image content, so mounting an empty PVC dir over
    the baked toolchain would shadow it. Empty registry/git caches are safe
    — cargo repopulates them.
  - `K8S_SANDBOX_CARGO_TARGET_ON_BUILD_CACHE` — set
    `CARGO_TARGET_DIR=/opt/moatless/build-cache/target` so build artifacts
    land on the build-cache volume instead of the workspace PVC.

## Configuration Reference

All read from env at backend startup (`backend/src/config.rs`).

### Lifecycle & reaper

| Env var | Default | Meaning |
|---|---|---|
| `SANDBOX_IDLE_REAPER_ENABLED` | `true` | Master switch for the reaper |
| `SANDBOX_IDLE_CHECK_INTERVAL_SECS` | `300` | Reap cycle interval (lease = 2×) |
| `SANDBOX_IDLE_TIMEOUT_MINUTES` | `15` | Default idle TTL (per-task `ttl_minutes` overrides; 0 = never) |
| `SANDBOX_BUSY_EXTENSION_MAX_HOURS` | `4` | Max deferral for live background processes (0 disables) |
| `TASK_INACTIVE_TIMEOUT_DAYS` | `7` | Auto-close floor for Open tasks (per-task override; 0 = never) |
| `SANDBOX_MAX_CRASH_RESTART_COUNT` | `5` | Restarts before crash-loop classification |
| `SANDBOX_ORPHAN_GC_ENABLED` | `true` | Orphaned-resource GC pass |
| `SANDBOX_ORPHAN_GC_RETENTION_HOURS` | `24` | Grace before closed/deleted tasks' leftovers are GC'd |
| `RECONCILE_REPROVISION_ENABLED` | `false` | Re-provision vanished sandboxes with stuck turns |
| `RECONCILE_PENDING_TIMEOUT_ENABLED` | `false` | Scale down pods stuck Pending |
| `RECONCILE_PENDING_TIMEOUT_MINUTES` | `15` | Pending age threshold |
| `RECONCILE_MAX_REPROVISION_ATTEMPTS` | `3` | Per-task re-provision budget |

### Resources & storage

| Env var | Default | Meaning |
|---|---|---|
| `K8S_RESOURCE_REQUEST_CPU` / `_MEMORY` | `1` / `2Gi` | Sandbox container requests |
| `K8S_RESOURCE_LIMIT_CPU` / `_MEMORY` | `4` / `16Gi` | Sandbox container limits (per-repo `resources` overrides win) |
| `K8S_SANDBOX_STORAGE_ENABLED` / `_SIZE` | `true` / `3Gi` | Workspace PVC |
| `K8S_SANDBOX_BUILD_CACHE_SIZE` | `1536Mi` | build-cache emptyDir size |
| `K8S_SANDBOX_SHARED_DATA_SIZE` | `2Gi` | shared-data emptyDir size |
| `K8S_SANDBOX_PERSIST_TOOLCHAIN` | `false` | PVC-backed cargo registry/git caches |
| `K8S_SANDBOX_CARGO_TARGET_ON_BUILD_CACHE` | `false` | CARGO_TARGET_DIR on build-cache |
| `K8S_POD_STARTUP_TIMEOUT_SECS` | `120` | Provision wait bound |

### Credentials

| Env var | Default | Meaning |
|---|---|---|
| `CODEX_REFRESHER_INTERVAL_SECS` | `3600` | Backend codex refresh loop |
| `SANDBOX_USER_TOKEN_TTL_SECS` | 7 days | Sandbox→backend JWT |
| `SANDBOX_BOOTSTRAP_TOKEN_TTL_SECS` | `1800` | Bootstrap JWT |

---

*Sources: `backend/src/task/reaper.rs`, `backend/src/sandbox/k8s/pod_watcher.rs`,
`backend/src/sandbox/k8s/{provider,status_resolver,pod_builder,auto_provision}.rs`,
`sandbox/src/{grpc_server,process_tree}.rs`, `sandbox/src/agent/handle/claude.rs`,
`backend/src/message/send.rs`, `backend/src/credentials/codex/refresh.rs`,
`backend/src/config.rs` — as of main @ July 2026.*
