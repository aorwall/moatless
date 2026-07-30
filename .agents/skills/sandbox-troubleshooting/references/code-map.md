# Code map — symptom → source

Read `docs/sandbox-lifecycle.md` first (canonical state machine, env-var config
table). `docs/sandbox-architecture.md` covers inside-the-pod details (startup
sequence, gRPC API, preview-server state machine).

> Note: `kubernetes-testing/references/metrics.md` describes a legacy
> TypeScript implementation (`apps/backend`, `apps/sandbox`, metric names like
> `moatless_kubernetes_pod_total`). The current code is Rust (`backend/`,
> `sandbox/`); use the verified metric names in the Metrics section.

## Backend — lifecycle & status

| Concern | File | What lives there |
|---|---|---|
| Pod failure / eviction / OOM classification | `backend/src/sandbox/k8s/pod_watcher.rs` | `detect_transitions()`, spot-reclaim vs fatal (`is_node_removal_reason`), OOM/crash-loop, dedup, `gc_failed_pods`, `sweep_pending_pods` |
| Status/diagnose engine | `backend/src/sandbox/k8s/status_resolver.rs` | `resolve_sandbox_status`, container diagnostics, runtime events, live gRPC agent state |
| Pure pod-status helpers | `backend/src/sandbox/k8s/pod_status.rs` | `is_sandbox_container_ready`, `check_sandbox_permanent_failure`, `pick_best_pod`, container status extraction |
| Naming | `backend/src/sandbox/k8s/naming.rs` | `moatless-ws-{task_id}` sanitize/truncate rules |
| Reaper (idle-stop, inactive-close, orphan-turn, reconcile, orphan GC) | `backend/src/task/reaper.rs` | leader-elected; `idle_reaped`, reconcile remediations |
| Resource CRUD | `backend/src/sandbox/k8s/{deployment,service,pvc,ingress}.rs` | scale (`scale_deployment`), idempotent create/delete |
| Storage backend selection | `backend/src/sandbox/k8s/storage.rs` | `Ebs` / `Ephemeral` / `S3` |
| Pod spec / volumes / probes | `backend/src/sandbox/k8s/pod_builder.rs` | emptyDir `sizeLimit`s, startup/liveness probe wiring |
| Resource-limit defaults | `backend/src/config.rs` + `k8s/helm/moatless-vibe/values.yaml` | `K8S_RESOURCE_LIMIT_MEMORY`, size knobs |
| HTTP endpoints | `backend/src/sandbox/{routes,handlers}.rs` | `/status`, `/restart`, `/redeploy`, `/servers/{name}/logs` |

## Sandbox (in-pod)

| Concern | File | What lives there |
|---|---|---|
| Health/liveness endpoints | `sandbox/src/main.rs` | `GET /health` (startup probe, 200 iff Ready), `GET /livez` (heartbeat-backed liveness) |
| Agent state machine | `sandbox/src/agent/agent_state.rs` | agent states surfaced via gRPC `GetAgentStatus` |
| Server monitoring | `sandbox/src/server/` | preview/files server process + status |

## Event types (`crates/events/src/lib.rs`)

- `SandboxStatus`: `not_created | initializing | ready | stopped | removing | removed | error`
- `ServerSubType`: `installing | starting | started | stopped | failed`
- `AgentSubType`: `... | error | recovery_exhausted | activity | ...` — `is_error()` ⇒ `error`, `recovery_exhausted`
- `TaskTurnSubType`: `active | completed | error | stopped | turn_interrupted`
- Sandbox `payload.lifecycleEvent` values to grep (event JSON key is camelCase;
  values are snake_case): `container_oom_killed`, `container_crash_loop`,
  `pod_evicted`, `pod_failed`, `node_reclaimed`, `idle_reaped`, `environment_error`.
  (The Prometheus label for the same field is snake_case — `lifecycle_event` — see below.)

## Prometheus metrics (verified names, `GET /metrics`)

```
sandbox_container_terminations_total{lifecycle_event,reason,exit_code}   # OOM/crash/fatal terminations
moatless_sandbox_startup_seconds                                         # pod creation -> sandbox Ready
moatless_sandbox_provision_duration_seconds{storage_backend,outcome}
moatless_sandbox_restart_duration_seconds{storage_backend}
task_reconcile_remediations_total{type,outcome}                          # reprovision / pending-scale-zero
task_turns_reconciled_total                                              # force-closed orphaned turns
moatless_workspace_sync_{total,bytes,duration_seconds}                   # S3 storage backend only
```

## Related skills

- `moat-cli` — full `moat` command reference.
- `kubernetes-testing` — creating/monitoring vibe tasks in K8s, integration tests.
- `rust-dev-guidelines` — Rust conventions for backend, sandbox, and shared-crate
  code.
