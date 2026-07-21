# Message assembly is server-owned

A **Message** is assembled into its UI-facing form (`UiMessage`) in exactly one
place: a pure `assemble(rows) -> Vec<UiMessage>` module on the backend. Both the
paginated read path (`GET /messages`) and the live stream cross this one seam.
Assembly means: fold content blocks by `message.id` (admitting `usage` once),
inline `tool_result`s into the assistant Message that called them, flag
interactive tools (`AskUserQuestion`/`ExitPlanMode`), and link subagents by
`parent_tool_use_id`.

The Sandbox forwards one event per content block; it does not coalesce. The
persistence consumer is the single writer: after persisting a source event's
block(s), it re-runs the assembler over the affected Message's rows and publishes
a **derived** `message.upsert` / `message.deleted` carrying the assembled
`UiMessage`. Derived events are UI-only — never written to `task_events`. Raw
`agent.message` stays the **source event** that feeds history and the Events tab.
Message identity on the wire is the persisted `uuid`.

The frontend renders; it does not assemble. It applies upsert-by-`uuid` to the
messages cache. Live optimism comes from the per-Message upserts; a single
newest-page reconcile on `taskTurn.completed`/`error`, plus mount/reconnect
hydration, covers upserts dropped by the lossy firehose.

## Considered options

**Client-side assembly.** A Rust assembler for the read path and a separate
TypeScript assembler for the live stream, reconciled by a debounced refetch.
Rejected: two assemblers over one concern drift (the TS twin omitted inlined
`tool_result`s), the streamed identity (`evt-<eventId>`) differed from the
persisted `uuid` and forced a second dedup rule, and the interactive-tool list
was hand-synced across three files.

**Sandbox-side coalescing (`AssistantBuffer`).** Keep the Sandbox merging content
blocks into whole assistant Messages before publishing. Rejected as the assembly
home: a DB-backed assembler sees all blocks at once and folds them by `message.id`
statelessly, so the buffer's order-sensitive release logic, `usage_seen` dedup,
and `flush_pending` drain are unnecessary. The Sandbox forwards per block instead.

**Stateful streaming assembler.** Keep per-task assembly state in the backend and
emit fine-grained field patches. Rejected: the in-memory buffer must survive
restarts and replica rebalancing, and field patches reintroduce a merge step on
the client — the exact seam that diverged before. A stateless full upsert keyed on
`uuid` is idempotent and self-healing.

## Consequences

Deleted: `AssistantBuffer` + `usage_seen` + `flush_pending` (Sandbox);
`uiMessageFromAgentEvent` + `appendStreamedMessage` + the `evt-<eventId>` identity
+ the per-Message refetch (frontend); the three-file interactive-tool sync. The
interactive-tool list and content-block type mapping move to shared enums in
`api-schemas`, consumed by the assembler and generated for the frontend.

New: a derived `message.upsert` / `message.deleted` event pair on the SSE relay,
distinct from source events and excluded from `task_events`. The persistence
consumer gains the assemble-and-emit step. Message rows are upserted as blocks
stream in, so the `task_messages` `ON CONFLICT (uuid)` path must COALESCE `usage`
and replace child rows correctly.

Event volume on the bus rises ~1–4× (one event per content block rather than per
merged response). The Sandbox→backend event contract changes shape: per-block
payloads carrying `message.id` and `parent_tool_use_id`.
