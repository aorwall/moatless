# Plan: Push enriched `UiMessage` on message events (kill the active-chat refetch)

## Status

- **Phase 1 (done, shipped in this PR)** — Observer gate: message events only
  refetch the task whose chat view is actually mounted. Background/sidebar-only
  tasks are marked stale (`refetchType: 'none'`) and reload lazily on open.
- **Phase 2a (done, shipped in this PR)** — Debounce: the active chat's
  newest-page refetch is coalesced into one trailing refetch per ~200 ms during
  a streaming turn; turn-final and interactive events flush immediately.
- **Phase 2b (this doc — NOT built)** — Eliminate the active chat's refetch
  entirely by having the backend push a fully-enriched `UiMessage` in the
  message event so the frontend upserts into cache with **zero** HTTP
  round-trips.

**Why this is a separate doc:** Phase 2b changes the chat-rendering **tool-result
join semantics**, the most visible surface in the app. It must be validated with
the agent-tier E2E suite (`CLAUDE_SDK_E2E=1`, real Claude CLI + sandbox), which
was not available in the environment where Phases 1/2a were written. Run this in
an agent that can execute the agent-tier tests.

## Background (current data flow)

- The sandbox streams Claude SDK messages → backend gRPC `PublishEvent`
  (`backend/src/sandbox/grpc_server.rs`) → `EventPublisher` builds a
  `StreamEvent` and `XADD`s it to Redis (`backend/src/events.rs`) → SSE
  broadcast to the frontend, **payload unchanged** (raw SDK envelope).
- Separately, `handle_agent_message` (`backend/src/event_handler/agent_handlers.rs:561`)
  persists the message to `task_messages` (+ content blocks, contexts).
- The frontend `GET /tasks/:id/messages` returns `UiMessage[]`, produced by
  `transform_to_ui` (`backend/src/message/transform.rs`) — a DB-join transform
  that, among other things, **folds a tool_result message into the `toolResults`
  of the earlier assistant message that issued the matching `tool_use`**
  (`transform.rs:56-110`).

The `agent.message` SSE payload is the raw SDK message; it is NOT a `UiMessage`.
That shape gap is the only reason the frontend refetches instead of inserting the
event directly.

### The hard part: deferred tool-result join

A single turn produces messages in this order:

1. assistant message with a `tool_use` block (id `T`) — at this moment there is
   **no** result yet, so `toolResults` is correctly empty.
2. (tool runs)
3. user/system message with a `tool_result` block referencing `tool_use_id = T`.

`transform_to_ui` joins (3) back into (1): the assistant `UiMessage.toolResults`
gains an entry, and the standalone tool_result message is not rendered on its
own. So an upsert keyed on `uuid` that only emits the message being persisted is
**insufficient** — when (3) persists we must also re-emit the **parent assistant
message** (1) so its `toolResults` fills in. Otherwise the active chat shows a
tool call stuck in "pending" until the next full refetch.

## Design

Emit a new SSE event `message.upserted` carrying one enriched `UiMessage`. The
frontend upserts it into the `tasks.messages` infinite query by `uuid`
(replace-in-place if present, else append to the newest page). Emit it from the
backend **after** DB persistence so the transform sees committed rows.

### 1. Backend — emit enriched `UiMessage` after persist

File: `backend/src/event_handler/agent_handlers.rs`, in `handle_agent_message`
after `state.messages.append(...)` succeeds (around line 756).

- Build the `UiMessage` for the just-persisted row. Reuse the existing
  transform rather than hand-rolling: fetch the small set of rows needed for a
  correct single-message transform and call `transform_to_ui`.
  - For an **assistant** message: transform just this row (+ its content blocks
    + contexts). `toolResults` will be empty — correct at this point.
  - For a **tool_result** (system/user) message: it resolves an earlier
    assistant message. Resolve the parent assistant `uuid` via the
    `tool_use_id` on the tool_result block, re-transform **that** assistant row
    (now that the result row exists it joins in), and emit a `message.upserted`
    for the **parent assistant** `UiMessage`. (Also consider whether the
    tool_result row itself should be emitted — today it is folded away, so
    usually only the parent needs emitting.)
- Publish via the existing `EventPublisher` with a new subtype. Keep the raw
  `agent.message` event flowing unchanged for now (metrics, other consumers) —
  `message.upserted` is additive.
- Consider a single helper, e.g. `message::transform::transform_single(...)`,
  that takes a message id/uuid, loads (message row, its content blocks,
  contexts, and any tool_result rows referencing its tool_use ids) and returns a
  `UiMessage`. Prefer extending `transform.rs` so the join logic stays in one
  place. Add unit tests there for: plain assistant, assistant+deferred
  tool_result, user text message.

### 2. Event schema (Rust + TS)

- `crates/events` — add the `message` event type / `upserted` subtype (mirror
  how existing typed events are declared) with a `UiMessage` payload.
- TS mirror: `packages/events` and/or `packages/api-client/src/sandbox/model`
  — add the event + reuse the existing `UiMessage` model so no shape drift.
- Frontend `apps/frontend/src/lib/types/events.ts` — add the discriminated-union
  variant.
- Ensure the SSE `types=` filter (default subscription) includes the new type
  (`global-event-stream-provider.tsx` `buildStreamUrl` / the default `types`
  list) so clients actually receive it.

### 3. Frontend — upsert instead of refetch

- Add `upsertMessageInCache(queryClient, taskId, uiMessage)` next to
  `refreshNewestMessagesPage` in
  `apps/frontend/src/features/tasks/hooks/use-tasks.ts`:
  - `setQueryData` on `queryKeys.tasks.messages(taskId)`; find the message by
    `uuid` across pages — replace in place if found, else append to the **last**
    page (newest). Preserve `pageParams`. No network.
  - Only mutate when the query has data (chat mounted); if absent, no-op (the
    observer gate + lazy load already covers the not-mounted case).
- In `apps/frontend/src/features/events/utils/event-handlers.ts`:
  - Handle `message.upserted` → `upsertMessageInCache` (observer-gated, same as
    today's gate).
  - **Remove `agent.message` from `MESSAGE_REFRESH_EVENT_TRIGGERS`** and remove
    the tool-trigger refetch — those are now covered by upserts.
  - Keep `taskTurn.completed`/`error` as a cheap **reconciliation** refetch
    (single fetch per turn) to self-heal any missed/edited/out-of-order upsert.
  - The Phase 2a debounce can then be deleted (no more streaming refetch to
    coalesce), or kept only for the reconciliation path (not needed — it's
    already once-per-turn).

### 4. Ordering / idempotency

- SSE can deliver out of order or duplicate. Upsert-by-`uuid` is idempotent for
  replacement. For append, guard against inserting a `uuid` that already exists
  on any page.
- The `taskTurn.completed` reconciliation refetch is the backstop for any
  dropped event.

## Files to touch

- `backend/src/event_handler/agent_handlers.rs` — emit `message.upserted`.
- `backend/src/message/transform.rs` — `transform_single` helper + tests.
- `backend/src/message/dao.rs` — a targeted fetch (message + blocks + contexts +
  related tool_result rows) if not already expressible.
- `backend/src/events.rs` — publish path already generic; just a new subtype.
- `crates/events/**` — typed event + payload.
- `packages/events/**`, `packages/api-client/**` — TS mirror.
- `apps/frontend/src/lib/types/events.ts` — union variant.
- `apps/frontend/src/features/tasks/hooks/use-tasks.ts` — `upsertMessageInCache`.
- `apps/frontend/src/features/events/utils/event-handlers.ts` — route
  `message.upserted`; drop `agent.message` refresh triggers; keep turn-final
  reconciliation.
- `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx`
  — ensure the new type is in the default subscription.

## Verification (requires E2E)

1. `cargo test -p moatless-vibe-backend` — transform unit tests (plain
   assistant, assistant + deferred tool_result join, user text).
2. `cargo clippy -p moatless-vibe-backend -- -D warnings`.
3. Frontend: `bun run type:check && bun run biome:check`; unit tests for
   `upsertMessageInCache` (replace-in-place, append-new, dedupe by uuid) and the
   updated `event-handlers` routing.
4. **Agent-tier E2E** (`CLAUDE_SDK_E2E=1`, real Claude CLI + sandbox) — the
   critical gate:
   - Run a multi-step tool-using turn; assert the chat renders assistant text,
     tool calls, AND their results **without** a full messages refetch on each
     block (observe network / add an instrumentation assertion).
   - Assert the deferred tool_result correctly joins into the parent assistant
     message (no permanently-pending tool call).
   - Assert plan mode (`ExitPlanMode`) and `AskUserQuestion` still surface the
     interactive prompt promptly.
   - Assert `taskTurn.completed` reconciliation leaves the final transcript
     identical to a fresh `GET /messages`.
5. Manual: multiple concurrent running tasks — confirm only upserts flow for the
   on-screen chat and background tasks issue no message fetches until opened.

## Rollback / safety

- `message.upserted` is additive; the raw `agent.message` event and the
  `GET /messages` endpoint are unchanged. If the upsert path misbehaves, revert
  the frontend trigger change (restore `agent.message` → debounced refetch from
  Phase 2a) and the chat returns to the refetch model with no backend rollback
  required.
