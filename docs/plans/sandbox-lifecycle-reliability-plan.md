# Sandbox Lifecycle & Event Reliability Plan

**Status**: Proposed
**Date**: 2026-07-11
**Scope**: `backend/src/sandbox/` (K8s provider, pod watcher, event pipeline), `sandbox/src/` (runtime, event publishing), `backend/src/event_handler/`, `backend/src/task/reaper.rs`

## Background

A deep review of the K8s sandbox solution concluded that the overall design is sound:
Deployments with `Recreate` strategy for RWO PVCs, kube-rs `watcher()` with proper
Init/InitApply resync, Lease-based leader election, idempotent provisioning via 409
handling, at-least-once Redis Streams consumption with PEL recovery, and durable event
persistence to the `task_events` table.

The problems are concentrated in three areas:

1. **Event reliability** — the sandbox→backend `PublishEvent` hop is fire-and-forget,
   and authoritative state (task status, turn lifecycle) is written *only* by event
   consumers. A lost event at the first hop is unrecoverable and leaves stale state
   that compensating logic (the reaper) can only partially rescue.
2. **Lifecycle correctness** — a handful of concrete bugs and gaps: the crash-loop
   auto-stop can wedge permanently, no liveness probes exist, cleanup can orphan PVCs,
   and no `ownerReferences` tie resources together.
3. **Security hardening** — user JWT and repo URL exposed in pod spec, containers run
   as root, unbounded emptyDir.

This doc is the prioritized fix plan. Phases are ordered by risk-reduction per effort;
each item is independently shippable.

---

## Phase 1 — Event reliability (highest value)

### 1.1 Sandbox-side event outbox (at-least-once first hop)

**Problem.** `publish_event` in `sandbox/src/backend_client.rs:177` is fire-and-forget:
one gRPC attempt (10s call timeout), failures logged at `warn` and dropped. During any
backend unavailability (backend rollout, network blip), all `agent.message`,
`agent.completed`, `taskTurn.active` events for that window are permanently lost.
Everything downstream (Redis consumer groups, `task_events` persistence, reaper
reconciliation) is at-least-once — but only for events that entered the pipeline.
Critically, `reconcile_orphaned_turns` depends on the `taskTurn/active` row existing;
if *that* event is lost, the stuck turn is invisible to the reaper too.

**Fix.** Introduce a small outbox in the sandbox runtime:

- A `tokio::sync::mpsc` -backed queue with a single drainer task. `publish_event`
  becomes enqueue-only (non-blocking, never fails the caller).
- The drainer sends events in order with retry + exponential backoff (cap ~30s),
  preserving per-task ordering. Attach a monotonically increasing `sequence` field so
  the backend can detect gaps/duplicates.
- Backend `publish_event` handler (`backend/src/sandbox/grpc_server.rs:185`) dedupes on
  `event_id` (already unique) — Redis `XADD` duplicates are then filtered by the
  existing `ON CONFLICT (event_id) DO NOTHING` persistence, but SSE consumers would see
  duplicates, so dedupe at the gRPC ingress with a short-lived per-task LRU of recent
  event ids.
- Bound the queue (e.g. 10k events / 32MB). On overflow, drop oldest *non-lifecycle*
  events first (keep `taskTurn.*`, `agent.completed`, `agent.error` at all costs) and
  emit a synthetic `events_dropped` marker so the gap is visible.
- On graceful shutdown (`sandbox/src/shutdown.rs`), drain the queue with a bounded
  timeout (~5s) before the transcript flush.
- Optional durability upgrade (follow-up, not required initially): spill the queue to
  `$WORKSPACE_DIR/.moatless/outbox.jsonl` on the PVC so events survive a pod crash.

**Effort**: ~2–3 days. **Risk**: low — additive, no protocol change beyond a new field.

### 1.2 Transactional state writes; events become projections

**Problem.** `tasks.status` and turn lifecycle are written by the Redis event-handler
consumer (`backend/src/event_handler/agent_handlers.rs`), not at the point the backend
learns the fact. There is no turns table — turn openness is *derived* from the
`task_events` log via `DISTINCT ON` (`backend/src/task_event/dao.rs:161`). The event
stream is thus the source of truth for business state; a lost or delayed event means
stale state, compensated by reaper heuristics.

**Fix.** Invert to a transactional-outbox shape at the gRPC ingress:

- In the backend `publish_event` gRPC handler, for state-bearing event types
  (`taskTurn.*`, `agent.completed`, `agent.error`, `agent.stopped`): write the DB state
  change (task status, `task_events` row) in one transaction *first*, then publish to
  Redis for SSE fan-out. The existing event-handler consumer keeps handling
  side-effects (channel notifications etc.) but no longer owns status writes — make its
  status updates idempotent no-ops when the row is already current.
- Add a `task_turns` table (`task_id`, `turn_number`, `status`, `agent_type`,
  `started_at`, `ended_at`, PK `(task_id, turn_number)`) and write it in the same
  transaction. `list_tasks_with_unclosed_active_turn` becomes an indexed query on
  `task_turns` instead of a `DISTINCT ON` scan over the event log. Migration goes in
  `drizzle/`.
- Keep `reconcile_orphaned_turns` as the backstop — it becomes simpler and cheaper
  against `task_turns`.

**Effort**: ~1 week including migration + backfill of open turns. **Risk**: medium —
touches the write path; mitigate by keeping the consumer-side writes as idempotent
fallbacks during rollout.

### 1.3 Events tab reads from Postgres, not Redis XRANGE

**Problem.** `GET /api/v1/tasks/{task_id}/events` replays `XRANGE` against the Redis
stream (`backend/src/sse/routes.rs:157`), which has a 72h TTL and `MAXLEN` trimming —
even though the durable `task_events` table holds the same data. History silently
truncates.

**Fix.** Serve historical events from `task_events` (paginated), and use Redis only for
the live SSE tail. Ensure the SSE subscription starts from the last-persisted event id
to avoid a gap between the two sources.

**Effort**: ~1 day. **Risk**: low.

---

## Phase 2 — Lifecycle correctness

### 2.1 Fix the crash-loop auto-stop wedge

**Problem.** When restart count crosses `max_crash_restart_count`, the watcher scales
the Deployment to 0. If `scale_deployment` fails, it's `warn!`-only
(`backend/src/sandbox/k8s/pod_watcher.rs:824`), and because the stored previous state's
restart count is now ≥ threshold, the "new crossing" edge never re-fires. The pod
crash-loops forever with no further remediation.

**Fix.** Track `auto_stop_pending: bool` in `PodObservedState`; set it when the
threshold crossing is detected, clear only on successful scale-to-0. On every
subsequent watcher event for that pod (and in the periodic sweep), retry while pending.
Same pattern for the exit-code-78 fast-fail path (`pod_watcher.rs:763`).

**Effort**: ~half a day. **Risk**: low.

### 2.2 `ownerReferences` for cascading cleanup

**Problem.** Each task creates Deployment + Service + PVC + N IngressRoutes with no
ownership chain. `cleanup()` (`backend/src/sandbox/k8s/provider.rs:289`) deletes each
independently, warn-on-error; any transient API failure orphans resources (worst case:
a bound PVC billing forever). There is no GC pass for orphans.

**Fix.**
- Create the Deployment first, then set `ownerReferences` on Service and IngressRoutes
  pointing at the Deployment. `cleanup()` then only needs to delete the Deployment and
  the PVC; Service/IngressRoutes cascade.
- Do **not** owner-ref the PVC to the Deployment — the PVC must survive stop/redeploy.
  Instead add an orphan-GC pass to the reaper: LIST PVCs labeled
  `moatless.io/task-id`, delete those whose task has been deleted/closed beyond a
  retention window. Same sweep can catch orphaned Services/IngressRoutes created before
  this change.
- Make `cleanup()` retry-aware: return an error (or enqueue a deferred cleanup) when
  any delete fails, instead of returning `Ok(())`.

**Effort**: ~2 days including the reaper GC pass. **Risk**: low-medium — verify
Traefik IngressRoute CRDs accept owner refs to Deployments (they do; GC is
resource-agnostic).

### 2.3 Liveness probe + honest `/health`

**Problem.** No container in the pod has a liveness probe
(`backend/src/sandbox/k8s/pod_builder.rs`), and the sandbox `/health` endpoint returns
200 whenever `InitPhase::Ready` (`sandbox/src/main.rs:235`) regardless of whether the
agent actor loop is alive. A hung-but-alive sandbox is never restarted by K8s; the
watcher only detects exit-based failures.

**Fix.**
- Extend `/health` (or add `/livez`) to verify the agent actor is responsive: send a
  no-op command over the actor channel with a short timeout, or have the actor bump a
  heartbeat `AtomicU64` that the handler checks for staleness. Keep the existing
  init-phase gate as the readiness signal.
- Add a liveness probe to the sandbox container: `periodSeconds: 10`,
  `timeoutSeconds: 5`, `failureThreshold: 6` (generous — a false-positive restart kills
  an in-flight agent turn, so tune conservatively and rely on the startup probe for
  boot).

**Effort**: ~1 day. **Risk**: medium — a too-aggressive probe restarting mid-turn pods
is worse than the status quo; ship the endpoint first, enable the probe behind config
(`K8S_SANDBOX_LIVENESS_ENABLED`), observe, then default on.

### 2.4 Small lifecycle bugs

| Bug | Location | Fix |
|---|---|---|
| `check_existing_pod` uses `list.items[0]` instead of `pick_best_pod` | `backend/src/sandbox/k8s/provider.rs:654` | Use `pick_best_pod` |
| `wait_for_pods_deleted` proceeds after 90s timeout, reintroducing the RWO Multi-Attach stall it exists to prevent | `backend/src/sandbox/k8s/deployment.rs:287` | Return an error on timeout; caller surfaces `environment_error` instead of stalling in Pending |
| IngressRoute name truncation is a byte slice on an unsanitized server name — mid-UTF-8 boundary panics; result can end in `-` (invalid) | `backend/src/sandbox/k8s/ingress.rs:58` | Run `server_name` through `sanitize_k8s_name`, trim trailing `-` after truncation |
| PVC create has no 409 handling; the lose-the-race path silently degrades to ephemeral storage | `backend/src/sandbox/k8s/pvc.rs:97`, swallow at `provider.rs:131` | On 409, re-run `find_task_pvc` and use the winner; only degrade to ephemeral on genuine failure, and publish a visible warning event |
| `TASK_ID` defaults to empty string | `sandbox/src/config.rs:280` | `expect()` like `BACKEND_GRPC_URL` |
| `materialize_ssh_keys` called twice back-to-back | `sandbox/src/init/credentials.rs:172,178` | Delete the duplicate block |
| `moatless.io/created-at` annotation is `{epoch}Z`, not RFC3339 | `backend/src/sandbox/k8s/pod_builder.rs:935` | Emit RFC3339 |
| `redeploy()` warn-and-continue: if the Deployment delete fails, the subsequent create hits 409 → "reuse" → old spec survives, silently defeating the redeploy | `backend/src/sandbox/k8s/provider.rs:269` | Fail the redeploy when the delete fails |

**Effort**: ~1–2 days total. **Risk**: low.

---

## Phase 3 — Security hardening

### 3.1 Move `USER_AUTH_TOKEN` off the pod spec

**Problem.** A 90-day user JWT is a plaintext env var
(`backend/src/sandbox/k8s/pod_builder.rs:498`) — visible in etcd, `kubectl describe`,
and to every process in the pod. The codebase already has the right pattern for
everything else: credentials delivered over gRPC `GetCredentials` at runtime.

**Fix.** Replace the env JWT with a short-lived bootstrap token (minutes, single
purpose: authenticate the first `GetCredentials` call), and deliver the long-lived
token via the existing credential channel. Shorten the JWT TTL regardless.

### 3.2 Stop annotating the raw repository URL

**Problem.** `moatless.io/repository-url` stores the raw `git_clone_url` as a pod
template annotation (`pod_builder.rs:52`). If the URL embeds a PAT/password, it's in
etcd. Same exposure for `GIT_CLONE_URL` env (`pod_builder.rs:442`).

**Fix.** Redact credentials from the annotation (host + path only). For the env var,
pass the credential-free URL and let the sandbox's credential broker (already wired as
a git credential helper via `/run/moatless/credentials.sock`) supply auth.

### 3.3 Run as non-root; bound the shared emptyDir

- Implement the existing TODO at `pod_builder.rs:896`: `runAsUser: 1000`,
  `runAsNonRoot: true`, pod-level `fsGroup`, seccomp `RuntimeDefault`. Requires image
  work (file ownership) — coordinate with `docker/Dockerfile.sandbox*`.
- Add `sizeLimit` to the `shared-data` emptyDir (`pod_builder.rs:81`) — currently
  unbounded node-disk usage. Also add requests/limits to the `nvme-setup` init
  container.

**Effort**: 3.1+3.2 ~2 days; 3.3 ~3 days (image changes dominate). **Risk**: 3.3 is
the risky one (permission regressions in user workloads) — gate behind config and roll
out per-environment.

---

## Phase 4 — Reconciliation backstop (longer term)

**Problem.** There is no desired-vs-actual convergence loop. The watcher is
event-driven and amnesiac across backend restarts (in-memory `pod_states`); provisioning
is imperative one-shots. Out-of-band damage (Deployment deleted manually, watch events
missed during reconnect) is never repaired. Pods stuck `Pending`/`ContainerCreating`
emit `scheduling_blocked`/`startup_slow` events but nothing remediates.

**Fix.** Extend the (already leader-elected) reaper into a slow reconcile pass, rather
than adopting kube-rs `Controller`/CRDs:

- Every cycle: LIST all `moatless.io/task-id`-labeled Deployments (one call, already
  done for idle-stop) and diff against DB expectations:
  - Task open + turn active, but Deployment missing → re-provision (bounded retries,
    then surface `environment_error`).
  - Deployment present, task deleted → cleanup (catches failed `cleanup()` runs).
  - Orphaned PVCs/Services/IngressRoutes (see 2.2) → GC.
  - Pods `Pending` past a hard deadline (e.g. 15 min) → scale to 0 + `environment_error`
    (today they linger forever).
- The watcher stays exactly as-is — it's the fast path; the reconcile pass is the
  slow correctness net. This is the standard "edge-triggered with level-based backup"
  controller shape, without adding CRD machinery the system doesn't need.

**Effort**: ~1 week. **Risk**: medium — re-provisioning logic must be conservative
(never fight an in-flight user stop); gate each remediation behind config initially.

---

## Explicitly out of scope / rejected

- **Replacing Redis Streams / SSE with polling or a message broker** — the consumer-group
  setup is idiomatic and at-least-once; the weakness was the first hop (1.1) and
  ownership of state writes (1.2), not the transport.
- **kube-rs `Controller` + per-task CRD** — the Deployment-per-task model with a slow
  reconcile pass (Phase 4) gets the same guarantees without a new API surface.
- **Removing the pod watcher's in-memory state** — acceptable once Phase 4 exists;
  persisting `pod_states` would add complexity for purely cosmetic gaps.

## Suggested sequencing

| Order | Item | Why first |
|---|---|---|
| 1 | 2.1, 2.4 (bug batch) | Small, independent, immediate risk reduction |
| 2 | 1.1 (sandbox outbox) | Biggest single reliability win; unblocks trust in events |
| 3 | 1.3 (events from Postgres) | Small, user-visible history fix |
| 4 | 2.2 (ownerReferences + orphan GC) | Stops resource/cost leaks |
| 5 | 1.2 (transactional writes + `task_turns`) | Depends on nothing above but benefits from 1.1 being in place |
| 6 | 2.3 (liveness), 3.1–3.2 (secrets) | Config-gated rollouts |
| 7 | 3.3 (non-root), Phase 4 (reconcile pass) | Larger, cross-cutting |
