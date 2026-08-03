# Fleet sandbox diagnostics for T3 Code

> **Status:** approved
> **Updated:** 2026-08-02
> **Vocabulary:** CONTEXT.md
> **Sibling spec:** `.plans/backend-otel-tracing.md` (independent; ships separately)

## Current solution

T3 Code's `/settings/diagnostics` page describes the *local T3 server process* — its
child agent processes and its NDJSON trace files. None of that exists on a Moatless
backend, so the fork gates the whole page off:

- `t3code/apps/web/src/fork/features.ts:71` — `diagnostics: false`. This hides the
  settings section and redirects the typed URL (`FEATURE_BY_SETTINGS_PATH`,
  `features.ts:95`).
- `t3code/apps/web/src/routes/settings.diagnostics.tsx` → `DiagnosticsSettingsPanel`
  (`apps/web/src/components/settings/DiagnosticsSettings.tsx:811`).

Moatless already computes rich **per-Sandbox** diagnostics, but only over REST and only
one Task at a time:

- `SandboxStatusFullResponse` (`backend/src/sandbox/schemas.rs:131`) — carries
  `containerDiagnostics[]` with per-container `restartCount`, state, and
  `cpuMillicores`/`memoryMb` (`schemas.rs:63-78`), plus `runtimeEvents`, `servers`,
  `sandboxStatus`, image fields.
- Served at `GET /api/sandbox/v1/tasks/{task_id}/status` (`backend/src/sandbox/handlers.rs:99`),
  assembled by `StatusResolver::resolve_sandbox_status` (K8s,
  `backend/src/sandbox/k8s/status_resolver.rs`) / `DockerStatusResolver` (Docker).
- Per-pod CPU/mem read: `pod_reader::fetch_container_resource_usage`
  (`backend/src/sandbox/k8s/pod_reader.rs:311`) — resolves the pod by label
  `moatless.io/task-id={task_id}`, then `GET /apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods/{pod}`.
  Best-effort: 403/404 → empty map, never an error (`pod_reader.rs:346-365`). Parse
  helpers `parse_cpu_to_millicores`/`parse_memory_to_bytes` (`pod_reader.rs:387-419`).

**What the code does not say:**

- **There is no fleet read today.** The only fleet-wide K8s call is
  `deployment::list_sandbox_deployment_states` (`backend/src/sandbox/k8s/deployment.rs:497`),
  used by `K8sFleetObserver::observe_fleet` (`backend/src/sandbox/lifecycle/observe.rs:39`)
  for reconciliation. It lists sandbox **Deployments** by label-key-present
  (`ListParams::default().labels("moatless.io/task-id")`) and returns only
  `{task_id, desired_replicas, ready_replicas, operation_generation}` — **no CPU/mem**,
  and it is not exposed to any read API.
- **Per-pod metrics is one-task-at-a-time.** No code lists every sandbox pod and fetches
  its metrics. The metrics.k8s.io API *does* support a namespace-wide pods listing
  (`GET /apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods`) — this codebase never calls
  that shape.
- **Any replica can read K8s.** `K8sClient` (`backend/src/sandbox/k8s/client.rs:8`) is
  built once per process (`main.rs:155`) on every replica; leader election
  (`k8s/leader_election.rs`) gates only the reconciler tick, not reads. A new read-only
  diagnostics method may run on any replica.
- **The UI-RPC layer is a single `match tag` dispatch**, not a router table:
  `ProtocolSession::handle_request` (`backend/src/ui_rpc/mod.rs:1448`), arms added around
  `mod.rs:1488`. Unknown tags fall through to `unsupported_exit(&id, &tag)`
  (`mod.rs:2241`). Scope is checked first via `auth::required_scope` (`backend/src/ui_rpc/auth.rs`);
  a tag with no scope entry is refused before the match (`mod.rs:1475-1486`).
- **`UiRpcState` holds only task-scoped seams** (`backend/src/ui_rpc/mod.rs:222-257`):
  `sandboxes: Arc<dyn SandboxConnector>` and `servers: ServersState` wrapping
  `observer: Arc<dyn SandboxObserver>` (`backend/src/sandbox/access.rs:40-113`). There is
  no fleet/list handle and no `K8sClient` on the struct. `K8sFleetObserver` is built in
  `main.rs:173` but only handed to the reconciler.
- **The two existing Sandbox UI-RPC methods discard the rich fields.** `servers::list`
  (`backend/src/ui_rpc/servers.rs:110`) keeps only `status.servers` + `sandbox_status`;
  `servers::sandbox_status` (`servers.rs:133`) keeps only `sandboxStatus`. Both are
  addressed by `threadId` (= `task_id`), per-Task.
- **T3 addresses formerly env-scoped methods with a deployment-wide `environmentId`.**
  The Moatless deployment is one T3 Environment (`moatless-<hash>`, `mod.rs:406`); it is
  not a per-Task selector (design decided in `t3code/.plans/moatless-convergence.md §2.2`,
  Option B). A fleet diagnostics method therefore takes **no** `threadId` — it is
  deployment-global.

## Planned solution

A deployment-global diagnostics page in T3 that lists **every Sandbox** with its status
and live resource usage, backed by one new fleet read exposed over UI-RPC. No per-Task
panel; no node-level metrics; read-only.

**New**
- `ClusterDiagnostics` read-only seam on `UiRpcState` (`backend/src/ui_rpc/`), with a K8s
  implementation (fleet pod list + namespace-wide metrics) and a Docker no-op.
- `sandbox.listDiagnostics` UI-RPC arm (env-scoped, read scope) in `backend/src/ui_rpc/mod.rs`.
- The contract for it in `t3code/packages/contracts/src/{rpc.ts,server.ts}` and a
  `sandboxDiagnostics` capability flag.
- A T3 component that renders the fleet list + aggregate usage on the diagnostics page,
  plus the RPC atom in `t3code/apps/web/src/state/`.

**Changes materially**
- `t3code/apps/web/src/fork/features.ts` — `diagnostics` flag removed (turning the
  feature on = deleting the flag and its gates, per the fork convention in that file).
- `DiagnosticsSettings.tsx` — the four T3 *trace* panels and the process/resource-monitor
  sections are removed; the page renders only the sandbox fleet component.

**Untouched**
- REST `/api/sandbox/v1/tasks/{id}/status` and `moat tasks diagnose` — unchanged.
- Sandbox lifecycle, the reconciler, and the leader election.
- Node-level metrics and any cluster RBAC beyond the pod-metrics grant that per-Task
  metrics already rely on.
- The per-Task `servers.list` / `sandbox.status` UI-RPC methods.

### Problem

An operator running many Tasks on a Moatless deployment has no single place to see the
fleet: which Sandboxes are running, how much CPU/memory each pod is using, and which are
restarting or erroring. The data exists per-Task over REST, but a T3 user cannot reach it
at all — the diagnostics page is blank.

### Behaviour

- **As a T3 user**, when I open `/settings/diagnostics`, I see a table of every Sandbox
  in the deployment: Task identifier/title, sandbox status, node (if available), per-pod
  CPU (millicores) and memory (MB), and restart count — plus an aggregate header (running
  count, total CPU, total memory). It refreshes on demand.
- **As a T3 user on a Docker-mode deployment**, the page loads and shows an empty/"not
  available in this runtime" state rather than an error.
- **When the metrics API is unavailable** (no metrics-server, RBAC missing), rows still
  render with status/restarts and blank CPU/memory, not an error.

### Architecture and key flows

- **`ClusterDiagnostics` seam** (new, `backend/src/sandbox/access.rs` alongside the
  existing read-only seams). One method: `list_sandbox_diagnostics() -> Vec<SandboxDiagnosticsEntry>`.
  Answer: "the status and usage of every Sandbox in this deployment, right now."
  - *K8s impl* (`backend/src/sandbox/k8s/`, reusing `K8sClient`): list pods by
    `moatless.io/task-id` present → set of `{task_id, pod_name, node_name, phase,
    container states + restart counts}`; then one `GET /apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods`
    (namespace-wide) and join by pod/container name, reusing `parse_cpu_to_millicores`/
    `parse_memory_to_bytes`. Metrics absent → entries with `None` usage. This is the new
    combinator the "what the code does not say" section flags as missing; model it on
    `pod_reader.rs` (per-pod) + `deployment.rs:497` (fleet list) patterns.
  - *Docker impl*: returns an empty vec (no node/pod-metrics concept), branching on
    `config.sandbox_provider` the way `SandboxObserver`/`FleetObserver` already do in
    `main.rs`.
  - Constructed once in `main.rs` alongside `K8sSandboxAdapter`, threaded through
    `UiRpcDependencies` → `UiRpcState`.
- **`sandbox.listDiagnostics` arm** (`backend/src/ui_rpc/mod.rs`, new match arm near the
  other `sandbox.*`/`servers.*` arms ~`mod.rs:1757-1789`): calls the seam, maps
  `Vec<SandboxDiagnosticsEntry>` to the T3 result JSON. Add its scope to
  `auth::required_scope`/`GRANTED_SCOPES` (`backend/src/ui_rpc/auth.rs`) as an
  orchestration-read scope (same class as `servers.list`).
- **T3 side**: add the RPC to `packages/contracts/src/rpc.ts` (`WS_METHODS`,
  `Rpc.make`, add to `WsRpcGroup`) and the result schema to `packages/contracts/src/server.ts`;
  add the query atom in `apps/web/src/state/server.ts` (mirror `processDiagnostics` at
  `packages/client-runtime/src/state/server.ts:668`, but with no `environmentId`-derived
  per-task input — it is env-scoped like the config subscription); render it in a new
  component used by `DiagnosticsSettings.tsx`.

Main flow: T3 opens the page → env-scoped `sandbox.listDiagnostics` over the existing
`/ws` socket → any backend replica → `ClusterDiagnostics::list_sandbox_diagnostics` →
K8s fleet pod list + one metrics read → mapped result → table + aggregate header.

### Data model

No persisted state. The result is a transient read. Define the wire entry (produced by
the seam, serialized by the RPC arm and mirrored in the T3 contract):

`SandboxDiagnosticsEntry`
| field | type | note |
|---|---|---|
| `taskId` | string | the Moatless `task_id` (T3 `threadId`) |
| `title` | string \| null | Task title/name for display, if cheaply available |
| `sandboxStatus` | string | reuse `SandboxStatus` values (`live_status.rs`) |
| `nodeName` | string \| null | pod's scheduled node, if known |
| `containers` | `{ name, state, restartCount, cpuMillicores?, memoryMb? }[]` | mirror `ContainerDiagnostics` (`schemas.rs:63`) |
| `totalCpuMillicores` | number \| null | sum across containers, null if metrics absent |
| `totalMemoryMb` | number \| null | sum across containers |

Follow the existing camelCase serde convention (`schemas.rs` uses
`#[serde(rename_all = "camelCase")]`). Encode optional fields to match whatever the T3
contract schema declares (plain nullable is fine here — this is a new Moatless-authored
result, so it need not use Effect's `{_tag}` Option encoding unless the T3 schema is
written with `Schema.Option`; write the T3 schema with nullable fields to keep the Rust
side simple).

### Interfaces and contracts

- **External contract (new):** `sandbox.listDiagnostics` UI-RPC method and the
  `sandboxDiagnostics` capability flag. Consumed by the T3 diagnostics page. Absent
  capability ⇒ unsupported, per the T3 contract's optional-capability rule; a stock T3
  server keeps working.
- **Internal interface (new, not a promise):** the `ClusterDiagnostics` trait. It exists
  because `UiRpcState` must not be handed the full K8s client or the lifecycle-mutation
  surface — it mirrors the read-only-seam boundary documented in
  `backend/src/sandbox/access.rs`. Single implementation per runtime; no speculative
  methods (per repo `AGENTS.md`: no trait with a single impl unless a test uses the mock —
  here the Docker + K8s impls plus the RPC test justify it).

### Failure, safety and recovery

| When | What the person sees | Wording fixed? |
|---|---|---|
| Metrics API 403/404 | Rows render with status + restarts, blank CPU/memory | no — reuse existing best-effort behaviour |
| Pod list fails (K8s API error) | The RPC returns a typed failure the T3 page shows as an error banner | no |
| Docker runtime | Empty state, not an error | no |

No writes, so no partial-write or rollback concerns. The read is idempotent and safe to
retry. A slow metrics read must not block the pod list — fetch the list first, then
overlay metrics best-effort (as `status_resolver.rs` already does per-Task).

### Testing and verification

- **Rust:** unit-test the K8s mapping (pod list + metrics JSON → `SandboxDiagnosticsEntry`)
  with fixture responses, including the metrics-absent path. Test the Docker impl returns
  empty. Add a `ui_rpc` test that the `sandbox.listDiagnostics` arm requires the read
  scope and returns the mapped shape (mirror existing `servers.*` ui_rpc tests). Run
  `cargo clippy -p moatless-vibe-backend --all-targets -- -D warnings` and
  `cargo test -p moatless-vibe-backend`; if `backend/tests/` is touched,
  `task test:integration`.
- **T3:** a component test rendering the table from a mocked RPC result (follow the
  frontend testing guidelines); `bun run type:check` + `bun run biome:check` in t3code.
- **Manual:** against the k8s dev cluster, open the repurposed page with several running
  Sandboxes and confirm the list + aggregate header populate and refresh.
- **Acceptance:** Given a deployment with ≥1 running Sandbox, When a T3 user opens
  `/settings/diagnostics`, Then every Sandbox appears with status and (when metrics-server
  is present) CPU/memory, and the page is reachable (the feature gate is gone).

## Out of scope

- Node-level cluster metrics and any new cluster-scoped RBAC.
- Per-Task drill-down panels (convergence Move 4).
- Live streaming/subscription of the fleet (one-shot query + manual refresh only).
- Any write action (stop/restart/kill) from the page.
- Backend request tracing — that is `.plans/backend-otel-tracing.md`.
