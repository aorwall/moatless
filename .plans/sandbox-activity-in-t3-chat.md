# Sandbox activity in the T3 chat view

> **Status:** approved
> **Updated:** 2026-08-02
> **Vocabulary:** CONTEXT.md

Creating a thread in T3 starts a Sandbox that takes tens of seconds to come up. The pod watcher says where it got to, which node it landed on, and why it is slow. None of it reaches the chat — a person watches a spinner that says nothing until the agent's first word.

After this ships, a person who starts a thread sees the Sandbox come up in the chat itself:

```
sandbox   Scheduled       Scheduled onto a node
          node i-0040f860ad3bd3487 · pending 37s

sandbox   Slow startup    Slow startup: FailedScheduling: 0/8 nodes are available: …
          node i-0040f860ad3bd3487 · cause FailedScheduling: 0/8 nodes are available: …

sandbox   Ready
```

`Slow startup` renders with an amber heading and `Out of memory` with a red one, using row treatments T3 already ships.

## Scope

**In scope**

- Sandbox *lifecycle* events reaching the conversation as work-log rows — the pod watcher's and the lifecycle's vocabulary, keyed on `payload.lifecycleEvent`.
- The metadata that makes them worth reading: `node`, `pending`, `cause`, `by`, `exit`, `reason`.
- Those rows arriving **live**, not only on a full re-read.
- The stall and failure cases: scheduling blocked, slow startup, evicted, OOM, crash loop, pod failed.

**Out of scope**

- The Sandbox's *own* init events — `CredentialsFetched`, `RepositoryCloned`, `SetupFailed` (`sandbox/src/init/mod.rs`). They carry no `lifecycleEvent`, and the label table this plan builds is keyed on it. A later question.
- Any change to the T3 client, `packages/contracts`, or the mobile app.
- Server-container events (`installing` / `starting` / `failed`).
- The Moatless Events tab and preview log, which read `task_events` and are unaffected.
- Backfilling threads that already ran.

## Current solution

The feature is roughly ninety percent built and was never wired up. `.plans/completed/event-sourced-conversation.md:263` says of `consumer.rs`: *"Also write a Sandbox activity — add the second consumer on the branch that exists."* The DAO, the mapping, the projection and their tests all shipped; the consumer call did not, and the plan was archived as complete.

### Anchors — every place this change lands

| Anchor | What it does today |
|---|---|
| `backend/src/event_handler/consumer.rs:519` | Branches on `EventKey::Sandbox(Initializing \| Ready \| Stopped)` and notifies Linear only. **The gap.** |
| `backend/src/event_handler/consumer.rs:493` | `EventKey::Sandbox(SandboxStatus::Error)` → Linear + `handle_sandbox_fatal`. Writes no activity either. |
| `backend/src/ui_rpc/projection.rs:339` | `change_events` returns only `session_set` when the change names no Message. **The gap.** |
| `backend/src/ui_rpc/dao.rs:781` | `changed_messages` reads activities by Message uuid; a standalone row has none, so it is never in the window. **The gap.** |
| `backend/src/message/dao.rs:749` | `record_sandbox_activity` — implemented, tested, **zero callers** |
| `backend/src/message/dao.rs:763` | `record_standalone_activity` — the same shape, used by the context-window path |
| `backend/src/task_activities/dao.rs:241` | `record_sandbox` — the collapse guard, reached only through the dead caller |
| `backend/src/task_activities/dao.rs:274` | `record_standalone` — `ON CONFLICT (activity_id) DO NOTHING` |
| `backend/src/task_activities/mapping.rs:615` | `sandbox_activity(event_id, task_id, status, detail, is_error) -> ActivityRow` |
| `backend/src/task_activities/mapping.rs:102` | `enum ActivityKind` — the closed set |
| `backend/src/task_activities/view.rs:76` | `t3_tone()` — `is_error` → `"error"`, everything else `"info"` |
| `backend/src/task_activities/view.rs:90` | `t3_kind()` — `SandboxStatus \| SandboxError` → `"task.progress"` |
| `backend/src/ui_rpc/feed.rs:459` | `poll()` — collects `message_uuid`s, reads the window, projects each change |
| `backend/src/ui_rpc/feed.rs:534` | `coalesced()` — keeps **every** change naming no Message |
| `backend/src/ui_rpc/projection.rs:775` | `activity_row` — puts an activity in the thread snapshot |
| `backend/src/task_activities/dao.rs:577` | `allocate` — the one `INSERT … ON CONFLICT DO UPDATE RETURNING` for both change kinds |
| `backend/src/sandbox/k8s/pod_watcher.rs:2033` | `format_sandbox_lifecycle_message` — already produces "Scheduled onto a node", "Ready", "Out of memory (OOMKilled)" |
| `backend/src/sandbox/k8s/pod_watcher.rs:1840` | `sweep_pending_pods` — publishes `scheduling_blocked` and `startup_slow` with node, cause and pending seconds |
| `backend/src/sandbox/lifecycle/events.rs:163` | Tags a `Stopped` completion with `lifecycle_event` + `initiator`; a failure with its bounded cause slug |
| `crates/events/src/lib.rs:432` | `SandboxEventPayload` — message, lifecycleEvent, error, exitCode, terminatedReason, nodeName, disruptionReason, initiator, pendingSeconds |
| `apps/frontend/src/features/preview/utils/event-messages.ts` | `DESCRIPTORS` + `buildChips` — the label, tone and chip vocabulary, in TypeScript, in the Moatless app only |
| `t3code/apps/web/src/session-logic.ts:713` | Forces the work-log tone to `"thinking"` for every `task.progress` row |
| `t3code/apps/web/src/components/chat/MessagesTimeline.tsx:1932` | `showWarningIndicator = sourceActivityKind === "runtime.warning"` → amber heading |
| `t3code/apps/web/src/components/chat/MessagesTimeline.tsx:1946` | `showDestructiveRowStyle` — requires `sourceActivityKind === "runtime.error"` → red heading |
| `t3code/packages/contracts/src/orchestration.ts:320` | `payload: Schema.Unknown` — the contract does not constrain what a row carries |

### What research turned up that the code does not say

**The status is not the event.** `pod_scheduled`, `scheduling_blocked` and `startup_slow` are all published as `SandboxStatus::Initializing`. The sub-type is coarse; the vocabulary lives in `payload.lifecycleEvent`. Anything that collapses on the status word collapses a stall into the row before it and shows nothing.

**The rendering being asked for already exists, in the wrong app.** `apps/frontend/src/features/preview/utils/event-messages.ts` holds a 45-entry `DESCRIPTORS` map keyed on `lifecycleEvent` (`pod_scheduled → "Scheduled"`, `startup_slow → "Slow startup"`), a four-value tone (`ok | recovering | error | neutral`), and `buildChips` producing exactly `node`, `cause`, `by`, `exit`, `reason`, `pending`. Only the Moatless preview log reads it. This plan ports the label and severity halves into Rust; the chips become text.

**T3 discards the tone on precisely these rows.** `session-logic.ts:713` sets the work-log entry tone to `"thinking"` whenever the activity kind is `task.progress`, without reading the wire tone — and every Sandbox activity is `task.progress` (`view.rs:93`). So `is_error` is computed, stored, projected, sent, and then dropped. An OOM kill renders with the same bot icon as a successful clone.

**That is not a T3 bug.** `task.progress` upstream is a spawned subagent's heartbeat — `{ taskId, description, summary, lastToolName, usage }` (`t3code/packages/contracts/src/providerRuntime.ts:469`), mapped from Claude's `task_progress` system message (`ClaudeAdapter.ts:2692`). Its tone is hard-coded `"info"` at the ingestion site (`ProviderRuntimeIngestion.ts:517`) and cannot be anything else, because a progress tick is not an outcome — the outcome arrives as `task.completed`, whose tone *is* conditional (`:543`). Forcing `thinking` is the client picking a nicer glyph for a row it knows is a running agent. The fork put Sandbox status on the one event type defined as never failing.

**Severity in T3's work log is carried by the activity *kind*, and only two kinds colour a heading.**

| kind | tone at ingestion | row treatment |
|---|---|---|
| `runtime.error` | `error` | red heading, red icon (`MessagesTimeline.tsx:1946`) |
| `runtime.warning` | `info` | amber heading, `x` glyph (`MessagesTimeline.tsx:1932`) |
| `tool.denied` | `error` | alert icon only |
| `task.completed` | `error` when failed | alert icon only |
| `task.progress` | `info` | forced to `thinking`, bot icon |

`tool.denied` and a failed `task.completed` carry tone `error` and still render a plain heading, because `showDestructiveRowStyle` requires `sourceActivityKind === "runtime.error"` or a non-tool-like entry, and tone `error` makes an entry tool-like (`session-logic.ts:143`). `runtime.warning` is the only amber in the whole vocabulary.

**The `runtime.*` namespace is exactly two kinds** — `runtime.warning` and `runtime.error` (`providerRuntime.ts:194-195`).

**Borrowing them is honest and narrow.** Upstream emits `runtime.*` when the environment running the agent has a problem, outside the conversation's content: CLI stderr that looked fatal (`CodexAdapter.ts:1267`), a transport error, an unparseable notification, the world-writable-directory warning. In this fork the Sandbox *is* that environment — the agent process runs inside the pod — so an OOM kill or an eviction is the same category one layer down, and the session does die underneath the conversation. Nothing provider-specific survives into the row: the activity payload for `runtime.error` is `{ message }` and nothing else (`ProviderRuntimeIngestion.ts:383`); `class`, `provider` and `providerRefs` live on the provider-runtime event, which this backend never emits. The client references are three, all presentational (`MessagesTimeline.tsx:1932`, `:1948`, `mobile/src/lib/threadActivity.ts:519`) — no retry, no banner, no session state.

**The change log names the row a change is about, not the transaction it happened in.** A tool result settles the row the call wrote, and `settle` moves neither `position` nor `message_uuid` (`task_activities/dao.rs:203`). So the change recording that result names the *calling* Message — `agent_handlers.rs:1022 anchor_message_uuid` — and the row it means sits at an older position. This is why `task_conversation_changes.message_uuid` exists and why a position-keyed read is not the answer for standalone rows: position says *when*, and only says *what* while the row has never moved.

**A change naming a row that does not exist is already tolerated.** `changed_messages` notes that a change "can name a uuid that has no Message row at all" (`ui_rpc/dao.rs:781`). So a collapsed Sandbox status that writes no row is safe.

**The live gap already bites production.** Context-window readings are written through the same standalone path (`agent_handlers.rs:566`), so the token meter only moves when a client re-reads the whole thread rather than when the reading arrives. Step 1 of this plan fixes that on its own.

**Event ids are stable, so the row id can be the event id.** `sandbox/src/events.rs:57` mints one UUID at enqueue and reuses it on retry; the ingress rejects an empty one (`grpc_server.rs:665`). Redelivery is therefore a no-op against `ON CONFLICT DO NOTHING`.

**Not every sandbox event is a status.** `workspace_synced` carries sync telemetry (bytes, direction, duration) every few minutes and is not a `SandboxStatus` value, so it already falls to `EventKey::Unknown` and is ignored. It must stay ignored.

**A thread created empty has no Sandbox at all.** `ui_rpc/backend.rs:163` sets `provision = first_message.is_some()`. This only shows anything on the "new chat with a message" path — which is the one the request is about.

**Migration numbering.** The highest is `0123_message_tool_response.sql`. This plan's migration is `0124`.

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| 1 | What becomes a row | One row per sandbox **lifecycle** event, collapsed against the row before it | Collapsing on the status word hides `pod_scheduled`, `scheduling_blocked` and `startup_slow` — all three are published as `initializing` |
| 2 | How a row reaches a live subscriber | The change record names the activity — a new `activity_id` column beside `message_uuid` | Position says *when*, not *what*: `settle` leaves a row at its old position, which is why the Message path already anchors by name. Client-side refetch was rejected: it changes T3 and refetches a whole thread per event. |
| 3 | How `node` / `pending` / `cause` reach the screen | The backend composes them into `payload.detail` as text | Typed fields plus a chip renderer is the better row, but it needs a T3 client change and the client stays untouched. The payload is `Schema.Unknown`, so that upgrade stays available later without a contract change. |
| 4 | How a stall or a failure is made legible | Map the activity **kind**: `runtime.error` / `runtime.warning` / `task.progress` | Severity in T3's work log is keyed on the kind, not the tone; those two are the only kinds that colour a heading. Putting severity only in the words gives up scanning; changing the T3 tone rule reopens the client for less. |

## Planned solution

**New**

- `activity_id` on `task_conversation_changes` (`backend/migrations/0124_conversation_change_activity.sql`) — nullable, catalogue-only.
- `record_activity_change` in `backend/src/task_activities/dao.rs`, beside `record_message_change` and `record_session_change`.
- `ActivityKind::SandboxWarning`, the third severity.
- A `lifecycleEvent → (label, severity)` table in `backend/src/task_activities/mapping.rs`.
- The second consumer on the Sandbox branch in `backend/src/event_handler/consumer.rs`.

**Changes materially**

- `change_events` gains a third branch; `changed_messages` reads named activities by primary key; `coalesced` folds by `activity_id`; no session upsert is emitted for an activity change.
- `t3_kind()` stops sending every Sandbox row as `task.progress`.
- `sandbox_activity()` composes the row instead of copying a status word through.
- `record_sandbox`'s collapse check moves ahead of the position allocation.
- The context-window meter starts moving live.

**Untouched**

- The T3 client, `packages/contracts`, the mobile app.
- Event publishing — the pod watcher, the lifecycle and the Sandbox's own init already say everything needed.
- The Moatless Events tab and preview log.
- Linear notification, which keeps its own branch.

### Data model

`backend/migrations/0124_conversation_change_activity.sql`:

```sql
-- What a change is about, when it is about an activity rather than a Message.
--
-- `message_uuid` names a Message; this names an activity that happened on no
-- Message — a Sandbox lifecycle change, a context-window reading. At most one
-- of the two is set; neither set means the conversation's own session moved
-- (its Turn, its Sandbox intent) and nothing else.
--
-- A name, not a description, for the same reason `message_uuid` is one: the
-- reader re-reads the row at delivery time and sends what it finds. And a name
-- rather than the position it was written at, for the reason the Message path
-- learned: `settle` updates a row in place and leaves it at the position of the
-- call, so the change that records a result is at a later position than the row
-- it means. Position says when; only a name says what.
--
-- No index: this is `task_activities`' primary key, and the reader looks up by
-- it. No foreign key, as on `message_uuid` and for the same reason — a record
-- must outlive what it names.
ALTER TABLE task_conversation_changes ADD COLUMN IF NOT EXISTS activity_id text;
```

Nullable and with no default, so the add is catalogue-only against a table that takes an insert per Message written. No `CHECK` constraint: validating it would scan the table, and the invariant is enforced by there being one recorder per case.

`ActivityKind` (`backend/src/task_activities/mapping.rs:102`) gains `SandboxWarning`, spelled `sandbox_warning`. Three places must be updated with it, and the existing `every_known_kind_is_readable` test fails until they are:

- `ActivityKind::as_str` and `ActivityKind::ALL`, plus whatever parses the column back.
- The SQL narrowing at `task_activities/dao.rs:119`.
- The Sandbox collapse read at `task_activities/dao.rs:248` — `kind IN ('sandbox_status', 'sandbox_error')` becomes three.
- The closed-set comment in `backend/migrations/0118_task_activities.sql:33`.

No migration for the kind: the column is `text` with the closed set enforced in code, which is the stated design ("adding a value must be a code change that fails to compile, not a migration").

### The vocabulary

A single `pub fn lifecycle_descriptor(lifecycle_event: &str) -> (&'static str, Severity)` in `backend/src/task_activities/mapping.rs`, ported from `DESCRIPTORS` in `apps/frontend/src/features/preview/utils/event-messages.ts`. `Severity` is `Status | Warning | Error`.

| lifecycleEvent | label | Severity | ActivityKind | T3 kind |
|---|---|---|---|---|
| `pod_ready` | Ready | Status | `SandboxStatus` | `task.progress` |
| `pod_scheduled` | Scheduled | Status | `SandboxStatus` | `task.progress` |
| `pod_deleted` | Deleted | Status | `SandboxStatus` | `task.progress` |
| `pod_succeeded`, `sandbox_completed` | Completed | Status | `SandboxStatus` | `task.progress` |
| `auto_stopped` | Auto-stopped | Status | `SandboxStatus` | `task.progress` |
| `idle_reaped` | Stopped (idle) | Status | `SandboxStatus` | `task.progress` |
| `startup_slow` | Slow startup | Warning | `SandboxWarning` | `runtime.warning` |
| `scheduling_blocked` | Scheduling blocked | Warning | `SandboxWarning` | `runtime.warning` |
| `pod_evicted` | Evicted | Warning | `SandboxWarning` | `runtime.warning` |
| `pod_recovering` | Recovering | Warning | `SandboxWarning` | `runtime.warning` |
| `node_reclaimed` | Node reclaimed | Warning | `SandboxWarning` | `runtime.warning` |
| `container_restarted` | Restarted | Warning | `SandboxWarning` | `runtime.warning` |
| `container_oom_killed` | Out of memory | Error | `SandboxError` | `runtime.error` |
| `container_crash_loop` | Crash looping | Error | `SandboxError` | `runtime.error` |
| `container_terminated` | Terminated | Error | `SandboxError` | `runtime.error` |
| `pod_failed` | Pod failed | Error | `SandboxError` | `runtime.error` |
| `fatal` | Fatal | Error | `SandboxError` | `runtime.error` |
| `server_auto_disabled` | Server disabled | Error | `SandboxError` | `runtime.error` |
| `runtime_infrastructure` | Infrastructure error | Error | `SandboxError` | `runtime.error` |
| `runtime_rejected` | Rejected | Error | `SandboxError` | `runtime.error` |
| `runtime_not_absent` | Cleanup stalled | Error | `SandboxError` | `runtime.error` |
| `intent_write` | Intent write failed | Error | `SandboxError` | `runtime.error` |
| `completion_commit` | Completion commit failed | Error | `SandboxError` | `runtime.error` |
| `edge_abandoned` | Operation abandoned | Error | `SandboxError` | `runtime.error` |

Fallbacks, in order: no `lifecycleEvent` → the coarse `SandboxStatus` (`ready → "Ready"`, `initializing → "Initializing"`, `stopped → "Stopped"`, `error → "Error"`), severity `Error` when the event's `is_error` is set and `Status` otherwise; an unknown `lifecycleEvent` → the slug with underscores replaced by spaces, same severity rule. That mirrors `describeEvent`'s own fallback and keeps a new watcher slug rendering as something readable rather than being dropped.

Put this in `mapping.rs`, not in the pod watcher. The watcher's `format_sandbox_lifecycle_message` produces the event's own message and stays where it is; display labels do not belong in the durable event payload that the Moatless log renders its own way.

### The row

`sandbox_activity()` (`backend/src/task_activities/mapping.rs:615`) takes the event's `SandboxEventPayload` rather than a status string, and builds:

- `summary` = the label.
- `detail` = the event's `message`, then one line per present field: `node <nodeName>`, `pending <formatted pendingSeconds>`, `cause <disruptionReason>`, `by <initiator>`, `exit <exitCode>`, `reason <terminatedReason>`. Same six as `buildChips`, same order, and the same compact duration format (`37s`, `6m 4s`, `1h 20m`).
- `kind` = `SandboxStatus | SandboxWarning | SandboxError` from the severity.
- `status` = `Failed` for `Error`, `Completed` otherwise.
- `is_error` = true for `Error` only. It stays truthful on the row for readers with their own rendering — the Moatless frontend, and any future T3 change.
- `activity_id` = the event id, `ordinal` = 0, `message_uuid` = `None`, as today.
- `payload` = `{ summary, detail }`, as today.

`t3_kind()` and `t3_tone()` (`backend/src/task_activities/view.rs:76`, `:90`):

```rust
ActivityKind::SandboxError => "runtime.error",
ActivityKind::SandboxWarning => "runtime.warning",
ActivityKind::SandboxStatus | ActivityKind::Thinking => "task.progress",
```

`t3_tone` keeps its `is_error => "error"` first arm, which gives `runtime.error` the tone T3 needs for `workEntryIndicatesToolFailure`. `SandboxWarning` falls through to `"info"`, which is what upstream assigns `runtime.warning`, and its amber comes from the kind.

### The live path

1. `record_activity_change(tx, task_id, activity_id)` in `task_activities/dao.rs`, calling the same `allocate` with `message_uuid = NULL` and the new column bound. `record_sandbox_activity` and `record_standalone_activity` (`message/dao.rs:749`, `:763`) call it instead of `record_session_change`.

2. `record_sandbox`'s collapse check moves ahead of the allocation. `record_sandbox_activity` reads the previous Sandbox row first and returns `Ok(false)` before touching the counter, so a restated status spends no position and no change record. Today it burns one of each and projects to nothing.

3. `ConversationChange` (`task_activities/models.rs`) gains `activity_id: Option<String>`; `changes_from`'s `SELECT` adds the column.

4. `changed_messages` (`ui_rpc/dao.rs:781`) takes named activity ids alongside the uuids and adds one read, `WHERE task_id = $1 AND activity_id = ANY($2)` — a primary-key lookup, no new index. Both result sets go into the same `Transcript.activities`.

5. `change_events` (`ui_rpc/projection.rs:339`) gets a third branch:

   - `message_uuid` set → today's behaviour.
   - `activity_id` set → the activity at `sequence(ordinal)`, and **no** `session_set`. Slot 0 belongs to the activity, which is what `sandbox_activity`'s own comment already assumes ("A Sandbox activity is alone in its change, so slot 0 is its own").
   - Neither set → `session_set`, as today.

6. `coalesced` (`ui_rpc/feed.rs:534`) folds changes naming the same `activity_id`, keeping the last position, exactly as it does for `message_uuid`. Changes naming neither stay all-kept.

7. `poll` (`ui_rpc/feed.rs:459`) collects both name lists and passes both to `changed_messages`.

### The consumer

Both Sandbox arms in `backend/src/event_handler/consumer.rs` write the activity:

- `:519` — the `Initializing | Ready | Stopped` arm, after the Linear notification, unconditionally (not gated on `linear_active_turns`).
- `:493` — the `SandboxStatus::Error` arm, alongside `handle_sandbox_fatal`.

Both parse `event.payload` as `SandboxEventPayload`, build the row through `sandbox_activity()`, and call `state.messages.record_sandbox_activity(&row)`. `Removing`, `Removed` and `NotCreated` keep falling through — they are intent bookkeeping, not something a person in a chat needs.

## Order of work

Four commits, each independently verifiable.

1. **The change log learns to name an activity.** The migration, `record_activity_change`, the `ConversationChange` field, `changed_messages`, `change_events`, `coalesced`, `poll`, and the collapse-check reorder. No Sandbox work at all.

   *Verifiable on its own:* the context-window meter starts moving live. That is the failing test — `agent_handlers.rs:566` already writes standalone activities that no subscriber receives.

2. **The vocabulary.** `ActivityKind::SandboxWarning` through the enum, both SQL narrowings, the migration comment, `t3_kind` and `t3_tone`; `lifecycle_descriptor`; `sandbox_activity()` composing summary and detail. Pure functions — unit-testable without a database.

3. **The consumer call.** Both Sandbox arms write the activity. Last, because this is the step that makes rows appear and the two before it are what makes them correct.

4. **The fork record.** In `t3code`: `docs/fork/upstream-merge-policy.md` gains a section for the `runtime.*` borrowing and a §4 convergence-watch row (behaviour landing on `runtime.error` or `runtime.warning` upstream would be inherited by Sandbox rows silently); `docs/fork/upstream-merge-log.md` gains the entry; `docs/internals/client-server-contract.md` §8 gains the kind mapping. Not `CONTEXT.md` — `runtime.error` is a wire spelling on the T3 side of the anti-corruption layer, and keeping those names out of the Moatless vocabulary is what `ui_rpc/` is for.

## Verification

Per `AGENTS.md`, after each Rust commit:

```
cargo clippy -p moatless-vibe-backend --all-targets -- -D warnings
cargo test -p moatless-vibe-backend
```

Step 1 touches `backend/tests/`, so it also needs `task test:integration` — a plain `cargo test` compiles only `supervisor_test.rs` and will pass green having built almost none of the rest.

Tests to add:

- `task_activities/dao.rs` — a change recorded with an `activity_id` reads back with it; `record_sandbox` returns `Ok(false)` and allocates no position when the status restates the row before it.
- `ui_rpc/projection.rs` — `change_events` on an activity-named change emits the activity and no `session_set`; the existing assertion that every `activity.tone` is one of the contract's four literals must still pass with `runtime.error` and `runtime.warning` in play, and a matching assertion should be added for the kind strings.
- `ui_rpc/feed.rs` — two changes naming the same `activity_id` coalesce to one; a change naming neither still yields `session_set`.
- `task_activities/mapping.rs` — `lifecycle_descriptor` for one event of each severity, the no-`lifecycleEvent` fallback, and the unknown-slug fallback; `sandbox_activity` composes a `detail` carrying node and pending for a `pod_scheduled` payload.
- `task_activities/view.rs` — the three kind mappings.

End to end, against a live deployment: create a thread with a first message, watch the T3 chat, and confirm `Scheduled` and `Ready` appear before the agent's first word. `GET /api/v1/tasks/{id}/events` is the ground truth for what was published.

## Open questions

**A healthy provisioning is now one or two rows.** With the Sandbox's own init events out of scope, a fast startup contributes `pod_scheduled` and `pod_ready` and nothing else. That may be too quiet to be worth the chat space, and is only judgeable once it is on screen.

**Threads that already ran.** No backfill, so an existing thread shows nothing until its Sandbox next moves.

**The Sandbox's own init events.** Thirteen publish sites in `sandbox/src/init/mod.rs`, three of them failures (`CredentialsFailed`, `RepositoryCloneFailed`, `SetupFailed`). All thirteen carry sub-type `initializing` and are stored with `is_error = false`, because the flag is derived from the sub-type alone (`event_ingress/mod.rs:299`); the only thing distinguishing a failure is `payload.eventType == "Warning"`. Bringing them in means deciding where that signal is read — and it must not be done by changing the sub-type, since a `failed` sub-type parses as `SandboxStatus::Error` and reaches `handle_sandbox_fatal`, which replies to the agent as if the Sandbox had crashed. A failed setup command is not fatal; the Sandbox goes on to Ready.
