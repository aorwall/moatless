# Failure modes — symptom → cause → fix

## Quick table

| Symptom / event | How to confirm | Root cause | Fix |
|---|---|---|---|
| `container_oom_killed`, exit 137 | `terminated_reason=OOMKilled` on a container | Container cgroup memory limit exceeded (often N parallel subagents + build children) | Raise the **limit**: `moat repos update <repo> --resource-memory-limit 24Gi` → `moat tasks restart`. Cluster default: `K8S_RESOURCE_LIMIT_MEMORY` in `backend/src/config.rs` + Helm `values.yaml` |
| `pod_evicted`, "Usage of EmptyDir volume … exceeds the limit" | `status.reason=Evicted` in pod JSON; `runtimeEvents` in `moat tasks diagnose` | An emptyDir volume exceeded its `sizeLimit` (build artifacts, logs) | Clean the dir or raise the size knob (below) |
| `node_reclaimed` / `DeletionByPodGC` / `NodeLost` / `NodeShutdown` / `Terminated` / `DeletionByTaintManager` | `kubectl get node <old-node>` → `NotFound` | **Benign** spot-node reclamation; pod auto-reschedules | None — expected. Not a real failure |
| `container_crash_loop` / `CrashLoopBackOff` | `waitingReason=CrashLoopBackOff` on sandbox container | Sandbox binary crashing on startup | `moat tasks logs <task> --server sandbox --previous`; crash-looping declared after `SANDBOX_MAX_CRASH_RESTART_COUNT` (default 5) |
| Stuck `Initializing`, `/health` 503 | `moat tasks diagnose` waitingReason; `runtimeEvents` | Image pull, scheduling, or volume-bind problem | `ImagePullBackOff`/`ErrImagePull`/`InvalidImageName`/`CreateContainerConfigError` → fix image/secret; no-node → check node pool |
| Server (preview/files) `failed` | `moat tasks status` per-server error; `--server preview --previous` logs | Bad start command / port / build failure | `moat repos get <repo>` → fix `--servers-json` → `moat tasks redeploy` |
| Idle-stopped | `sandbox/stopped` event, `lifecycle_event: idle_reaped` | Reaper stopped an idle sandbox past TTL | `moat tasks start <task>` (PVC preserved) |
| `turn_interrupted` taskTurn event | correlate with a sandbox event at same UTC timestamp | Pod terminated involuntarily mid-turn (eviction/OOM/reclaim) | Diagnose the paired sandbox event; not itself an error |

## Volume size knobs (emptyDir eviction sources)

Exceeding any emptyDir `sizeLimit` gets the pod **evicted by the kubelet**.
Defined in `backend/src/sandbox/k8s/pod_builder.rs`; overridable via repo
`resources` or these env vars (defaults in `backend/src/config.rs` / Helm values):

| Volume | Mount | Env knob | Default |
|---|---|---|---|
| `build-cache` | build caches (e.g. cargo target) | `K8S_SANDBOX_BUILD_CACHE_SIZE` | `1536Mi` |
| `shared-data` | `/opt/moatless/data` (server logs/artifacts) | `K8S_SANDBOX_SHARED_DATA_SIZE` | `2Gi` |
| `workspace-data` (Ephemeral/S3 tiers) | workspace | `K8S_SANDBOX_STORAGE_SIZE` | `3Gi` |
| `docker-storage` (dind, opt-in) | dind data | `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE` | `10Gi` |

> Watch out: if `K8S_SANDBOX_CARGO_TARGET_ON_BUILD_CACHE=true` moves
> `CARGO_TARGET_DIR` onto `build-cache`, a large Rust repo can blow the 1536Mi
> cap and get evicted — raise `K8S_SANDBOX_BUILD_CACHE_SIZE`.

## Storage backends (workspace persistence)

Resolved per-repo `resources.storage_mode` → `K8S_SANDBOX_DEFAULT_STORAGE_MODE`
→ `Ebs` (`backend/src/sandbox/k8s/storage.rs`):

| Backend | Mechanism | Survives restart? |
|---|---|---|
| `Ebs` (default) | Per-task PVC (RWO), size = `K8S_SANDBOX_STORAGE_SIZE` | Yes |
| `Ephemeral` | Node-local emptyDir | No (only transcripts persist via S3) |
| `S3` | emptyDir + workspace-sync loop to/from S3 | Effectively yes, via S3 |

## Lifecycle timing / crash knobs

| Knob | Meaning | Default |
|---|---|---|
| `K8S_POD_STARTUP_TIMEOUT_SECS` | bounds provisioning; drives startup-probe `failureThreshold` | 120 |
| `SANDBOX_MAX_CRASH_RESTART_COUNT` | restarts before "crash-looping" | 5 |
| `K8S_SANDBOX_LIVENESS_ENABLED` | opt-in `/livez` liveness probe (heartbeat-backed) | false |

## The timezone trap (recurring dead-end)

Event timestamps are **UTC**; the UI often shows local time. A filter built from
a local timestamp silently returns nothing. **Always dump a wide window first
(`tail -80`), then narrow.** This wasted real time in past investigations.
