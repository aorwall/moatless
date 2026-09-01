# One owner for "the agent is working"

> **Status:** implemented (see "What landed" below)
> **Updated:** 2026-09-01
> **Vocabulary:** CONTEXT.md
> **Anchored to:** `main` @ `b2fff5c`

Whether an agent is working is derived three times, from two raw signals, in three
places. No module owns the fact, so each surface assembles it from the signals itself —
and the answers disagree. Task `0d48056e` is the observed case: `moat tasks status`
reports `Agent: running`, the old frontend agrees, and the T3 sidebar shows the thread
settled. Neither client is wrong about the signal it reads.

| Part | Fixes | Mechanism |
|---|---|---|
| 1 | The state machine and the turn row drift apart | One act per transition, in the coordinator |
| 2 | Drift is corrected in one direction only | Symmetric reconcile in the reaper |
| 3 | The rule for "running" has no home | One pure derivation; every surface projects from it |

Part 2 is independently shippable and closes the observed gap. Part 3 is the seam that
makes consistency structural rather than maintained by hand.

## Current solution

### The fact is derived three times

| Site | Raw signal | Shape |
|---|---|---|
| `derive_live_status` — `crates/sandbox-runtime/src/live_status.rs:12` | live gRPC heartbeat | pure fn, 7 tests |
| `session_status` — `crates/t3code/src/projection.rs:1403-1427` | `task_turns` rows | pure fn, different inputs |
| `useTaskActivity` — `apps/frontend/src/features/navigation/hooks/use-task-activity.ts:210-212` | SSE event flow | client-side accumulator |

`derive_live_status` combines Task intent with the live heartbeat and reports
`agent_status: Option<AgentStatus>` (`Running`/`Waiting`/`Idle`/`Unknown`,
`crates/types/src/sandbox.rs:239-288`); it is `None` unless the Sandbox is `Ready`
(`live_status.rs:26-30`). `session_status` never sees the heartbeat: it reads
`turn_is_open` (`projection.rs:1307`, `task_turns.status ∈ {active, interrupted}`) and
`turn_is_owed` (`projection.rs:1333`, a user message newer than the newest turn's end),
and reports `running`/`starting`/`ready`/`stopped`/`error`. `useTaskActivity` reads
neither: it flips a client-held boolean on `agent.started`/`agent.message` and
`taskTurn.active` versus `agent.completed`/`stopped`/`error` and `taskTurn.completed`/
`error` (`use-task-activity.ts:119-138`).

The old frontend runs two of the three at once — the accumulator above for the sidebar,
and `sandboxStatus?.agentStatus === 'running'` off the REST live-status poll for the
chat and task card (`chat-container.tsx:79`, `chat-input-section.tsx:98`,
`task-card/task-card.tsx:263`, cached and event-mutated at
`global-event-stream-provider.tsx:61-90`).

### Two representations of one state machine

The agent's state lives in the Sandbox (`sandbox/src/agent/agent_state.rs`: `Idle`,
`Running`, `Waiting`, `Stopped`) and is projected onto `task_turns` at the backend. Both
are driven from the same signals in `sandbox/src/agent/coordinator.rs`, but as two
separate steps of the run loop:

- `self.coordinator.handle_signal(signal)` (`coordinator.rs:948`) publishes the events —
  `publish_turn_start` on `Signal::ExecutionStarted` (`coordinator.rs:276`, definition
  `:440-448`, emitting `agent.started` then `taskTurn.active`), and the terminal
  `agent.completed`/`stopped`/`error` on an outcome.
- The state machine is mutated afterwards, in a different block:
  `set_running` at `coordinator.rs:962-964`, `set_idle` at `:978` and `:1123`,
  `set_stopped` at `:1005` and `:1269`.

Nothing ties the two writes together. They are derived from one signal and applied at two
sites, with no invariant and no check.

The backend commits the event side transactionally at gRPC ingress:
`resolve_turn_for_run` is the sole allocator of turn numbers
(`crates/tasks/src/task_turn/dao.rs:358-433`, advisory-locked, inserting `status='active'`
at `:409-419`), and `apply_ingress_event` (`dao.rs:236-296`) applies `Open`/`Interrupted`/
`Terminal` in one transaction with the `task_events` row.
`EventHandlerConsumer::project_turn_state` (`crates/backend/src/event_handler/consumer.rs:346-394`)
re-applies the same classification idempotently as a backstop.

**What the code does not say:**

- **`agent.started` writes nothing durable on the backend.** Its consumer arm is empty —
  `EventKey::Agent(AgentSubType::Started) => {}` (`consumer.rs:481`), commented "Nothing
  generic to do — the Adapters that surface a live status subscribe to it." The turn row
  opens off `taskTurn.active` and the allocator, not off `agent.started`.
- **Reconciliation runs one way only.** `reconcile_orphaned_turns`
  (`crates/backend/src/reaper.rs:692-730`) selects turns *still open past an age floor*
  and closes them when `agent_is_terminal` (`reaper.rs:746-750`) reports `idle`/`stopped`.
  By construction it never examines a settled turn, so "agent `Running` while the newest
  turn row is terminal" has no corrective path at all. That is the observed state.
- **No statement ever reopens a terminal turn.** The three status writes in
  `task_turn/dao.rs` are the `Open` upsert gated `WHERE task_turns.status = 'interrupted'`
  (`:61-73`), the `Interrupted` update gated `AND status = 'active'` (`:74-80`), and the
  allocator's stale-active sweep (`:401-407`). A `completed`/`error`/`stopped` row is
  final.
- **Agent status is never persisted or cached.** No migration names it; no Redis key holds
  it. Every consumer — the REST live-status handler, T3's `ContractObserver`
  (`crates/backend/src/t3code/projection.rs:34-80`), and the reaper — makes its own live
  call.
- **That live call is expensive and per-task.** K8s: two Deployment GETs plus a Pod LIST
  (`crates/sandbox-k8s/src/status_resolver.rs:400-465`), then a fresh unpooled gRPC channel
  and `get_agent_status_for_task` when `Ready` (`:297-329`). Docker: `inspect_container`
  plus the same fresh gRPC round trip (`crates/sandbox-docker/src/status_resolver.rs:190-321`).
  **There is no batch variant.** `ClusterDiagnostics::list_sandbox_diagnostics` carries no
  `agent_status` field, and `FleetObserver::observe_fleet`
  (`crates/sandbox-runtime/src/fleet.rs:68-72`) is one label-selector LIST per tick that
  never asks the agent anything.
- **The T3 shell projection is strictly SQL.** `Conversation` (`crates/t3code/src/dao.rs:43-104`)
  is populated by one parameterized query (`dao.rs:655-880`) over `tasks`, `task_follows`,
  `sandboxes`, `task_turns`, `task_messages`, `task_activities`. `ShellFeed::poll`
  (`crates/t3code/src/feed.rs:102-223`) reads the change-feed counter first and re-reads
  only the task ids it names. The crate holds a `SandboxConnector` port but never calls it
  from `dao.rs`, `feed.rs` or `projection.rs`.
- **The polling intervals are not the gap.** Both surfaces poll: the old UI's REST
  live-status at `staleTime: 5000`, T3's `orchestration.subscribeShell` at 250 ms
  (`crates/t3code/src/lib.rs:117`), `subscribeThread` at 1 s (`:127`),
  `sandbox.subscribeStatus` at 2 s (`crates/t3code/src/sandbox.rs:24`). The gap is the
  source field, not the cadence.
- **`sandbox.status` computes `agent_status` and drops it.** `crates/t3code/src/sandbox.rs:94-101`
  calls `SandboxObserver::live_status` and serializes only `{"sandboxStatus": …}`.
  Un-dropping it would not move the sidebar pill, which is fed by
  `orchestration.subscribeShell`, not `sandbox.subscribeStatus`.

## Decisions

| Question | Decision |
|---|---|
| Should the T3 shell projection call the runtime for agent status? | **No.** The listing serves N conversations off one change-feed-gated query every 250 ms; `live_status` is per-task, uncached, ~3 infra calls plus a gRPC round trip, with no batch variant. A seam the listing cannot afford to cross is not a seam for the listing. |
| Add a `RunningStatus` port with an implementation per surface? | **No.** One production adapter is a hypothetical seam. Nothing varies about the *rule*; what varies is the source of the heartbeat, and that seam exists with two real adapters (`SandboxStatusReader`, `crates/sandbox-runtime/src/status_reader.rs:32-57`). |
| Persist agent state as a new column beside `task_turns.status`? | **No.** A second status column is two fields that can disagree, which is the defect. `task_turns` is already the durable projection of the state machine (`reaper.rs:682-685`); make its writers honest instead. |
| Where does the rule live? | **`moatless-backend-core`,** by the precedent already set for `ActivitySource`/`Activity`. Not `sandbox-runtime` — it would have to learn turn concepts, which is the wrong direction. Not `t3code` — declared read-only, and it cannot own a rule two bands need. |
| Name it `Activity`? | **No.** `moatless_backend_core::activity::Activity` already means *tool* activity (`AskUserQuestion`, `ExitPlanMode`). Collision. |
| Does `sandbox.status` stop discarding `agentStatus`? | **Yes**, in Part 3, for the sandbox panel — but it is not what fixes the sidebar. |
| Does the SSE client keep accumulating? | **No.** The `agent`/`taskTurn` events carry the derived value and the client applies it. |
| Order? | 1, 2, 3. Part 2 alone closes the observed gap and can ship first if the fleet needs it sooner. |

## Planned solution

**New**

- A `run_id`-stamped agent-state report on the existing gRPC status call, so a heartbeat
  can be matched against the turn it belongs to rather than compared as a bare string.
- A symmetric arm in the reaper: a terminal turn whose agent reports `running`/`waiting`
  for the same `run_id` is reconciled, not ignored.
- One pure derivation in `moatless-backend-core` over one input struct, with the
  precedence rules and their tests. `derive_live_status` and `session_status` become
  callers of it.

**Changes materially**

- `sandbox/src/agent/coordinator.rs` — the event publish and the state-machine transition
  become one act per signal.
- `crates/backend/src/reaper.rs` — `reconcile_orphaned_turns` gains the converse case and
  is renamed for what it now does.
- `crates/t3code/src/projection.rs` — `session_status` projects the shared derivation
  instead of re-deriving from turn columns; `UiRpcTurnStatus` (`:28-47`) stays, because
  parsing an unknown status defensively is still this crate's job.
- `crates/sandbox-runtime/src/live_status.rs` — `derive_live_status` becomes a projection
  of the shared derivation. Its wire-contract test (`:181-198`) is the change detector and
  is expected to fail; update it deliberately.
- `apps/frontend/src/features/navigation/hooks/use-task-activity.ts` — the accumulator
  (`:119-138`, `:210-212`) is deleted in favour of the value the events now carry.

**Untouched**

- The change-feed tables and their write discipline (`crates/backend-core/src/change_feed/mod.rs:19-34`).
  Every new write lands in the same transaction as its `record_*_change`, per the pattern
  `crates/sandbox-control/src/lifecycle/dao.rs:337-344` already follows.
- `SandboxStatusReader`, its two adapters, and the per-task live read they serve. It stays
  the correction path.
- Sandbox lifecycle, the reconciler, leader election, and `SandboxStatus` itself.
- The `task_turns` schema (`crates/core/migrations/0071_task_turns.sql:14-22`, plus `0082`,
  `0084`). No migration in this plan.

---

## Part 1 — Bind the transition and the publish into one act

**`sandbox/src/agent/coordinator.rs`**

The run loop publishes at `:948` and mutates `agent_state` at `:952-966` and `:968-1005`.
Both derive from one signal. Collapse them so a signal cannot be half-applied: the
coordinator's signal handler owns both, and the run loop calls it once.

Concretely, `Signal::ExecutionStarted` performs `set_session_id`, `set_run_id`,
`set_running` and `publish_turn_start` together; each `Outcome` arm performs its
`set_idle`/`set_stopped` and its terminal publish together. `turn_active` follows from the
same place rather than being tracked separately at `:965` and `:970`.

This does not make the pair atomic across the process boundary — a pod that dies between
the publish and the ack still leaves the backend behind, which is what Part 2 exists for.
It removes the case where a live process holds two states that disagree with each other
locally.

**Test:** a coordinator-level test that drives `ExecutionStarted` → `Outcome::Completed`
and asserts, at each step, that the published event and `agent_state.status()` agree. It
must fail against the current two-block arrangement.

## Part 2 — Reconcile in both directions

**`crates/backend/src/reaper.rs`**

`reconcile_orphaned_turns` (`:692-730`) reads turns still `active`/`interrupted` past the
age floor and closes the genuinely orphaned. Add the converse selection: turns whose
newest row is `completed`/`stopped`/`error` while the agent reports `running` or
`waiting`.

Two rules keep the new arm from fighting the old one:

1. **Match on `run_id`, not on liveness alone.** `task_turns.run_id` exists
   (`crates/core/migrations/0082_task_turn_run_id.sql:10`) and the agent already holds one
   (`set_run_id`, `coordinator.rs:959-961`). A heartbeat for a *newer* run than the newest
   turn row means the turn-open event was lost — allocate the turn. A heartbeat for the
   *same* run as a terminal row means the terminal event was premature or the state
   machine never stood down — the disagreement is real and belongs in a metric and a
   `warn!`, not a silent write. Part 1 makes the second case rare enough to be a signal.
2. **`waiting` is not terminal.** `agent_turn_is_over` (`reaper.rs:757-759`) already
   encodes this — a parked run continues where it left off. The new arm reuses that
   function rather than restating the rule.

Allocation goes through `resolve_turn_for_run` (`task_turn/dao.rs:358-433`), which is the
sole allocator and already advisory-locks per task and records the change
(`dao.rs:421-429`). No new SQL and no new writer: the reaper calls the owner.

**Test:** a db-tests case that seeds a `completed` turn plus a stubbed connector reporting
`running` under a newer `run_id`, runs the reaper, and asserts a new `active` turn plus a
`task_conversation_changes` row. A second case asserts the same-`run_id` disagreement logs
and writes nothing.

## Part 3 — One rule, one function

**New — `moatless-backend-core`**

One input struct and one function over it. The inputs are the union of what the two
existing derivations take:

| Input | Source today |
|---|---|
| task status | `tasks` |
| sandbox desired state, `sandbox_started_at`, stop reason | `sandboxes` |
| observed sandbox status | runtime (`SandboxStatusReader`) |
| agent state | runtime heartbeat |
| turn status, `turn_ended_at` | `task_turns` |
| `latest_user_message_at` | `task_messages` |

Seven of the eight already sit on `Conversation` (`crates/t3code/src/dao.rs:43-104`), so
for the T3 caller this is a row it already loads plus a call. The eighth — agent state —
is available to the reaper and the REST handler, which make the live read anyway, and is
`None` for the shell listing. The function must therefore be honest about absence: a
missing heartbeat is *unknown*, and unknown resolves to the turn row, which is exactly the
precedence `derive_live_status` already applies to a non-`Ready` Sandbox (`live_status.rs:26-30`).

Precedence to encode, all of it moved rather than invented:

- Closed task with no Sandbox reads `Removed` (`live_status.rs:19-24`).
- Agent state is not applicable unless the Sandbox is `Ready` (`live_status.rs:26-30`).
- A running Sandbox with an open or owed turn is the agent working
  (`projection.rs:1409-1411`); the owed case covers the window between a person sending and
  the turn opening, and its reasoning at `projection.rs:1320-1332` moves with it.
- A running Sandbox that has not come up yet is connecting, not idle (`projection.rs:1412-1413`).
- `Waiting` is working, not settled (`reaper.rs:752-756`).

**Callers**

- `derive_live_status` (`crates/sandbox-runtime/src/live_status.rs:12`) projects
  `LiveStatus` from it. The three-field wire contract is unchanged; its pinning test
  (`:181-198`) stays green or the change is wrong.
- `session_status` (`crates/t3code/src/projection.rs:1403-1427`) projects the T3 session
  vocabulary from it. `crates/t3code` reads and does not write, so nothing here strains its
  licence.
- `crates/t3code/src/sandbox.rs:99-101` stops discarding `agentStatus` and adds it to the
  `sandbox.status` payload, with the contract change in
  `t3code/packages/contracts/src/` and a capability flag. This serves the sandbox panel;
  the sidebar is served by the `session_status` change above.
- The `agent` and `taskTurn` SSE events carry the derived value, so
  `apps/frontend/.../use-task-activity.ts` applies rather than infers and its
  `isRunningStart`/`isRunningStop` pair (`:119-138`) is deleted. `liveStatusToActivityState`
  (`features/tasks/utils/live-status.ts:22-33`) keeps its shape and reads the carried value.

**Deletion test.** Delete the function and the rule reappears at `session_status`,
`derive_live_status`, `use-task-activity.ts:210`, `Sidebar.logic.ts:483-507`,
`chat-container.tsx:79` and `task-card.tsx:263`. Six sites. It earns its keep.

**Test:** the precedence table above as unit cases on the pure function, including a
`None` heartbeat resolving to the turn row, and `Waiting` reading as working. The existing
seven tests in `live_status.rs:52-199` move to it and must still pass through the
projection.

## Verification

- `cargo clippy --workspace --all-targets -- -D warnings`, plus the
  `moatless-backend-core` and `db-tests` lines from AGENTS.md.
- `cargo test -p moatless-backend-core -p moatless-tasks -p moatless-t3code`;
  `cargo test -p moatless-backend --features db-tests` for the Part 2 reaper cases;
  `cargo test -p moatless-sandbox` for Part 1.
- `bun run type:check` and `bun run biome:check`.
- The success criterion for the whole plan is one assertion, and it should be written
  first: for a task whose agent reports `running`, the old frontend's `agentStatus` and
  T3's `session.status` resolve from the same call, so they cannot disagree. Task
  `0d48056e` is the reproduction — `moat tasks status` says `Agent: running` while the T3
  sidebar shows the thread settled.

---

## What landed

All three parts, plus the surfacing the plan folded into Part 3. Deviations from the
plan as written are named below, each with its reason.

### Part 1 — one act per signal

`Coordinator` holds the `AgentState` and the run's cancellation root
(`Coordinator::bind`, called by `CoordinatorService::new` with the *same* `Arc` the
service holds, so the two cannot drive different state machines). `handle_signal` now
opens the turn (`open_turn`: mint the turn token, `set_session_id`/`set_run_id`/
`set_running`) *before* `publish_turn_start`, and leaves it (`close_turn`) *before*
`publish_outcome`. It returns the minted token for the run loop's cancellation arm.
`turn_active` moved off the service onto the coordinator.

**Deviation.** `Outcome::Failed` does not transition inside `close_turn`. What a failure
does to the state machine depends on the retry budget, which is the service's to know, so
the service settles it in the block that already ran *before* the publish (where
`suppress_next_failure_event` and credential recovery already lived). The invariant is
unchanged — the turn is left before any event says it ended — and this avoids a second
`set_error` double-counting the breaker.

**Test.** `a_turn_moves_the_state_machine_and_the_event_stream_together`
(`sandbox/src/agent/coordinator/tests.rs`) drives `ExecutionStarted` → `Completed` and
asserts after *every* signal that `agent_state.status() == Running` iff the events say
the turn is open. Verified to fail against the previous arrangement.

### Part 2 — reconcile in both directions

`TaskTurnDao::list_settled_turns_with_later_activity` narrows the fleet with a pure-DB
predicate first (newest turn terminal, `ended_at` recorded, and an `agent.activity` event
after it), then `Reaper::reconcile_live_runs` confirms against the agent. The decision is
extracted as the pure `classify_live_run` → `LiveRunVerdict`, with six cases as tests.
`GetAgentStatusResponse` gained `run_id` so a heartbeat is matched to a turn rather than
compared as a bare string. Allocation goes through `resolve_turn_for_run`, the sole
allocator. `task_turns_reopened_total` and `task_turns_disagreed_total` are described.

### Part 3 — one rule

`moatless_backend_core::running` holds `Working`, `RunningInputs` and `derive_running`
(16 tests). `TurnStatus` moved to `moatless-types` so `backend-core` can name it.

Callers:

- `session_status` (`crates/t3code/src/projection.rs`) projects from it.
- `apply_running_rule` (`crates/sandbox-runtime/src/live_status.rs`) folds the durable
  turn state into a `LiveStatus` the runtime produced. It only ever *upgrades*: a
  `Running` heartbeat is ground truth and stays; what changes is the case the heartbeat
  cannot speak to. The three-field wire contract is unchanged and its pinning test is
  still green.
- The REST `live-status` handler and the T3 `ContractObserver` both apply it, reading the
  turn half through the new `TurnStateReader` port (`backend-core::task`, implemented by
  `TurnLedger` over one query). A failed turn read degrades to the runtime's own answer
  rather than to an error.

**Deviations.** `UiRpcTurnStatus` in `crates/t3code/src/projection.rs` was deleted rather
than kept: `TurnStatus::parse` returns the same defensive `Option`, which was the reason
the plan gave for keeping it. And `derive_live_status` was *not* rewritten as a projection
— it runs inside the runtime providers, which can see the pod and the heartbeat and
nothing else. Splitting the question was the better seam: it keeps owning sandbox-status
assembly, and `apply_running_rule` runs where the Task band's signals are also in hand.

### Surfacing

- `sandbox.status` reports `agentStatus` (absent, never null, when there is no agent to
  report on), behind the `sandboxAgentStatus` capability. Contract changes in
  `packages/contracts/src/sandbox.ts` and `environment.ts` of the T3 client.
- `StreamEvent` carries `agentWorking: Option<bool>`. Absent means *this event does not
  say* — never "not working" — which is also what a pre-field server's events mean, so no
  version check is needed. Derived by `moatless_types::event::agent_working`, the
  degenerate case of `derive_running` where only the turn half is known. The `(type,
  sub_type)` table itself lives on `TurnStatus::from_event`, which
  `classify_turn_transition` now delegates to, so there is one table and not two.
- `use-task-activity.ts` applies the carried value; `isRunningStart`/`isRunningStop` are
  deleted. `global-event-stream-provider.tsx`'s live-status overlay reads the same field
  rather than its own `subType` table, so the cache and the sidebar agree by construction.
  Its cache gate admits `taskTurn` alongside `agent`, because when the reaper closes a turn
  on behalf of a pod that can no longer speak, a `taskTurn` event is the only carrier of
  the answer. That is also why only an `agent` event is allowed to claim the sandbox is
  ready: the pod publishes those and its own liveness is implied, whereas the backend
  publishes `taskTurn` and knows nothing about the pod by publishing it.

### Verified

`cargo clippy --workspace --all-targets -- -D warnings`, plus the `moatless-core`,
`moatless-backend-core` and `db-tests` lines; `cargo test` on `moatless-types` (88),
`moatless-core` (58), `moatless-backend-core` (18), `moatless-tasks` (213),
`moatless-sandbox-runtime` (146), `moatless-sandbox-control` (181), `moatless-t3code`
(366), `moatless-sandbox` (633), `moatless-backend --lib` (134), `moatless-arch-rules`;
`check-table-ownership`, `check-crate-seams`, `check-appstate-composition-root`,
`check-dao-composition-root`, `check-bindings-adapters-baseline`,
`check-plan-assertions`; `bun run type:check` and `bun run biome:check`; the T3 client's
`pnpm typecheck`.

`openapi-specs.json` carries this change's `agentWorking` field and nothing else. A
regeneration against `main` also emits the `/tasks/{id}/fork` surface, which `main` landed
without committing; that is not this change's to absorb, and CI regenerates the spec from
the binary rather than diffing the committed copy, so nothing gates on it either way.

**Executed against Postgres 16.2.** Two db-tests in
`crates/backend/tests/integration_data/task_turn_dao_test.rs` cover the two new queries —
`settled_turns_with_later_activity_are_the_only_candidates` pins the candidate predicate
against every near-miss (quiet, still-open, no `ended_at`, superseded, wrong event type),
and `latest_running_signals_reads_the_newest_turn_and_the_newest_person` pins the
live-status read. Both pass.

The first was also confirmed to be a real regression detector rather than a test that
merely passes: restoring the `e.timestamp > tt.ended_at` comparison makes it fail with
`42883 operator does not exist: text > timestamp with time zone`. The candidate query
therefore dates activity by `task_events.created_at` (timestamptz), not by `timestamp`,
which the schema declares `text` and a publisher fills free-form — `message/dao.rs` states
the same rule for the same reason.

Running them took standing up a throwaway Postgres, because nothing else will: `ci.yml`
*links* the db-tests tier (`cargo nextest run --features db-tests --no-run`) rather than
running it, and no workflow in the repo starts a database. That is a deliberate build-cost
decision recorded in `references/build-costs.md`, and its cost is that the tier rots
unobserved — **51 db-tests fail identically on `origin/main` and on this branch** (4 in
`integration_adapters`, 11 in `integration_data`, 36 in `integration_surface`), on stale
fixtures rather than on anything either branch changed. Verified by running both revisions
against the same server. Every `task_turn_dao_test` passes, as does `--lib` (134).

That gap is worth closing on its own terms, and does not belong to this change; see
`docs/deferred-changes.md`.

**Still owed:** the reaper-level db-test (seed a `completed` turn, stub a connector
reporting `running` under a newer `run_id`, assert a new `active` turn plus a
`task_conversation_changes` row). The decision half is covered by the pure
`classify_live_run` tests; what is untested is `open_turn_for_live_run`'s write.
