# Sandbox Lifecycle Architecture

**Status:** Accepted 2026-07-21 · implementation in progress. Order-of-work
step 1 — the lifecycle module foundation — is on PR #154; steps 2–4 remain. An
ADR for the ordering/serialization choices ships with the implementation PR.

A deep `sandbox::lifecycle` module owns durable lifecycle intent and
orchestration for a Task's Sandbox. Kubernetes and Docker are infrastructure
adapters behind the `SandboxRuntime` seam.

## Decision

Callers identify a Task and request a lifecycle transition. The module resolves
the Task and Workspace, persists intent, constructs a fresh runtime plan when
needed, invokes the selected runtime adapter, records bookkeeping, publishes the
operation-level event, and returns a typed outcome. Callers never supply a
`ProvisionRequest`, repositories, event publishers, or the runtime adapter.

- **Locality** — intent, ordering, and failure rules live in one module.
- **Leverage** — HTTP, Task, Message, Loop, Adapter, Reaper, and watcher-healing
  callers share one behavior.
- **Test surface** — tests cross the same interface as callers.

## Seams

Two traits, both implemented by the K8s and Docker adapters:

- **`SandboxRuntime`** (mutations): `ensure_running`, `stop`, `restart`,
  `redeploy`, `remove` — every mutation returns a typed `RuntimeOutcome` so
  lifecycle never infers what happened through a second status read. Held
  **only** by the lifecycle module: a **task-scoped** lifecycle mutation that
  bypasses lifecycle is a compile error, not a review comment. The one named
  exception is the **orphan resource sweep** — label-based deletion of
  resources whose Task row no longer exists, which lifecycle cannot express. A
  recording test adapter implements this trait for lifecycle module tests.
- **`SandboxAccess`** (interior access + observation): `sandbox_status`, `live_status`, `container_statuses`,
  `expected_sandbox_image`, `server_logs`, `sandbox_client`. Lives in `AppState`
  for status handlers, file operations, notify, and the reaper's eligibility
  reads. Not read-only: `sandbox_client` grants a gRPC client that mutates the
  sandbox's interior (agent, files). The seam line is infrastructure state
  transitions (lifecycle only) vs. interior access and observation — consistent
  with Agent/Turn lifecycle sitting outside the module. These callers are
  unaffected by the lifecycle migration. `SandboxAccess` is deliberate
  interface segregation — a capability restriction, not a deep module; the
  depth claim in this document applies to `sandbox::lifecycle`.

Repository seams stay internal to the lifecycle implementation.

## Interface (illustrative)

```rust
pub struct Lifecycle { /* private dependencies */ }

impl Lifecycle {
    pub async fn start(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError>;
    pub async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<LifecycleOutcome, LifecycleError>;
    pub async fn restart(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError>;
    pub async fn redeploy(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError>;
    pub async fn remove(&self, task_id: &str, retention: ArchiveRetention) -> Result<LifecycleOutcome, LifecycleError>;
}
```

Typed surface requirements:

- `LifecycleOutcome`: the common result of every operation, with **two
  independent effect axes** — `intent_effect: IntentEffect` (`Changed` |
  `Unchanged`, from the intent write; `Changed` covers any real intent change:
  the desired-state enum, stop reason, or archive retention — not only enum
  transitions), `runtime_effect: RuntimeEffect` (`Applied` | `NoOp`, from the
  adapter), an observed status (`Ready` | `Starting` | `Stopped` | `Removed`),
  and success diagnostics (degradation warnings, per-server failures). The
  axes are independent: a stop against already-stopped compute is `Changed` +
  `NoOp` — the intent change is real and publishable even though
  infrastructure did nothing. Adapters legitimately differ:
  Docker completes transitions synchronously and reports `Ready`; Kubernetes
  returns `Starting` and readiness arrives through observation.
- `ArchiveRetention::{RetainArchives, DeleteArchives}`: typed policy for S3
  archives replacing the bare `retain_storage: bool`; runtime resources are
  always removed. Persisted alongside `Removed` intent in the same atomic write,
  so a crashed remove can be retried with the original policy.
- `LifecycleError`: classifiable, not stringly — the reconciler's attempt cap and
  the schedule runner's failure metrics need error classes; HTTP callers map to
  strict failures, background callers to best-effort logging. Includes
  `OperationInProgress { kind }` for per-Task lock contention — not "Busy",
  which already means agent/process activity here; carrying the conflicting
  operation's kind lets callers react without a marker read (HTTP → 409,
  background → bounded latency retry or return to the reconciler).
- `RuntimeOperationContext { operation_generation }`: passed to **every**
  `SandboxRuntime` mutation so the generation reaches the runtime. Kubernetes
  stamps the pod-template label — restart patches the template while preserving
  the Deployment's `Recreate` strategy, so the old pod terminates before the
  replacement pod is scheduled and the replacement carries the new generation.
  Docker **restart recreates the sandbox container** (strict remove + create
  from the same config, new container ID, new label) —
  labels are immutable on a live container, so in-place restart could never
  carry identity; workspace survives on the host mount, in-container tmpfs is
  lost, same as a pod restart. Docker server containers remain subordinate
  runtime resources: their events are excluded from Sandbox lifecycle
  observation by filtering the watcher to `moatless.service=sandbox`; server
  failures remain visible through container-status diagnostics.
- `RuntimeOutcome`: returned by **every** `SandboxRuntime` mutation — observed
  status, `RuntimeEffect::{Applied, NoOp}`, and success diagnostics
  (degradation warnings such as PVC failure → ephemeral-storage fallback,
  per-server failures). The effect must come from the adapter: on retries
  intent is already persisted, so lifecycle cannot derive "did anything
  change" from intent, and it must not infer it through an extra status read.
  **Failure diagnostics belong to `RuntimeError`/`LifecycleError`** — the
  success type never describes failures. Adapters have no event publisher.

`ensure_running` names what the runtime start actually does: reuse Ready, wait
for Initializing, scale up Stopped, rebuild on image change, create missing,
heal network resources, and replace a terminal/error pod from the fresh plan.
An existing failed pod is not reported as indefinitely `Initializing`.

## Semantics

| Operation  | Persisted intent          | Runtime effect                 | Outcome            |
| ---------- | ------------------------- | ------------------------------ | ------------------ |
| `start`    | Running, clear stop reason| Ensure running from fresh plan          | Ready or Starting  |
| `stop`     | Stopped + reason          | StopAgent (best-effort), stop compute, preserve storage | Stopped |
| `restart`  | Running, clear reason     | Restart current deployment              | Ready or Starting  |
| `redeploy` | Running, clear reason     | Replace from fresh plan                 | Ready or Starting  |
| `remove`   | Removed + reason + retention | StopAgent (best-effort), delete runtime resources | Removed |

**Graceful stop is lifecycle's ordering rule:** `stop` and `remove` issue a
best-effort `StopAgent` through a narrow internal agent-stop port before the
runtime effect, so the pod exits promptly instead of riding termination grace
into SIGKILL. Failures never block the transition. This applies to every
caller, including HTTP stop.

Operation eligibility by Task status — the Closed guard is per-operation, not
universal, because close flows persist `Closed` before removing the sandbox:

| Task status | `start`      | `restart` | `redeploy` | `stop`         | `remove` |
| ----------- | ------------ | --------- | ---------- | -------------- | -------- |
| Open        | ✓            | ✓         | ✓          | ✓              | ✓        |
| Error       | ✓ (resumable)| ✓         | ✓          | ✓              | ✓        |
| Closed      | rejected     | rejected  | rejected   | ✓ (idempotent) | ✓        |

Intent precedence inside eligibility: `stop` against a Task whose persisted
intent is already `Removed` is an idempotent no-op — it never rewrites intent
to `Stopped`, which would discard the archive-deletion obligation the `Removed`
intent and its retention carry.

- **Intent before effect, for all five operations.** If the runtime effect fails,
  intent remains the requested target and the caller receives the typed error.
  Asynchronous visibility and retry are per operation (matrix below).
- **Intent writes are atomic, and carry the correlation marker.** Desired state,
  stop reason, retention policy, `sandbox_desired_state_updated_at`, and the
  **operation correlation marker** (`kind` + `operation_generation` +
  `started_at` — "operation_generation" to avoid colliding with Kubernetes
  resource generations) change
  in one statement (or one transaction) — readers never observe partial
  intent. The marker outlives the adapter call: Kubernetes returns `Starting`
  before readiness and Docker events arrive asynchronously, so clearing on
  return would misclassify late events. The **`operation_generation` is
  stamped onto the runtime resources it creates** (K8s pod-template label,
  Docker container label) and watchers carry the observed generation on every
  event —
  Docker restart reuses the container name, so identity needs the label, not
  the name. A newly requested operation allocates a generation; recovery of an
  incomplete operation reuses its marker generation rather than minting a new
  one, so attempt accounting and event identity remain stable across retries.
  Acknowledgement is per operation class:
  - **start** clears after observed Ready;
  - **stop** clears after observed convergence **and** durable persistence of
    its deterministic `Stopped` completion event;
  - **remove** clears only after strict success **and** durable persistence of
    its deterministic `Removed` completion event — observed absence is not
    enough, because the archive-deletion obligation has no observable trace, so
    a persisting remove marker with desired `Removed` tells the reconciler to
    retry `remove` even when no resources are live;
  - **edge operations** (restart/redeploy) clear only when the **replacement
    generation** is observed Ready — desired==observed alone would acknowledge
    the edge before the adapter did anything, since desired is already Running
    and the superseded pod may still be Ready;
  - an operation completing `Unchanged` + `NoOp` against an already-converged
    target clears its marker synchronously — no completion fact will ever
    exist for its generation, so waiting for one would strand the marker.
  Completion facts are persisted and the marker cleared in **one DB transaction
  before publication**; a failed completion commit is a typed error — the
  operation does not report success and nothing is published, so a
  deterministic event ID is never live without its persisted fact.
  Suppression proves an event belongs to a superseded generation (observed
  generation < marker generation). Operation and convergence event IDs are
  deterministic (`sandbox:{task_id}:{operation_generation}:{fact}`) and use the
  existing transactional-outbox/idempotent-persistence path. The reconciler can
  therefore persist a missing completion fact and clear its marker in one DB
  transaction after a backend crash, without duplicating the event. (The
  current DAO issues one UPDATE per patched field; the lifecycle implementation
  must not.)
- **The Sandbox reconciler — the single async recovery owner, for level
  states only.** A runtime-neutral, durable level-based controller
  (lease-elected on Kubernetes, lease-free single-replica in Docker) compares
  persisted intent against observed status and converges through lifecycle
  operations, never directly:
  - desired Running, observed stopped/missing → `lifecycle.start` (guarded by
    recent-explicit-stop and the attempt cap);
  - desired Stopped, any observed live status other than Stopped →
    `lifecycle.stop`;
  - desired Removed, **live resources or an incomplete remove marker** →
    `lifecycle.remove` with the persisted retention, once
    `sandbox_desired_state_updated_at` exceeds the grace window (replacing
    `decide_orphan`'s Removed-intent and Closed-retention concerns; its
    taskless branch survives as the orphan resource sweep). The marker branch is
    load-bearing when runtime resources are absent but archive deletion has not
    completed.
  The reconciler absorbs today's mid-turn reconcile pass (the original naming
  objection — "reconciler collides with the reaper's pass" — dissolves with
  it); the idle reaper remains but is not a recovery owner. The reconciler is
  also the correlation marker's **acknowledgement owner** (per-class rules
  above). Marker age is a recovery signal, never proof of completion: stale
  level-operation markers are reconciled and **never cleared merely because of
  age**; in particular, an incomplete remove marker survives until strict
  success. A stale edge marker first disables event suppression, then is cleared
  only after the replacement generation is observed Ready or the edge is
  durably classified as abandoned with its deterministic `Error` fact
  persisted. Its
  **leader-local attempt cap** (deliberately not called a durable budget — it
  resets on leader failover, accepted looseness) uses exponential backoff,
  retries only transient error classes (runtime/infra), never permanent ones
  (bad plan, Closed). It resets when a new caller-requested operation generation
  is accepted or a newly observed, causally eligible pending message requests a
  wake; reconciler retries that reuse the same generation do not reset it.
  Observed-convergence map, the one normalized definition the reconciler
  uses (it compares `SandboxDesiredState` against `SandboxObservedStatus` —
  never bare "state"): desired **Running** ↔ observed `Ready`; desired **Stopped** ↔
  observed `Stopped` **or absent** (a missing runtime satisfies Stopped);
  desired **Removed** ↔ **every runtime resource absent, PVC unconditionally
  included** (`ArchiveRetention` governs S3 archives only, never PVCs) —
  accepted-but-still-terminating is not converged. Non-converged states all
  have a rule: desired Running + observed `Starting` → wait, remediated by
  the pending-timeout pass after its deadline; desired Running + observed
  error/crash → retry start under the attempt cap, and `ensure_running`
  replaces the terminal/error pod rather than treating it as Initializing;
  desired Stopped + Starting/Ready/error → stop; desired Removed + any live
  status → remove. There is no unmapped pairing. Archive deletion is part of
  strict remove *success*, not of observed convergence — the remove marker
  carries that obligation (below). Resources whose Task row is gone are the
  **orphan resource sweep** (direct label-based
  deletion) — "orphan" is reserved for exactly this taskless case; lifecycle
  requires a Task, so that seam is explicitly administrative and outside the
  module.
- **Restart and redeploy are caller-owned edges.** They are synchronous,
  edge-triggered operations with no async replay: a failure where nothing
  changed surfaces the typed error to an owner that retries — the HTTP user,
  and the auto-disable healer, whose fire-once guard **must clear on redeploy
  failure** with a bounded attempt count (today the guard is sticky regardless
  of outcome, so the healer would otherwise never retry). A failure after
  teardown leaves desired Running with the deployment missing, which the
  reconciler converges via a fresh-plan start — completing the redeploy's
  intent. The reconciler never replays an edge operation. **Edge teardown is
  strict in both adapters**: a redeploy or Docker recreate-restart never
  proceeds to provision past a failed delete — Kubernetes already fails hard;
  Docker's force-remove
  becomes load-bearing (today it is swallowed, letting provision reuse the
  still-running old container and report success without applying the fresh
  plan).
- **Per-operation recovery matrix:**

  | Operation  | Failure visibility                                  | Async retry owner                                        |
  | ---------- | --------------------------------------------------- | -------------------------------------------------------- |
  | `start`    | Live status: desired Running vs. Stopped/NotCreated | Sandbox reconciler                                       |
  | `stop`     | Live status: desired Stopped vs. still running (new)| Sandbox reconciler                                       |
  | `restart`  | Synchronous typed error                             | None — caller-owned edge; a torn-down deployment converges to Running |
  | `redeploy` | Synchronous typed error                             | None — caller-owned edge; a torn-down deployment converges to Running |
  | `remove`   | `Removed` intent + live resources or incomplete marker | Sandbox reconciler, persisted retention and generation |

- **Permanent deletion requires successful removal.** Task deletion is
  `lifecycle.remove` with `ArchiveRetention::DeleteArchives`, followed by row
  deletion only on success. On failure the typed error surfaces, the row
  survives with `Removed` intent and its persisted policy, the reconciler keeps
  retrying, and the delete is idempotent to re-attempt. The Task row is the
  durable retry record — it is never deleted ahead of the infrastructure and
  archives it tracks. **Removal is strict:** every
  deletion is load-bearing — Deployment and PVC (already strict today), S3
  archive deletion (today logged-and-swallowed in
  `delete_task_transcripts`), and Docker force-removal — so a "successful"
  remove cannot leave invisible archives behind. **Strict success means
  observed absence, not accepted deletion:** Kubernetes delete calls return
  when the request is accepted, so `remove` awaits actual absence of every
  resource (as `wait_for_pods_deleted` already does for pods) before it
  reports success, and Task-row deletion is permitted only after that.
- **Serialization:** lifecycle operations are serialized per Task across backend
  replicas with `pg_try_advisory_lock` on a **dedicated lock-only connection
  pool**, bounded retry, and the connection released between attempts;
  exhausted retries surface `LifecycleError::OperationInProgress { kind }`.
  The holder keeps its lock connection for the operation's duration; lifecycle
  DB writes go through the main pool. RAII rule: a lock connection whose unlock fails, or that drops
  mid-operation, is closed — never returned to its pool — so the session lock
  dies with the connection. Blocking `pg_advisory_lock` on the shared pool and
  the xact-scoped variant are both forbidden: the former can starve the
  (default five-connection) pool, the latter holds a DB transaction open across
  runtime effects that can wait tens of seconds. This design **is** a durable,
  level-based controller (persisted desired state, markers, a reconciler);
  what stays excluded is a durable operation **queue** with exactly-once
  execution — operations are at-least-once with idempotent effects. Every
  operation takes the lock — there is no lock-free fast path (a read-first
  shortcut has no safe linearization point against a concurrent stop/remove;
  an uncontended try-lock is one round-trip). Message-wake durability does
  not rest on in-memory requeues: messages are already durable rows with
  delivery acknowledgement, and **undelivered messages are reconciler input**.
  Causal precedence is explicit: only Open/Error Tasks participate; a pending
  message older than `sandbox_desired_state_updated_at` cannot supersede a
  later stop/remove; a newer message may supersede Stopped, and may supersede
  Removed only for an Open/Error Task whose removal retention is
  `RetainArchives` (the durable explicit wake after a soft removal) — an
  in-progress `DeleteArchives` removal is never superseded, deletion wins;
  Closed is never resurrected. The reconciler compares DB-owned
  Message `id`/`created_at` with the DB-owned intent timestamp, avoiding process
  clocks and user-supplied timestamps. An eligible Task is converged to Running
  and, while Ready, re-notified until the pending rows are acknowledged.
  Notification is idempotent and uses leader-local exponential backoff keyed by
  Task; failover may cause one extra
  notification but cannot lose the durable wake. The in-line handling of
  `OperationInProgress` is only a latency fast path: a toward-Running `kind`
  stops further inline work, while stop/remove gets a bounded inline retry;
  neither path is the durability owner.
- **Cancellation safety:** multi-step transitions run detached from caller
  futures, so an HTTP client disconnect cannot leave a sandbox
  half-transitioned. A severed caller surfaces `LifecycleError::Interrupted`
  while the operation itself runs to completion and reports through events.
- **Events — operation-driven vs. observed, desired state as arbiter:**
  - Lifecycle publishes two event classes with distinct sources.
    **Acceptance events** derive from `IntentEffect::Changed`: `Removing` when
    intent actually moves to `Removed`. **Completion events** (`Stopped` with
    the reason tag, `Removed`) publish when an operation **newly establishes
    convergence** — it succeeds and the Task was not already converged on its
    target. This is derived without a pre-effect status read: final convergence
    plus any of `IntentEffect::Changed`, `RuntimeEffect::Applied`, or an
    incomplete marker proves the operation established the target. Events use
    the deterministic operation-generation ID above, so at-least-once retries
    of one generation persist once. This covers recovery: a first remove that
    persists `Removed` and fails publishes `Error`; the reconciler's retry is
    `Unchanged` + `Applied` yet newly establishes convergence, so it publishes
    `Removed` — intent-change alone would lose that event while the watcher
    suppresses the matching echo. `RuntimeEffect::NoOp` completions still
    publish when they newly converge (stopping already-stopped compute is a
    real stop). Only **infrastructure-effect events** gate on
    `RuntimeEffect::Applied`: `Initializing` when start/restart/redeploy
    actually does work. `Error` is the failure fact for an attempt; recovery of
    the same operation generation may later publish its completion fact.
    `Unchanged` + `NoOp` on an already-converged Task (reusing a
    Ready sandbox on a message-wake) publishes nothing.
  - Lifecycle never publishes `Ready`. Ready is the **watcher's convergence
    event** in both providers — the Docker container watcher and the K8s pod
    watcher already own it.
  - Watchers classify every observation against desired state **and the
    correlation marker**, into exactly one of four cases:
    1. **Convergence** — observed Ready while desired Running: published (the
       one desired-matching observation that is published).
    2. **Operation echo** — a terminal observation matching desired state that
       lifecycle already announced (observed stop while desired Stopped):
       suppressed.
    3. **Expected intermediate** — only an observation the marked
       operation's `kind` declares expected: pod/container deletion and stop
       of the **superseded** pod during restart/redeploy (the pod watcher
       tracks by `pod_name`, so identity — not the clock — distinguishes the
       old pod's death from the new pod's crash): suppressed. OOM kills,
       evictions, and crash exit codes are **never** expected and always
       publish, marker or not.
    4. **Divergence** — everything else (crash, OOM, eviction, external stop
       while desired Running): published.
    Reason tags survive because the operation-driven event carries them
    (`SandboxStopReason::IdleReaper` → `idle_reaped` + initiator).
  - Success diagnostics carried on `LifecycleOutcome` and failure diagnostics
    carried on the error types (degradation warnings, per-server failures) are
    republished by lifecycle, so Docker's per-server failure visibility
    survives. Adapters publish nothing themselves.

## Ownership

| Lifecycle owns                                | Lifecycle excludes                          |
| --------------------------------------------- | ------------------------------------------- |
| Task existence and Closed guard               | HTTP authorization and response mapping     |
| Persisted Sandbox desired state               | Task status Open/Closed/Error               |
| Stop reason and start timestamp               | Reaper eligibility; reconciler attempt cap  |
| Workspace-to-runtime plan construction        | Message notification over gRPC              |
| Fresh plan on start and redeploy              | Agent and Turn lifecycle (beyond the best-effort StopAgent in stop/remove) |
| Intent/effect ordering                        | Observed status projection                  |
| Per-Task operation serialization                | Kubernetes or Docker mechanics              |
| Typed outcomes and failures                   | Watcher detection policy                    |
| Operation-level lifecycle events              | Adapter-specific delivery behavior          |
| Cancellation-safe long transitions            | Orphan resource sweep (taskless resources)  |
| Graceful agent stop inside stop/remove        |                                             |

The lifecycle module owns desired intent and operations. Runtime adapters own
infrastructure effects. Watchers and status modules own observed state.

**Watcher crash remediation** follows the same split: the pod watcher keeps
detection and the disabled-servers decision, then issues its remediation as
lifecycle operations — `lifecycle.redeploy` after auto-disabling a crash-looping
server (a fresh plan picks up `disabled_servers`), `lifecycle.stop` with an
`Error` reason for crash autostop. The watcher performs no direct K8s mutation
and writes no intent fields itself.

## Migration

| Current caller                  | Current knowledge                       | Call after migration      | Caller keeps                                |
| ------------------------------- | --------------------------------------- | ------------------------- | ------------------------------------------- |
| `task/handlers.rs`              | Plan, intent, runtime                   | `lifecycle.start/remove`  | Authorization, Task response                |
| `sandbox/handlers.rs`           | All transitions + `record_sandbox_*`    | `lifecycle.*`             | HTTP mapping                                |
| `message/send.rs`               | Message persistence + direct Running-intent patch | Pending row is reconciler input; no direct intent write | Message validation and persistence |
| `sandbox/notify.rs`             | Plan construction + start               | `lifecycle.start`         | Notify when outcome is Ready                |
| `bindings/runtime.rs`           | Plan + best-effort start                | `lifecycle.start`         | Binding delivery (serves Slack/Telegram)    |
| `triggers/schedule_runner.rs`   | Plan + best-effort start                | `lifecycle.start`         | Loop metrics and routing                    |
| `adapters/linear/agent_runtime.rs` | Plan, start (clone of github_pr)     | `lifecycle.start`         | External subject behavior                   |
| `adapters/github_pr/runtime.rs` | Plan, start, cleanup                    | `lifecycle.start/remove`  | External subject behavior                   |
| `task/reaper.rs`                | Runtime + inline intent patches         | `lifecycle.stop/start/remove` | Eligibility, attempt cap, orphan resource sweep |
| `sandbox/k8s/pod_watcher.rs`    | Direct K8s redeploy + intent patches    | `lifecycle.redeploy/stop` | Crash detection, disabled-servers choice    |

Notes that bound the migration:

- The Closed-task guard is already centralized in `build_provision_request`;
  what the call sites duplicate is the ~15-line build→provision→log wrapper and
  each site's ad-hoc error policy (swallow / swallow+metric / publish-error /
  propagate / retry cap). `sandbox/handlers.rs` contains two plan-build
  sites (provision and redeploy).
- Message-send and agent-event paths reach provisioning through
  `notify.rs::ensure_sandbox_and_notify`; migrating notify migrates their
  latency path. The direct `SandboxDesiredState::Running` patch in
  `message/send.rs` is deleted: lifecycle remains the only intent writer, while
  the durable pending row is the reconciler's recovery input.
- The Message repository gains a runtime-neutral query for Tasks with
  unacknowledged user Messages and a supporting partial index on pending rows;
  reconciliation must not scan Task Messages one Task at a time.
- The failure event `insert_and_provision_task` publishes today moves into
  lifecycle.
- The plan-construction repositories (`repos`, `provider_tokens`,
  `plugin_repos`) move behind lifecycle. `SandboxApiState` keeps `tasks` and
  `teams` (authorization), `workspaces` (status/server-config reads), and
  `secrets`, alongside the lifecycle handle and the access trait.
- The public `/cleanup` HTTP route path is stable; internals rename to
  `remove`.

Order of work:

1. **Lifecycle foundation** (`backend/src/sandbox/lifecycle/` — landed, PR
   #154): the typed surface, the `SandboxRuntime` seam trait with recording
   adapters, the orchestrator over ports (`LifecycleStore`, `LifecycleLock`,
   `LifecycleEventSink`, `AgentStopPort`, `WallClock`), and the pure decision
   modules (`convergence`, `classify`, `marker`, `wake`, `events`, `attempt`),
   unit-tested through the interface and unwired in production. The seam trait
   exists from this step because `SandboxProvider` cannot express
   `RuntimeOutcome` or `RuntimeOperationContext`; step 3 is adapter
   implementation and retirement, not a rename.
2. **Move all mutations** — migrate start, stop, restart, redeploy, and remove
   callers (including watcher healing and the six `stop_agent_for_task` call
   sites, which lifecycle's graceful-stop rule replaces); add the persisted
   retention and correlation-marker columns; thread `RuntimeOperationContext`
   through both adapters; add deterministic operation-event IDs through the
   transactional outbox; restrict Docker lifecycle observation to the sandbox
   container; make `delete_task` require successful `lifecycle.remove` before
   row deletion; add the pending-Message query/index; stand up the
   desired-state reconciler (subsuming the `decide_orphan`
   `Removed`-intent rule) in both providers — lease-elected on K8s, lease-free
   in Docker. The Postgres `LifecycleStore` adapter owns the single-statement
   atomic intent write and the completion-fact + marker-clear transaction; the
   `LifecycleLock` adapter owns the dedicated advisory-lock pool. This step
   activates the Postgres-dependent test cases: the lifecycle contract's
   Postgres half, the race matrix's genuinely concurrent serialization and
   lock-retry exhaustion, the eligibility matrix's Error-resumable row, and
   the start matrix's bad-plan case (plan construction moves behind the
   interface here).
3. **Implement the runtime seam** — the K8s and Docker adapters implement
   `SandboxRuntime` (provision logic becomes `ensure_running`); retire
   `SandboxProvider`; split out `SandboxAccess`.
4. **Delete shallow paths** — plan orchestration and intent helpers in callers
   (`record_sandbox_*`, adapter `provision_task` clones, watcher side door).

Each step lands only its own machinery — no unwired abstractions ahead of
their step beyond the foundation above. Corners discovered during
implementation are PR findings or issues, not spec growth; this document
changes only by deliberate decision.

## Tests

The interface is the test surface:

- **Lifecycle contract:** local Postgres + recording runtime adapter.
- **Start matrix:** Ready, Starting, missing, stopped, Closed, bad plan,
  runtime failure.
- **Eligibility matrix:** Closed rejects start/restart/redeploy, permits
  idempotent stop and remove.
- **Intent matrix:** every operation persists target and reason in the specified
  order.
- **Race matrix:** concurrent start/stop and stop/remove serialize per Task;
  a contender exhausting the bounded lock retry gets
  `OperationInProgress { kind }`.
- **Convergence matrix:** the complete desired/observed cross-product in both
  providers — Running/Stopped-or-absent starts, Running/Starting waits then
  times out, Running/error repairs through `ensure_running`, Stopped/absent
  converges, Stopped/any-live-status stops, and Removed/any-live-status removes.
- **Removal-obligation matrix:** accepted-but-not-absent Kubernetes deletion
  does not succeed; an archive failure after runtime absence retains the remove
  marker; stale remove markers are never age-cleared; Removed/absent with an
  incomplete marker retries with the persisted retention and generation.
- **Correlation matrix:** `RuntimeOperationContext` reaches every adapter
  mutation; K8s restart labels the replacement pod under `Recreate`; Docker
  restart recreates and labels the sandbox container; server-container events
  never enter the Sandbox lifecycle classifier.
- **Watcher classification:** each of the four cases (convergence, operation
  echo, expected intermediate under a fresh marker, divergence) publishes or
  suppresses exactly as specified — a restart's deliberate pod stop is not a
  crash.
- **Delete ordering:** row deletion happens only after successful remove; a
  failed remove leaves the row with `Removed` intent for the reconciler.
- **Message-wake matrix:** pending newer/older than stop and remove intent,
  Open/Error/Closed eligibility, lost initial notification, conflicting
  operation failure, Ready re-notification until acknowledgement, and
  leader-failover backoff reset. A later explicit stop is never undone by an
  older pending Message.
- **Completion durability:** crash after runtime convergence but before event
  publication is recovered from the marker; deterministic event persistence
  and marker acknowledgement commit atomically; retries of one generation do
  not duplicate `Stopped`/`Removed`.
- **Attempt-cap matrix:** retries of one generation consume the same cap; a new
  caller operation or causally eligible pending Message resets it; leader
  failover reset remains accepted looseness.
- **Graceful stop ordering:** stop and remove issue StopAgent through the
  agent-stop port before the runtime effect (asserted via recording port).
- **No-op suppression:** reusing an already-Ready sandbox returns a no-op
  outcome and publishes nothing.
- **Failure visibility:** one typed error and one deterministic `Error` fact per
  operation generation (after any bracketing event); recovery may later publish
  that generation's completion fact; no caller duplicates.
- **Adapter conformance:** Docker and Kubernetes satisfy shared idempotency
  cases.

Replace, do not layer — tests that disappear with the shallow paths: caller
tests asserting exact `build_provision_request` wiring, handler tests for
private `record_sandbox_*` helpers, adapter-specific start-failure repeats, and
mocks exposing every repository through handler state. The sandbox authz unit
tests (`authorize_*_inner`) test HTTP-side authorization and survive unchanged.
