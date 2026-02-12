# Plan: Resource Pooling, TTL/Auto-Expiry & K8s-Native Improvements

## Context

After comparing our sandbox architecture (`backend/src/sandbox/`) with the [OpenSandbox Kubernetes Controller](https://github.com/OpenSandbox/kubernetes), several gaps and improvement opportunities were identified. OpenSandbox is a K8s-native operator designed for high-throughput batch sandbox delivery with resource pooling and task injection. Our system targets interactive, long-lived dev environments for individual users.

This plan covers three areas:
1. **Resource pooling** — pre-warm pods for near-instant provisioning
2. **TTL/auto-expiry** — deterministic sandbox lifecycle limits
3. **K8s-native improvements** — leverage K8s primitives while keeping our functionality

## Current Architecture (As-Is)

```
Frontend → POST /provision → K8sSandboxProvider
    → build_pod() (code-generated Pod spec)
    → create Deployment (wraps pod, replicas=1)
    → create Service (ClusterIP)
    → create PVC (optional)
    → create IngressRoutes (Traefik)
    → seed Redis cache
    → return Initializing (pod starts in background)

Cold start: 30-90s (image pull + container init + git clone + server startup)
```

**Lifecycle**: provision → initializing → ready → (idle reaper) → stopped → start → ready → cleanup

**Idle reaper**: Leader-elected background task checks message timestamps, stops idle sandboxes (scale Deployment to 0). Activity-based heuristic only — no hard TTL.

---

## 1. Resource Pooling

### Problem

Every `provision()` call creates a fresh Deployment + Pod from scratch. This means:
- Image pulls (even with `IfNotPresent`, first pull per node is slow)
- Container initialization (git clone, npm install, build)
- Server startup (file explorer, preview server)

For a user clicking "Start environment", 30-90 seconds feels unresponsive.

### Approach: Application-Level Warm Pool

Rather than introducing CRDs (which would require a separate operator), we implement pooling inside the existing Rust backend. This keeps operational complexity low and works within our single-binary deployment model.

**Core idea**: Maintain a set of "generic" warm pods (image pulled, container running, base dependencies installed) that can be quickly claimed and customized for a specific task.

### Design

#### Pool Manager (`sandbox/k8s/pool_manager.rs`)

```
┌──────────────────────────────────────────────────────────────┐
│ PoolManager (background task, leader-elected)                │
│                                                              │
│  Config:                                                     │
│    pool_min_buffer: 2    (minimum warm pods)                 │
│    pool_max_buffer: 5    (maximum warm pods)                 │
│    pool_max_total: 20    (hard cap on pool pods)             │
│    pool_image: "..."     (base sandbox image)                │
│                                                              │
│  Loop (every 30s):                                           │
│    1. List pool pods (label: moatless.io/pool=warm)          │
│    2. Count available (not claimed)                          │
│    3. If available < pool_min_buffer:                        │
│         Create (pool_min_buffer - available) new pods        │
│    4. If available > pool_max_buffer:                        │
│         Delete excess pods (oldest first)                    │
│    5. Prune unhealthy pool pods                              │
└──────────────────────────────────────────────────────────────┘
```

#### Pool Pod Lifecycle

```
Pool Pod (warm, generic)              Claimed Pod (task-specific)
─────────────────────────             ──────────────────────────
Labels:                               Labels:
  moatless.io/pool: warm                moatless.io/pool: claimed
  moatless.io/pool-image: <hash>        moatless.io/task-id: <task_id>
                                        moatless.io/claimed-at: <ts>
No task-id label
No Service/Ingress

Pod runs sandbox image with
minimal init (no git clone)
```

#### Claim Flow (modified `provision()`)

```
provision(task_id, request):
  1. Check existing pod (unchanged)
  2. Try claim from pool:
     a. List pods with label moatless.io/pool=warm, phase=Running
     b. Filter by image match (pool-image hash == requested image hash)
     c. If found:
        - Patch pod labels: pool=claimed, add task-id, remove pool=warm
        - Create Service + IngressRoutes pointing to claimed pod
        - Send gRPC init command to pod (git clone, env setup)
        - Return Initializing (init completes in 5-15s vs 30-90s)
     d. If not found:
        - Fall through to current cold-start path
  3. Cold-start path (unchanged current behavior)
```

#### Key Decisions

**Pool pods use Pods directly, not Deployments.** Reasoning:
- Pool pods are ephemeral placeholders — no need for restart/scale semantics
- Once claimed, we wrap them in a Deployment for the task lifecycle
- Simpler cleanup: just delete the Pod

**Wait — actually, we should keep using Deployments for claimed pods.** The Deployment is what enables our stop/start (scale 0/1) semantics. Two options:

**Option A: Pool as bare Pods, convert to Deployment on claim**
- Pool creates bare Pods (fast, simple)
- On claim: delete Pod, create Deployment with same image (pod restarts but image is cached on node)
- Benefit: image is pre-pulled on node, reducing cold start from 90s to ~15-30s
- Drawback: still creates a new Pod (but image pull is instant)

**Option B: Pool as Deployments (scale 0), claim by scaling up + patching**
- Pool creates Deployments with replicas=0 (no running pods, but template ready)
- On claim: patch Deployment with task-specific env vars, scale to 1
- Benefit: uniform Deployment model
- Drawback: pods still start cold, only saves Deployment creation time (~1s)

**Option C (Recommended): Pool as running Pods, adopt into Deployment on claim**
- Pool creates bare Pods running the sandbox image in "idle" mode
- On claim: create a Deployment with `selector` matching the pod's labels, set `replicas: 1`
- K8s Deployment controller adopts the running Pod (no restart)
- Send gRPC `init` command to pod to configure for the task
- Benefit: truly instant — no pod restart, running pod is adopted
- Complexity: Deployment adoption requires matching labels + owner references

**Recommendation**: Start with **Option A** (pool as bare Pods with pre-pulled images). It's simpler, avoids Deployment adoption complexity, and still cuts provisioning time significantly (image pull is the biggest bottleneck). Option C can be explored as a v2 optimization.

#### Config Additions

```
SANDBOX_POOL_ENABLED=true
SANDBOX_POOL_MIN_BUFFER=2
SANDBOX_POOL_MAX_BUFFER=5
SANDBOX_POOL_MAX_TOTAL=20
SANDBOX_POOL_CHECK_INTERVAL_SECS=30
SANDBOX_POOL_IMAGES=<default sandbox image>  # comma-separated for multi-image pools
```

#### New Files

| File | Purpose |
|------|---------|
| `sandbox/k8s/pool_manager.rs` | Leader-elected background pool maintenance |
| `sandbox/k8s/pool_pod.rs` | Pool pod CRUD (create warm pod, claim, release, prune) |

#### Modified Files

| File | Change |
|------|--------|
| `sandbox/k8s/provider.rs` | `provision()` tries pool claim before cold-start |
| `sandbox/k8s/pod_builder.rs` | New `build_pool_pod()` for generic warm pods |
| `config.rs` | Pool config fields |
| `main.rs` | Spawn pool_manager background task |

### Phases

**Phase 1: Image Pre-Pull DaemonSet — ALREADY IMPLEMENTED**

This is already deployed via Helm chart `templates/image-prepull-daemonset.yaml`:
- DaemonSet runs on all sandbox-eligible nodes (respects `sandboxNodeSelector` + `sandboxTolerations`)
- Init container pulls the sandbox image (`imagePullPolicy: Always`), main `pause` container keeps pod alive
- Enabled by default: `kubernetes.imagePrepull.enabled: true`
- Sandbox image resolved via `moatless-vibe.sandboxImage` helper (supports per-env overrides)
- Resource footprint: 10m/16Mi (init), 1m/4Mi (pause) per node

**Remaining gap**: The DaemonSet only pre-pulls the **sandbox** image. The `files-server` and `preview-server` images are NOT pre-pulled. Consider extending the DaemonSet with additional init containers for these images (simple Helm change, no Rust code needed).

**Phase 2: Warm Pod Pool (next step)**
- Pool manager maintains running warm pods (image already cached thanks to Phase 1)
- `provision()` claims warm pod → creates Deployment → pod restarts with cached image
- Expected 10-20s provision time (down from 30-60s, since image pull is eliminated by Phase 1)

**Phase 3: Zero-Restart Claim (optional, v2)**
- Deployment adoption or StatefulSet approach
- Pod runs without restart, receives init via gRPC
- Sub-5s provision time

---

## 2. TTL / Auto-Expiry

### Problem

Currently, sandbox lifetime is managed only by the idle reaper (activity-based heuristic). There's no hard upper bound on how long a sandbox can exist. This means:
- Zombie sandboxes that receive periodic pings but are abandoned
- No way to enforce billing/quota periods
- No guaranteed cleanup for sandboxes with no messages (idle reaper treats these as idle, but only after the timeout)

### Design

#### Expiry Model

Add two TTL concepts:

| Field | Purpose | Default |
|-------|---------|---------|
| `max_lifetime_minutes` | Hard cap from first provision (absolute) | None (unlimited) |
| `idle_timeout_minutes` | Existing: stop after no messages | 60 min |

**`max_lifetime_minutes`** is absolute — the sandbox will be cleaned up regardless of activity after this duration. This is useful for:
- Free-tier users (e.g., max 2 hours per sandbox)
- Cost control (no sandbox runs longer than 24 hours)
- Evaluation/demo environments

#### Storage

Add `expires_at` timestamp to:

1. **K8s annotation**: `moatless.io/expires-at` on the Deployment (set at provision time)
2. **DB column**: `tasks.expires_at` (nullable timestamp, set at provision time)

The annotation is the source of truth for the reaper; the DB column enables frontend display ("Expires in 45 min").

#### Enhanced Idle Reaper → Lifecycle Reaper

Rename `idle_reaper.rs` → `lifecycle_reaper.rs` and extend the reap cycle:

```
reap_cycle():
  1. List running Deployments (replicas > 0)
  2. For each deployment:
     a. Check moatless.io/expires-at annotation
        → If past: cleanup(task_id)  // hard expiry, delete everything
     b. Check idle timeout (existing logic)
        → If idle: stop(task_id)     // soft stop, preservable
  3. List stopped Deployments (replicas == 0)
     a. Check moatless.io/expires-at annotation
        → If past: cleanup(task_id)  // don't keep stopped expired sandboxes
```

**Key distinction**:
- **Idle timeout** → `stop()` (scale to 0, preserves PVC, user can restart)
- **Hard expiry** → `cleanup()` (delete all resources including PVC)

#### Provision Flow Changes

```
provision(task_id, request):
  ...existing steps...

  // Calculate expiry
  if let Some(max_lifetime) = config.sandbox_max_lifetime_minutes {
      let expires_at = Utc::now() + Duration::minutes(max_lifetime);
      // Set annotation on Deployment
      annotations.insert("moatless.io/expires-at", expires_at.to_rfc3339());
      // Update DB
      task_dao::set_expires_at(db, task_id, expires_at).await;
  }
```

#### Frontend Integration

- Status endpoint returns `expires_at` in `WorkspaceStatusFullResponse`
- Frontend shows countdown: "Environment expires in 1h 23m"
- Frontend shows warning at 15min / 5min before expiry
- On expiry, frontend receives `Removing` event via SSE

#### Config Additions

```
SANDBOX_MAX_LIFETIME_MINUTES=<none>     # Hard cap (none = unlimited)
SANDBOX_STOPPED_CLEANUP_MINUTES=1440    # Clean up stopped sandboxes after 24h
```

#### New/Modified Files

| File | Change |
|------|--------|
| `idle_reaper.rs` → `lifecycle_reaper.rs` | Renamed, add hard-expiry + stopped-cleanup logic |
| `types.rs` | Add `expires_at` to `WorkspaceStatusFullResponse` |
| `k8s/provider.rs` | Set expiry annotation on provision |
| `k8s/deployment.rs` | Helper to read/set expiry annotation |
| `config.rs` | `sandbox_max_lifetime_minutes`, `sandbox_stopped_cleanup_minutes` |
| `db/task_dao.rs` | `set_expires_at()`, `get_expires_at()` |
| DB migration | Add `expires_at TIMESTAMPTZ` column to `tasks` table |

---

## 3. K8s-Native Improvements

These changes make the system more idiomatic to Kubernetes without changing user-facing functionality.

### 3.1 K8s Annotations for Structured Metadata

**Current**: Sandbox metadata lives only in Redis cache and DB.

**Improvement**: Store key operational metadata as K8s annotations on the Deployment, making it queryable via `kubectl` and surviving Redis restarts.

| Annotation | Purpose |
|------------|---------|
| `moatless.io/provisioned-at` | Provision timestamp |
| `moatless.io/expires-at` | Hard expiry (from TTL section) |
| `moatless.io/user-id` | Owning user |
| `moatless.io/repository-id` | Source repository |
| `moatless.io/last-active-at` | Last message timestamp (updated by reaper) |
| `moatless.io/provision-source` | `api` or `auto-provision` |

**Benefit**: `kubectl get deploy -l app.kubernetes.io/managed-by=moatless -o custom-columns=...` gives operators full visibility without Redis/DB access.

### 3.2 Finalizers for Safe Cleanup

**Current**: `cleanup()` deletes resources sequentially (Deployment, Service, Ingress, PVC, Secret). If the backend crashes mid-cleanup, orphan resources remain.

**Improvement**: Add a K8s finalizer `moatless.io/cleanup` to the Deployment. The cleanup flow becomes:

```
cleanup(task_id):
  1. Delete Deployment (triggers finalizer)
  2. Finalizer handler (in pod_watcher or separate controller):
     a. Delete Service
     b. Delete IngressRoutes
     c. Delete PVC (if expired or explicitly requested)
     d. Remove finalizer → K8s completes Deployment deletion
```

**Alternative (simpler)**: Use OwnerReferences. Make the Deployment the owner of the Service and IngressRoutes. When the Deployment is deleted, K8s garbage collector cascades the deletion automatically.

**Recommendation**: **OwnerReferences** (simpler, no custom controller needed):

```rust
// In service.rs and ingress.rs:
fn set_owner_reference(resource: &mut ObjectMeta, deployment: &Deployment) {
    resource.owner_references = Some(vec![OwnerReference {
        api_version: "apps/v1".to_string(),
        kind: "Deployment".to_string(),
        name: deployment.metadata.name.clone().unwrap(),
        uid: deployment.metadata.uid.clone().unwrap(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }]);
}
```

With this, `cleanup()` becomes: delete Deployment + delete PVC (PVC intentionally not owned, since it may outlive the Deployment for data preservation).

**Impacted files**:
- `k8s/service.rs` — set owner reference to Deployment on create
- `k8s/ingress.rs` — set owner reference to Deployment on create
- `k8s/provider.rs` — `cleanup()` simplifies to: delete Deployment + optionally delete PVC
- `k8s/deployment.rs` — `create_workspace_deployment()` returns full Deployment object (for UID)

### 3.3 Pod Disruption Budgets

**Current**: No PDB. If a K8s node drains (upgrade, scaling event), all sandbox pods on that node are evicted simultaneously.

**Improvement**: Create a PodDisruptionBudget per sandbox:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: moatless-ws-{task_id}
spec:
  minAvailable: 1
  selector:
    matchLabels:
      moatless.io/task-id: {task_id}
```

**Benefit**: During voluntary disruptions (node drain, cluster upgrades), K8s will only evict sandbox pods one at a time, giving each time to reschedule.

**Complexity**: Low — create PDB alongside Deployment in `provision()`, delete in `cleanup()` (or use OwnerReference).

### 3.4 Resource Quotas & LimitRanges

**Current**: Sandbox pods have resource limits on server containers but not on the sandbox container itself.

**Improvement**:
- Add resource requests/limits to the sandbox container in `pod_builder.rs`
- Create a `ResourceQuota` per user or per namespace to cap total sandbox resource usage

```
SANDBOX_RESOURCES_CPU_REQUEST=500m
SANDBOX_RESOURCES_CPU_LIMIT=2000m
SANDBOX_RESOURCES_MEMORY_REQUEST=512Mi
SANDBOX_RESOURCES_MEMORY_LIMIT=4Gi
```

### 3.5 Replace Deployment with StatefulSet (Considered, Not Recommended)

StatefulSets provide stable network identity and ordered pod management. However:
- We use Deployments for scale 0/1 (stop/start), which StatefulSets also support
- StatefulSets add complexity (ordered creation, stable hostname requirements)
- We don't need stable pod names (our naming is Deployment-based)
- PVC binding is already handled by label matching in `pvc.rs`

**Decision**: Keep Deployments. They're the right abstraction for our use case.

### 3.6 K8s Events for Audit Trail

**Current**: Lifecycle events go to Redis Streams only.

**Improvement**: Also emit K8s Events on the Deployment:

```rust
use k8s_openapi::api::core::v1::Event;

async fn emit_k8s_event(k8s: &K8sClient, deployment: &Deployment, reason: &str, message: &str) {
    // Create Event object referencing the Deployment
    // reason: "Provisioned", "Stopped", "Started", "Expired", "CleanedUp"
}
```

**Benefit**: `kubectl describe deploy moatless-ws-{task_id}` shows full lifecycle history. Integrates with K8s monitoring tools (Datadog, Grafana).

---

## Implementation Priority

| Priority | Item | Impact | Effort | Dependencies |
|----------|------|--------|--------|--------------|
| ~~Done~~ | ~~Phase 1 Pool (DaemonSet image pre-pull)~~ | ~~Faster cold starts~~ | ~~Done~~ | Already in Helm chart |
| **P0** | TTL/Auto-Expiry | Cost control, prevents zombie sandboxes | Small | DB migration |
| **P0** | OwnerReferences | Prevents orphan resources on crash | Small | None |
| **P1** | K8s Annotations | Operational visibility | Small | None |
| **P1** | Sandbox container resource limits | Stability, prevents noisy neighbors | Small | None |
| **P1** | Pre-pull server images too | Extend DaemonSet for files-server + preview-server | Small | Helm change only |
| **P2** | Stopped sandbox cleanup | Reclaims PVCs | Small | TTL |
| **P2** | Phase 2 Pool (warm pods) | Sub-20s provisioning | Medium | Phase 1 (done) |
| **P3** | PodDisruptionBudgets | Graceful node drains | Small | None |
| **P3** | K8s Events | Audit trail | Small | None |
| **P4** | Phase 3 Pool (zero-restart) | Sub-5s provisioning | Large | Phase 2 |

## Migration & Rollout

- All new config fields have sensible defaults (pooling disabled, TTL unlimited)
- OwnerReferences added on new provisions only; existing sandboxes get them on next `start()`
- DB migration adds nullable `expires_at` column (backwards compatible)
- `idle_reaper.rs` → `lifecycle_reaper.rs` rename is internal (no API change)
- Pool manager is opt-in (`SANDBOX_POOL_ENABLED=false` by default)

## Not In Scope

- **CRD-based operator**: Our backend is an application-level orchestrator, not a K8s operator. Introducing CRDs would require kubebuilder/operator-sdk and a separate deployment model. The benefits (declarative state, `kubectl` management) don't outweigh the complexity for our interactive use case.
- **Batch sandbox delivery**: OpenSandbox's `replicas: N` batch semantics target RL training/evaluation. We serve individual users with 1 sandbox per task.
- **Task injection**: OpenSandbox's task-executor sidecar pattern is for running arbitrary processes in sandboxes. Our sandboxes run a fixed agent (Claude Code). If we need this later, it would be a separate feature.
- **Multi-cluster support**: Out of scope for now; pools and TTL are per-namespace.
