# The agent.message event is a skeleton; top-level assembly resolves the affected parent; subagent transcripts are lazy

Message content is written **once**, to `task_messages`. The full `agent.message`
content is a **transient ingestion payload**: the Sandbox emits it to the Redis
stream, the event-handler consumer reads it once to build `task_messages`, and it
is never persisted to `task_events` nor relayed to clients in full. The persisted
and client-facing form of `agent.message` is a **metadata skeleton** —
`{ message: { role }, uuid, blockCount, toolNames }`. Message *content* reaches the
Chat only through the derived `message.upsert` (top-level Messages) or the
on-demand subagent transcript endpoint (subagent Messages).

**Assembly resolves the affected parent.** A persisted top-level row that folds
into another Message — a `tool_result`/`system` row inlined into the assistant
Message that called it — has no standalone `UiMessage`. The derived upsert targets
that **parent's** `uuid`, so a top-level tool result renders live under its tool
call instead of waiting for the turn-final reconcile. Read (`GET /messages`) and
live (`message.upsert`) cross **one** `assemble()` seam that owns exact loading,
file hydration, shaping, and affected-parent resolution.

**Subagent transcripts are lazy.** The top-level read carries only a small,
derived **summary** per `Agent` tool call — `{ messageCount, tokens, status }` on
the `Agent` content block — and the collapsed card renders from that alone. The
top-level Messages read excludes subagent rows (they carry `parent_uuid`, the
spawning `Agent` tool-use id). The full subagent transcript is fetched **on demand**
from `GET …/tool-calls/{tool_use_id}/messages` when the card is expanded. Live, a
subagent step emits a tiny `message` / `subagentProgress` derived event — the same
summary shape, absolute values — that the frontend writes through onto the cached
parent's `Agent` block; an expanded card refetches its transcript on each tick. No
subagent transcript content ever rides the firehose, and no growing subtree is
re-shipped.

`toolNames` on the skeleton is the source-event signal for transport reactions:
files-invalidation reads it as a field, staying on `agent.message` — the derived
`message.upsert` remains purely the Chat's rendering concern.

## Considered options

**Keep the raw `agent.message` log (forensic ground truth).** Persist and relay
the full per-block content, as ADR-0002 left it. Rejected: after ADR-0002 that
content is diagnostic-only — the lifecycle timeline reads only `role`
(`to-lifecycle-event.ts`: "leave the content to the chat view"), and no consumer
reconstructs state from it. `task_events` never prunes, so the largest payloads
accumulate forever as a second copy of what `task_messages` already holds.

**Slim new rows only, leave existing rows fat.** Rejected: `task_events` is
append-only with no retention, so the historical bulk never reclaims. The change
backfills existing `agent.message` rows to skeletons; their assembled content
survives in `task_messages`.

**Invalidate the files query off `message.upsert`.** Rejected: it couples a
transport reaction (cache invalidation) to the Chat's rendering stream. Keeping
`toolNames` on the skeleton keeps invalidation on the source event, preserving the
source-event/derived-event layering.

**Targeted parent-upsert fix only.** Rejected for top-level assembly in favour of
the full seam: it would keep the imprecise newest-100 assembly window. The full
seam uses a dedicated affected-Message loader.

**Server-side subagent nesting delivered in the parent Message upsert.** Assemble
the whole subagent subtree into the parent `UiMessage` (`subagentsByToolUseId`) and
re-emit the parent on every subagent step. Rejected: because the frontend replaces
a Message wholesale by `uuid`, each subagent step must carry the current full
transcript, so the parent upsert re-ships the growing subtree — `O(N²)` bytes over
a run on the live wire — and the top-level read carries subagent content most of
which is never expanded. The lazy model ships each fact once: a small absolute
summary on the wire, full transcript only on demand.

## Consequences

- `task_events` stores a skeleton for `agent.message`; the table's write volume
  and size drop (that event is its largest payload). The Events timeline and
  `moat events --json` show the skeleton — `role`, tool names, block count — and
  link to content in the Chat by the same `uuid`.
- Clients receive top-level message content **once** (`message.upsert`); the live
  wire stops double-shipping content as raw `agent.message` *and* assembled
  `message.upsert`.
- Top-level tool results render live under their tool call.
- The assembler owns top-level Message shape. Subagent transcripts are summarized
  on the read path and fetched on demand; the frontend `group-messages.ts`
  grouping is deleted (the frontend renders `subagentSummary` and fetches the
  transcript, it does not assemble).
- The live wire never carries subagent transcript content; a `subagentProgress`
  event keeps the collapsed `Agent` card's count/tokens current, and the read-path
  summary and the progress event share one shape (one source of truth for the
  card, reconciled by construction).
- Readers tolerate both payload shapes only transiently — the backfill removes the
  fat rows, after which one shape remains.
- `DELETE /messages/{message_id}` and its `message.deleted` event are removed with
  the frontend queued-cancel affordance; the pending-message list becomes
  view-only.
- **Trade-off accepted (forensic log):** the raw per-block emission log is gone, so
  "what did the Sandbox send, and in what order" is no longer reconstructable from
  persisted state. The assembled Message in `task_messages` is the canonical record,
  and the raw log had no reader — so the forensic loss is deemed acceptable.
- **Trade-off accepted (expanded liveness):** an expanded subagent transcript
  refreshes by debounced refetch on progress ticks, not message-by-message
  streaming — a small lag on a user-initiated detail view, in exchange for never
  streaming subagent content.
- Refines ADR-0002, which kept raw `agent.message` as the persisted source event
  feeding history with full content. This reduces it to a skeleton and completes
  server-owned assembly for top-level Messages (affected-parent upserts).

## Domain-model impact

Three glossary entries in `CONTEXT.md` sharpen when this lands (updated with the
code, not ahead of it):
- **Source event vs derived event** — a source event's full content is a transient
  ingestion payload; its persisted and client-facing form is a metadata skeleton.
  Message content reaches clients only via the derived event or the on-demand
  transcript endpoint.
- **Message upsert** — a derived upsert targets the affected *parent* Message; a
  top-level tool result upserts the assistant Message that owns it.
- **Subagent transcript** — lazy: a per-`Agent`-call summary rides the read path
  and a `subagentProgress` event; the full transcript is assembled server-side but
  fetched on demand, never nested into the parent Message nor streamed on the wire.
