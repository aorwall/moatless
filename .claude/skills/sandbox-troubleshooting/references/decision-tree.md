# Symptom decision tree — detect → confirm → cause → fix

Once you've spotted a symptom (via the detection moves in SKILL.md), jump to the
matching branch. Command blocks are in [commands.md](commands.md); the
symptom→cause→fix lookup table and all knobs are in
[failure-modes.md](failure-modes.md).

## Sandbox restarted / turn interrupted
1. `moat events list <task_id> --types sandbox,server --limit 400 --json` — read
   the sandbox `lifecycle_event`s around the restart time (**events are UTC —
   widen the window before narrowing; a local-vs-UTC mismatch silently returns
   nothing**).
2. Classify the cause:
   - `container_oom_killed` / exit 137 → **OOM** branch below.
   - `pod_evicted` + "exceeds the limit" → **eviction** branch below.
   - `node_reclaimed` / `DeletionByPodGC` / `NodeLost` / `NodeShutdown` →
     **benign spot reclaim**; the pod auto-reschedules. Confirm with
     `kubectl get node <old-node>` → `NotFound`. Not a real failure.
   - `container_crash_loop` → sandbox binary crashing; read
     `moat tasks logs <task_id> --server sandbox --previous`.

## Won't start / stuck Initializing
- `moat tasks diagnose <task_id>` → look at `containerDiagnostics[].waitingReason`
  on the **sandbox** container: `ImagePullBackOff` / `ErrImagePull` /
  `InvalidImageName` / `CreateContainerConfigError` / `CrashLoopBackOff` mark it
  permanently failed (`environment_status: error`).
- Startup probe (`GET /health` on :8001) returns 503 until the agent is Ready;
  if it never flips, check `runtimeEvents` for scheduling problems (no node,
  volume not bound). See `docs/sandbox-lifecycle.md`.

## OOMKilled
- Confirm: `terminated_reason=OOMKilled`, `exit_code=137` on a container.
- Cross-check what was running: read the task transcript
  (`moat messages list <task_id> --full`) — parallel subagents + build children
  are the usual cause.
- Fix: raise the memory **limit** (not the request):
  `moat repos update <repo_id> --resource-memory-limit 24Gi`, then
  `moat tasks restart <task_id>`. Cluster default lives in
  `backend/src/config.rs` + Helm `values.yaml`.

## Evicted (volume overflow — the most common self-inflicted restart)
- An emptyDir (`build-cache`, `shared-data`, workspace when Ephemeral/S3) exceeds
  its `sizeLimit` → kubelet evicts the pod.
- Confirm: `kubectl get pod <pod> -n $NS -o jsonpath='{.status.reason}{" "}{.status.message}'`
  → `Evicted` / "Usage of EmptyDir volume ... exceeds the limit". Same fact shows
  up as a `runtimeEvents` entry in `moat tasks diagnose`.
- Fix: clean the offending dir, or raise the size knob
  (`K8S_SANDBOX_BUILD_CACHE_SIZE`, `K8S_SANDBOX_SHARED_DATA_SIZE`,
  `K8S_SANDBOX_STORAGE_SIZE`). Knob table in [failure-modes.md](failure-modes.md).

## Preview / server container failing
- `moat tasks status <task_id>` → per-server status/error/detail.
- `moat tasks logs <task_id> --server preview --previous` (server names:
  `sandbox`, `preview`, `files`, or the configured `*-server` suffix).
- Config problems: `moat repos get <repo_id>` → fix `--servers-json` → restart.

## Agent stuck / not responding / errored
- `moat tasks status <task_id>` → agent state. `none` while the pod is Running
  can mean the gRPC `GetAgentStatus` call failed (network/DNS) rather than a
  genuinely idle agent — check sandbox logs.
- `moat events list <task_id> --types agent --json` → `error`,
  `recovery_exhausted`, `session_context_lost`, `retrying` subtypes.
- `moat messages list <task_id> --full` → read the transcript to see the last
  thing the agent did, a tool error, or a permission prompt it's blocked on.
- A `taskTurn` `turn_interrupted` event ⇒ the pod was terminated mid-turn —
  correlate with the sandbox event at the same UTC timestamp.

## Idle-stopped unexpectedly
- The reaper stops a sandbox only when TTL is exceeded **and** a live gRPC
  `GetAgentStatus` confirms idle with no busy processes. Look for a
  `sandbox/stopped` event with `lifecycle_event: "idle_reaped"`.
- Just `moat tasks start <task_id>` to resume (PVC state is preserved).

## restart vs redeploy vs raise-limits

- `moat tasks restart <task_id>` — recreate the pod, **keep** the PVC. First
  move for a wedged/crashed pod.
- `moat tasks redeploy <task_id>` — delete+recreate Deployment/Service/Ingress
  with **fresh repo config**, keep the PVC. Use after changing server config.
- Raise resource limits (`moat repos update ... --resource-memory-limit`) — for
  recurring OOM; then restart.
