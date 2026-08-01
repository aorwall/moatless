# Storing a conversation as messages and activities

> **Status:** shipped
> **Updated:** 2026-08-01
> **Vocabulary:** CONTEXT.md

## Current solution

A Task's conversation is stored as rows in `task_messages`, each holding a jsonb array of Anthropic
content blocks in `message_content` (`backend/migrations/0001_schema.sql:158`), plus a second,
partial copy of the same blocks as rows in `task_message_content_blocks`
(`backend/migrations/0001_schema.sql:215`). Nothing in the schema distinguishes what somebody said
from what the agent did on the way to saying it. Both readers of the conversation compute that
distinction themselves, on every read.

All measurements below were taken against the shared dev database on 2026-07-31. PostgreSQL 17.4.

### Where a conversation's state lives

| State | Table / column | Written by | Read by | Size |
|---|---|---|---|---|
| Every block of every message, as received | `task_messages.message_content` jsonb | `event_handler/agent_handlers.rs:856` (per-block read-modify-write splice) | `message/transform.rs`, `ui_rpc/projection.rs`, `ui_rpc/dao.rs` | 3,080 MB over 2,627,130 rows |
| A partial second copy, one row per renderable block | `task_message_content_blocks` | `agent_handlers.rs:1207` | `message/dao.rs:1080`, `:1186`, `:1258`, `task/statistics.rs:84` | heap 624 MB + TOAST 1,038 MB + indexes 199 MB |
| The full text of a tool result | `task_message_content_blocks.tool_result_content` | same, always with `show_in_ui = false` (`agent_handlers.rs:1236`) | selected by all three assembly reads, then discarded by every consumer | 1,104 MB |
| Message text | `task_message_content_blocks.text_content` | same | assembly | 147 MB |
| A skeleton of every assistant message | `task_events`, `type = 'agent.message'` | event ingress | the Events tab timeline | 163,141 rows of 1,763,348 |
| A turn's outcome | `task_turns.status` — `active \| completed \| error \| stopped \| interrupted` (`backend/migrations/0071_task_turns.sql:13`) | `task_turn/dao.rs` | `ui_rpc/dao.rs:369` selects `turn.status` | one row per turn |
| A conversation's ordering counter | `task_conversation_position.position` | `task_conversation::record_*_change` | the feed | one row per Task |

### What is actually in `message_content`, by block type

| Block | Rows | Bytes | Rendered today? |
|---|---|---|---|
| `text` | 1,598,492 | 239 MB | Yes |
| `tool_result` | 465,696 | 2,301 MB | Yes, in full — `apps/frontend/src/features/tasks/components/chat/message-history/tool-calls/generic-tool-call.tsx:124` applies no truncation |
| `tool_use` | 464,995 | 228 MB | Yes |
| `thinking` | 124,103 | 301 MB | No. Nothing renders the thought — only 19,993 blocks carry any text at all, averaging 110 characters; the remaining bytes are redacted-thinking signatures |
| `command_execution`, `turn_result`, `agent_message`, `file_change` | 1,597 | 3.5 MB | Partly |

140,056 tool results — 30.1% — exceed 2,000 characters, with 1,875 MB above that mark.

### What is actually in `task_messages`, by `message_type`

`message_type` is the axis both readers branch on. `message_role` is a second, coarser column and is
not the one the code uses.

| `message_type` | Rows | With prose | What it is |
|---|---|---|---|
| `assistant` | 794,145 | 229,024 | The agent speaking. The other 565,121 carry only `tool_use` blocks. |
| `user` | 52,758 | 52,646 | A person writing. 2.0% of the table. |
| `system` | 474,018 | 7,860 | 466,158 are tool-result carrier rows. The 7,860 with prose are compaction summaries and injected skill text. |
| `result` | 1,165,256 | 1,164,948 | 1,162,114 are the identical string "Stopped by user request". |
| `error` | 141,497 | 141,497 | 137,189 are "Sandbox terminated: Container is in CrashLoopBackOff". |

Both large numbers are single incidents, not workload. 1,163,898 of the 1,165,256 `result` rows were
written in April 2026 across 395 Tasks; 137,193 of the 141,497 `error` rows in February 2026 across
three Tasks. Every other month runs a few hundred of each.

153 rows hold a bare jsonb string rather than an array. Every one of them is the literal text `{}`,
all written in one week in February 2026 across 33 Tasks.

Folded to prose — `string_agg` over the `text` blocks — the whole table is 193 MB, averaging 127
characters and peaking at 847,553.

### The trace: an assistant turn's blocks arriving, in seven hops

1. **The Sandbox publishes one event per block.** `sandbox/src/agent/coordinator.rs:17` states the
   contract set by ADR-0002: each `Signal::Source` is published as its own `agent.message` carrying
   "a native block plus its `blockIndex`, grouped by `uuid`". The index is allocated at
   `coordinator.rs:314`. A `tool_use` block is therefore already a call-start event; what Moatless
   lacks is t3's *contentless* start, not the event.
2. **Ingress durably queues it.** ADR-0006's outbox, transient by design.
3. **The consumer dispatches it.** `event_handler/consumer.rs` routes by `EventKey`. Sandbox status
   events already branch on exactly `Initializing | Ready | Stopped` for the Linear adapter
   (`consumer.rs:546`).
4. **The handler splices the block into the Message.** `agent_handlers.rs:856` reads
   `message_content`, merges the arriving block at its index, and writes the array back. This is a
   read-modify-write on a shared JSON document, once per block.
5. **The handler writes a block row.** `agent_handlers.rs:1207` maps the block to a
   `task_message_content_block`. Four block types never get a row at all —
   `UNRENDERED_BLOCK_TYPES = ["thinking", "redacted_thinking", "server_tool_use",
   "web_search_tool_result"]` (`agent_handlers.rs:1144`) — and a `tool_result` row is always written
   with `show_in_ui = false` (`agent_handlers.rs:1236`).
6. **A reader assembles the conversation.** `message/transform.rs` walks the Messages, inlines each
   tool result into the assistant Message that called for it, and drops the carriers. The sibling
   lookup is an `EXISTS` over the block table keyed on `tool_use_id` (`message/dao.rs:1155`).
7. **Or the other reader assembles it differently.** `ui_rpc/projection.rs:560` `rendered_transcript`
   walks `message_content` directly and produces two collections — messages and activities.

### The second reader, and what it already proves

`backend/src/ui_rpc/projection.rs:514` states the model this spec adopts, in the code, today:

> T3 splits a conversation the way a reader does and Moatless does not: what somebody said is a
> message, and what the agent did on the way to saying it is an activity.

It computes that split on every read, and in doing so already applies the exclusions this spec makes
into storage rules:

- `projection.rs:604` — a `system` row contributes no bubble: *"The carrier rows for tool results.
  Their content is already attached to the call it answers, and rendering them as their own bubbles
  would put every tool's raw output in the conversation."*
- `projection.rs:610` — a `user` row carrying tool results contributes no bubble: *"the SDK streaming
  results back, not a person writing. `message/transform.rs` makes the same exclusion for the same
  reason."*
- Every branch drops a message with empty text.
- `projection.rs:622` — `Result` and `Error` rows become a `system`-role bubble: *"The run's own
  closing line … Neither the person nor the agent said it."*
- `projection.rs:634` `message_activities` — only an assistant Message contributes activities;
  `thinking` and `tool_use` are the two kinds.
- `projection.rs:736` and `:795` — an activity's id is `{message_uuid}:{block_index}`.
- `projection.rs:797` — `"kind": if outcome.is_some() { "tool.completed" } else { "tool.updated" }`.
- `projection.rs:827` `tool_outcomes` — results arrive on later rows than the call, so the outcome is
  matched by `tool_use_id`, not by position.
- `projection.rs:936` `TOOL_DETAIL_LIMIT = 2_000`, with its reason: *"a subscription re-sends the
  snapshot every time the conversation moves"*.
- `projection.rs:550` `activity_sequence = message.id * BLOCKS_PER_MESSAGE + block_index`, with
  `BLOCKS_PER_MESSAGE = 1_024` (`projection.rs:554`) — a synthetic ordering invented because
  activities have no position of their own.
- `ui_rpc/dao.rs:282` `TRANSCRIPT_LIMIT = 1_000` — the transcript read is capped at 1,000 Messages.

The equivalent split is what t3 stores natively: `projection_thread_messages` holds `role` and a flat
`text` and no blocks, no tool calls and no tool results; `projection_thread_activities` holds
`(activity_id, tone, kind, summary, payload_json)`.

### Dead paths, and paths that exist only to be discarded

- `task_events.event_type` is NULL on all 1,763,348 rows. The column in use is `type`.
- `tool_result_content` — 1,104 MB — is selected by all three assembly reads and then dropped by
  every consumer, because the same rows carry `show_in_ui = false`.
- `thinking` blocks are 301 MB in `message_content` and are rendered by nothing. The Moatless
  frontend has no thinking branch; `ThinkingIndicator` is a live spinner. The T3 surface deliberately
  carries only the word "Thinking" (`projection.rs:729`).
- `StreamEvent(PartialAssistantMessage)` is parsed by the SDK
  (`crates/claude-agent-sdk/src/types/messages.rs:173`) and forwarded nowhere.
- Sandbox lifecycle rows repeat: 92,165 of 139,595 restate the status before them. The intent path
  already guards against this with an `IS DISTINCT FROM` check around `record_session_change`
  (`backend/src/sandbox/lifecycle/dao.rs:290`); the event path has no such guard.
- `stop_marker_text` (`backend/src/message/models.rs:18`) still exists, but `stop_is_reported`
  (`:35`) already restricts it to error stops, with the reason stated in full: *"The other reasons
  are routine lifecycle — the user hit stop, the idle reaper or cleanup reclaimed the sandbox, the
  system stood it down — and the reader already knows: the task's live status and turn state say so,
  and for externally-bound tasks a 'Stopped by user request' line is pure noise on Slack or Linear."*

## Planned solution

**New**

- `task_activities` — one row per tool call carrying its own outcome, per stretch of thinking, and
  per Sandbox status change or error.
- `task_messages.message_text` (`text`) — a Message's prose, as prose.
- One block-to-row mapping shared by the live writer and the migration, so the two cannot drift.
- A batched migration that folds prose, deletes the rows that are not conversation, and drops
  `task_message_content_blocks` and `task_messages.message_content`.

**Changes materially**

- `event_handler/agent_handlers.rs` — a block becomes a typed row instead of a splice into a shared
  JSON array; only `text` still folds.
- `task_messages` — role and prose only. A row exists when a person or the agent said something.
  2,627,130 rows become 281,670; 3,080 MB of jsonb becomes 96 MB of prose.
- `message/transform.rs` and `message/dao.rs` — conversation assembly returns two ordered collections
  instead of one assembled blob per Message.
- `ui_rpc/projection.rs` — `rendered_transcript` reads the two collections instead of computing them
  from `message_content` on every read. Same rows out.
- `task/statistics.rs` — reads typed columns instead of scanning blocks.
- `event_handler/consumer.rs` — a Sandbox status event becomes an activity, on the branch that
  already exists for Linear.
- `task_events` — 163,141 `agent.message` rows deleted and never written again; it becomes
  lifecycle-only.
- `message/models.rs` — `stop_marker_text` and the `Result` row it feeds are removed.
- The conversation feed ships (PR #176's counter allocator).

**Untouched**

- What a Message *is* to anything outside the conversation — same uuid, same identity, same ordering.
- The Sandbox and what it sends. ADR-0002's per-block `agent.message` contract is unchanged.
- The ingress outbox and its delivery guarantee (ADR-0006).
- Every current-state query — pending user messages, session binding, the shell listing — which reads
  scalars, not content.
- The Events tab, which keeps every event type including the ones that never become activities.
- `task_turns` and its status vocabulary.
- Subagent conversations as a feature: they keep their own rows and their own read path, though
  Decision 13 prunes those rows the same way.
- Replay. It was on the table and is deliberately not being built — Decision 12.

### Problem

**For the person reading a conversation.** A tool result is fetched off disk twice and rendered once.
Thinking is stored at 301 MB and rendered never. A conversation cannot be asked a question — "which
tool calls failed" is a scan of a JSON blob, not a query.

**For whoever writes the conversation.** Every arriving block is a read-modify-write against a shared
JSON document (`agent_handlers.rs:856`). Two blocks that race splice over each other, and there is no
second copy to repair from. The block table is a partial second copy that silently drops four block
types, so it cannot be that repair either.

**For whoever operates it.** `task_messages` is 4,143 MB, of which 3,080 MB is jsonb and roughly half
the rows are two repeated sentences from two incidents. The largest single column, 1,104 MB of
`tool_result_content`, is read on every conversation load and thrown away every time.

**For the T3 Code client.** Everything it needs already exists, computed on each read by walking the
same jsonb (`projection.rs:560`). It pays that walk on every snapshot, and a subscription re-sends the
snapshot every time the conversation moves.

### Behaviour

**The person reading a conversation in Moatless**

- Given a Task written after the migration, when its conversation is opened, then it shows the same
  messages, tool calls, results and thinking it shows today.
- Given a tool call, when it is expanded, then the full result text appears, uncapped, exactly as it
  does today.
- Given a Task written before the migration, when its conversation is opened, then it shows its
  messages and none of its tool calls, thinking or Sandbox lifecycle, and its statistics say the Task
  predates the change rather than reporting zero tool calls.
- Given a turn the user stopped, when the conversation is read, then no message says "Stopped by user
  request" and the turn's status is `stopped`.
- Given a Sandbox that failed fatally, when the conversation is read, then the exit code, terminated
  reason, disruption cause and container logs appear on one Sandbox activity.
- Given a Sandbox that reports the same status twice in a row, when the conversation is read, then it
  appears once.

**The paired T3 Code client**

- Given any Task with activities, when it reads the conversation, then it receives the same messages
  and activities `rendered_transcript` returns today, with the same ids, kinds, tones and summaries.
- Given a tool result of any size, when a snapshot is built, then the detail it carries is at most
  2,000 characters and is marked as truncated.
- Given a stored thinking activity, when it reaches the T3 surface, then it is sent as
  `task.progress` — the only kind that client renders in the thinking tone.
- Given a Sandbox activity, when it reaches the T3 surface, then its kind ends in neither `.started`
  nor `.completed`, so it claims no position in that client's lifecycle sort.

**The agent writing the conversation**

- Given a `tool_use` block, when it is received, then one activity row is written with status
  `in_progress`.
- Given the matching `tool_result` block, when it is received — on a later Message than the call —
  then that same row is updated to `completed` or `failed`, and no second row appears.
- Given the same block delivered twice, when the second copy is received, then the row is unchanged
  and no duplicate is written.
- Given a block set with no `text` block, when it is written, then no Message row is created.

**The operator running the migration**

- Given the migration is interrupted, when it is restarted, then it resumes and produces the same
  result.
- Given the migration is run twice, when it finishes, then no row is duplicated.
- Given the reclaim is running, when a conversation is opened, then it stalls; everything that does
  not read conversation content keeps working.

### Architecture and key flows

| Module | Observed | Proposed responsibility | Proposed change |
|---|---|---|---|
| `backend/src/event_handler/agent_handlers.rs` | Splices each block into `message_content` (`:856`) and writes a partial block row (`:1207`), dropping four types (`:1144`) | Turn one arriving block into exactly one typed row | Replace the splice and the block write with one call into the block-to-row mapping. Only `text` still folds, now onto `message_text`. |
| `backend/src/event_handler/` — new module for the mapping | — | Decide what an arriving block becomes | New. One function, called by the live writer and by the migration, so the two cannot drift. |
| `backend/src/message/dao.rs` | Assembly selects blocks and inlines results by an `EXISTS` on `tool_use_id` (`:1155`); `get_subagent_aggregates` sums usage over Messages (`:1206`) | Return two ordered collections | Replace the block joins with reads of `task_messages` and `task_activities`. |
| `backend/src/message/transform.rs` | Inlines tool results and skips carriers (`:184`, `:196`) | Shape the two collections for the frontend | The skips become unnecessary — the rows no longer exist. |
| `backend/src/ui_rpc/projection.rs` | Computes messages and activities from `message_content` on every read (`:560`) | Project stored rows onto the T3 contract | Keeps the contract, loses the walk. `activity_sequence` (`:550`) and `BLOCKS_PER_MESSAGE` are deleted: activities carry a real position. |
| `backend/src/task/statistics.rs` | Derives failed tool ids from block rows (`:84`) | Count from typed columns | Read `task_activities.is_error`. Report "predates the change" for a Task with no activities. |
| `backend/src/event_handler/consumer.rs` | Dispatches Sandbox statuses for Linear (`:546`) | Also write a Sandbox activity | Add the second consumer on the branch that exists, with the `IS DISTINCT FROM` collapse guard the intent path already uses (`sandbox/lifecycle/dao.rs:290`). |
| `backend/src/message/models.rs` | `stop_marker_text` (`:18`), `stop_is_reported` (`:35`) | — | Both removed with their call site. |
| `backend/src/task_conversation/` | Allocates a per-conversation position | One ordering shared by messages and activities | Allocate once per transaction for two writers. PR #176 provides it. |
| `backend/migrations/` | — | The fold, the delete, the drops, the reclaim | New migration plus an operator runbook. |

#### Flow 1 — an assistant turn's blocks arriving (the main success path)

1. The Sandbox publishes one `agent.message` per block, carrying the block and its `blockIndex`
   (`coordinator.rs:314`, ADR-0002).
2. Ingress persists it to the outbox (ADR-0006) and the consumer dispatches it.
3. The handler opens one transaction and allocates the conversation position once.
4. The mapping decides what the block becomes:
   - `text` → appended to `message_text` on the Message for this `uuid`, creating the Message row if
     this is its first `text` block.
   - `tool_use` → one activity, `kind = tool_call`, `status = in_progress`, `activity_id =
     {message_uuid}:{block_index}`, `tool_use_id` from the block, `payload` holding the tool name and
     input.
   - `thinking` / `redacted_thinking` → one activity, `kind = thinking`, summary only.
   - `tool_result` → `UPDATE task_activities … WHERE (task_id, tool_use_id) = (…)`, setting `status`,
     `is_error` and the full result into `payload`.
   - anything else → nothing, and no warning.
5. The Message and its activities commit together. A reader never sees one without the other.

#### Flow 2 — a tool result arriving on a later Message (the consistency boundary)

`projection.rs:827` establishes that a result lands on a different Message row than its call.
Therefore the result cannot derive the call's `{message_uuid}:{block_index}` key. It finds the row by
`(task_id, tool_use_id)`, which is unique. This is what makes one-row-per-call possible and why that
uniqueness is an index, not a convention.

If no row matches — the call was never received, or the Task predates the migration — the result is
discarded, nothing is created, and nothing errors.

#### Flow 3 — a Sandbox status event becoming an activity

The consumer already branches on `Initializing | Ready | Stopped` (`consumer.rs:546`). It gains a
second consumer that writes an activity, guarded so a status identical to the conversation's previous
Sandbox activity writes nothing. The event row is recorded either way, so the Events tab is
unaffected. Applied to today's history this takes 139,595 rows to 46,741.

A fatal Sandbox failure is already published with its full detail —
`backend/src/sandbox/k8s/pod_watcher.rs:803` puts the same `detailed_message` (message, exit code,
terminated reason, node, disruption cause, container logs) into both the `message` and `error` fields
of a `SandboxStatus::Error` payload. That becomes a `sandbox_error` activity. This is why Decision 14
loses nothing that matters: the `error` Message row was a duplicate of an event the design stores
anyway.

#### Flow 4 — the migration

Batched and resumable, in this order, and the order is what makes it affordable:

1. `ALTER TABLE task_messages ADD COLUMN message_text text` — a nullable add, no rewrite.
2. Batched `UPDATE`: fold `message_content`'s `text` blocks into `message_text` for `message_type IN
   ('user', 'assistant')`. 281,670 rows, 96 MB.
3. Batched `DELETE` of every other row: `system`, `result`, `error`, the prose-less `user` and
   `assistant` rows, and the 153 rows holding `{}`. 2,345,460 rows.
4. `ALTER TABLE task_messages DROP COLUMN message_content`.
5. `DROP TABLE task_message_content_blocks`.
6. `VACUUM FULL task_messages`. Run last, when every live tuple is already narrow, so the rewrite is
   the small table and not the 4,143 MB one.

No activity backfill runs (Decision 9).

### Data model

#### Logical model

A **conversation** is one ordered stream belonging to a Task, made of two kinds of thing:

- A **Message** is something a person or the agent *said*. It has a role — `user` or `assistant` —
  and prose. Nothing else. It is original: no log stands behind it, and it cannot be rebuilt.
- An **activity** is something that *happened* on the way: a tool call and its outcome, a stretch of
  thinking, a Sandbox status change, a Sandbox error. It is identified by where it came from rather
  than by when it was written, which is what makes redelivery a no-op. It is created by one block and
  completed by another, so it is the one entity in the design that is deliberately mutable.

Both are positioned on the Task's conversation counter, which is what interleaves them.

Two invariants:

1. A Message and its activities are written in one transaction. A reader never sees one without the
   other.
2. A tool result exists in exactly one place — the activity's `payload`, whole and uncapped.

#### Physical representation

New table `task_activities`:

| Column | Type | Meaning |
|---|---|---|
| `activity_id` | `text` PRIMARY KEY | Derived from the source, never generated. `{message_uuid}:{block_index}` for a tool or thinking activity — the id `projection.rs:736`/`:795` already emits — or the event id for a Sandbox activity. Redelivery is `ON CONFLICT DO UPDATE` on this key. |
| `task_id` | `text` NOT NULL | The conversation. No foreign key: a record must outlive what it names. |
| `position` | `bigint` NOT NULL | From `task_conversation_position`. Indexed with `task_id`. |
| `message_uuid` | `text` NULL | Provenance, not a foreign key — the Message row may not exist (a prose-less turn creates none). Null for a Sandbox activity. |
| `kind` | `text` NOT NULL | Closed set: `tool_call`, `thinking`, `sandbox_status`, `sandbox_error`. A value outside the set is a bug, not an extension point. The mapping onto T3's kinds stays in `projection.rs`. |
| `status` | `text` NOT NULL | For `tool_call`: `in_progress` until the result lands, then `completed` or `failed`. This is what makes one row per call work. |
| `tool_use_id` | `text` NULL | UNIQUE with `task_id`. Load-bearing: a result arrives on a later Message than its call (`projection.rs:827`), so this is how it finds the row. |
| `summary` | `text` NOT NULL | The collapsed label — the tool's name, or the Sandbox status. What `projection.rs:798` already puts there. |
| `payload` | `jsonb` NOT NULL | Tool name, item type, status, command, and **the tool result whole**. What the Moatless frontend renders, one activity at a time. |
| `detail` | `text` NULL | The first 2,000 characters of that result's visible output, plus the cut marker. Written by `settle`, from `mapping::detail`. NULL until the call comes back, and on every activity that is not a tool call. Decision 16. |
| `is_error` | `boolean` NOT NULL | Same axis as `task_events.is_error`. Replaces `task_message_content_blocks.tool_result_is_error`. |
| `external_event_id` | `text` NULL | What this call created on the other side, when it created something — `github:ic:555` for a comment the agent posted. Decision 15. |
| `external_adapter` | `text` NULL | Which adapter the id belongs to, so the dedup gate reads it in the same vocabulary the inbound side writes. |
| `external_source_name` | `text` NULL | The human name of the source — `foo/bar` — for the chip the UI renders on the turn. |

Indexes: `(task_id, position)` for the conversation read; `(task_id, tool_use_id)` UNIQUE for the
result update; a partial index on `external_event_id WHERE NOT NULL` for the dedup gate's global
lookup, which asks which Task — if any — already claims an id.

**The 2,000-character cap is a column beside the whole result, not a rule applied on the way out.**
`payload` holds the result whole, for the Moatless frontend, which asks for one activity at a time.
`detail` holds the preview, cut once when the result arrived. The T3 surface re-sends its whole work
log every time the conversation moves, so cutting on the way out means cutting the same result again
on every send. See Decision 16.

Changes to `task_messages`:

| Column | Change |
|---|---|
| `message_text` `text` | **New.** The Message's prose. 96 MB across 281,670 rows. Still assembled by read-modify-write — an append to a string rather than a splice into a JSON document. |
| `message_content` `jsonb` | **Dropped.** Everything it held has a home: text in `message_text`, tool calls, results and thinking in activities. 3,080 MB reclaimed. |
| the rows themselves | **89.3% deleted.** A row exists when somebody said something: `user` and `assistant` with prose. |
| `turn_status`, `turn_result` | Unchanged and still unread — `ui_rpc/dao.rs:369` takes the turn's status from `task_turns`, never from the Message. |

Dropped whole: `task_message_content_blocks`. Both surviving readers move to typed columns —
`tool_result_is_error` to `task_activities.is_error`, and the `tool_use_id` `EXISTS` filter at
`message/dao.rs:1155` to the activity's own `tool_use_id`.

`task_events`: 163,141 `agent.message` rows deleted, none written again. Lifecycle-only.

`task_conversation_position.position`: read and unchanged, now allocated once per transaction for two
consumers rather than one.

### Interfaces and contracts

**External contract — the T3 shell surface.** Unchanged, and that is the requirement. The client
receives the same messages and activities, with the same `activity_id` shape, the same kinds
(`task.progress` for thinking; `tool.updated`/`tool.completed` for a call), the same tones, the same
2,000-character truncation with its marker, and the same 1,000-Message transcript cap
(`ui_rpc/dao.rs:282`). `rendered_transcript` is the specification to match, not merely a call site to
update. What changes is where its rows come from.

**External contract — the Moatless frontend.** The conversation response becomes two ordered
collections instead of one assembled blob per Message. A tool result is still rendered in full when
expanded. Two behaviours change and are visible:

1. A Task written before the migration shows no tool calls, thinking or Sandbox lifecycle, and its
   statistics say it predates the change.
2. A run's closing line — "Stopped by user request", "Sandbox terminated: …" — is no longer a
   message. A stopped turn is known by `task_turns.status`; a Sandbox failure by its activity.

**External contract — the Events tab.** Unchanged except that message skeletons leave it. It keeps
every event type, including the ones that never become activities: `taskTurn`, `agent`, `server`, and
`workspace_synced`.

**ADRs.** ADR-0003's persistence clause is withdrawn by Decision 5; its relay clause survives as the
whole ADR. ADR-0002 (per-block source events) and ADR-0006 (transient ingress outbox) are unchanged
and both are load-bearing here.

**Internal interfaces** — a plan, not a promise; changing one later needs no spec update:

- The block-to-row mapping's signature, and whether it returns an enum or writes directly.
- The shape of `payload` beyond "holds the result whole".
- Whether the conversation read is one query with a union or two queries merged in Rust.

Call sites with a wide blast radius, listed as information: `message/dao.rs:1080`, `:1186`, `:1258`
(assembly reads selecting blocks), `message/dao.rs:1155` (the sibling fold), `task/statistics.rs:84`,
`ui_rpc/projection.rs:560`, `message/transform.rs:184` and `:196`.

### Failure, safety and recovery

| Failure | State before | State after | Caller sees | Retry safe? | Handling |
|---|---|---|---|---|---|
| Block redelivered | Activity exists | Unchanged | Nothing | Yes | `ON CONFLICT DO UPDATE` on `activity_id` |
| Result arrives for an unknown call | No matching row | No change | Nothing | Yes | Discarded silently — a result whose call predates the migration is the common case |
| Result arrives twice | Row `completed` | Unchanged | Nothing | Yes | Same update, same outcome |
| Blocks arrive out of order | Partial | Correct | Correct conversation | Yes | The activity is keyed on the source, not on arrival order |
| Transaction fails mid-turn | Prior state | Prior state | The turn stalls; ingress redelivers | Yes | Message and activities share one transaction |
| Sandbox repeats a status | Activity exists | Unchanged | Nothing | Yes | `IS DISTINCT FROM` collapse guard |
| Migration interrupted | Partly folded | Partly folded | Nothing | Yes | Batched and resumable; each batch is its own transaction |
| Migration run twice | Complete | Complete | Nothing | Yes | The fold is idempotent; the delete is by predicate |
| Reclaim holds its lock | Normal | Normal | Conversations stall | n/a | Maintenance window, not a deploy step |

**The write that can still corrupt.** Text is the one block type still assembled by read-modify-write,
now onto `message_text`. With no log behind it, a bad write is not recoverable by replay. This is
accepted (Decision 12) and it is 96 MB of the 3,080 MB that stops being spliced. It is also a narrower
failure: appending to a string cannot corrupt a document's structure.

**Rollback.** Up to the moment `task_message_content_blocks` is dropped and `message_content` is
deleted, rolling back means writing tool blocks back from `task_activities` — a real migration, not a
revert. After those two DDL steps there is no rollback: the source is gone. The runbook must state
this and the two steps must be separated from everything before them by a deliberate go/no-go.

**Degraded mode.** While the reclaim holds its lock, everything that does not read conversation
content keeps working: the shell listing, the event log, message sending, every Sandbox write path.
What stalls is conversation assembly.

**Security.** No change to what is stored or who can read it. Tool results were already stored in full
in `message_content`; they move rather than appear.

### Testing and verification

**Commands.** `cargo clippy -p moatless-vibe-backend -- -D warnings` and `cargo test -p
moatless-vibe-backend` for the unit tier. Everything under `backend/tests/` except `supervisor_test.rs`
is behind `#![cfg(feature = "db-tests")]`, so **`task test:integration` is the only tier that builds
it** — the plain `cargo test` line and `ci.yml` compile almost none of those files. Run
`task test:integration` after touching anything in `backend/tests/`. Frontend: `bun run type:check`
and `bun run biome:check`.

**Seams.** The block-to-row mapping is the module this change is tested at, and it is the highest
available seam: it is a pure function from a block plus its context to a row, it is shared by the live
writer and the migration, and testing it once covers both. `rendered_transcript` is the second seam —
it already has a test suite asserting the exact contract, and those tests must pass unchanged with the
rows coming from tables instead of from jsonb. That is the strongest single verification this change
has, and `backend/tests/ui_rpc_thread_test.rs` is where it lives.

**Unit**

- The mapping, one case per block type, including the four in `UNRENDERED_BLOCK_TYPES` and one type
  it has never seen.
- `activity_id` derivation for a tool activity, a thinking activity and a Sandbox activity.
- The truncation helper: at the cap, one over, and on a multi-byte boundary.
- The Sandbox collapse guard: same status, different status, first status.

**Integration** (`task test:integration`)

- A tool call and its result on different Messages produce one activity moving `in_progress` →
  `completed`.
- The same, with `is_error` set, produces `failed`.
- A result with no call creates nothing and returns `Ok`.
- Redelivery of every block type leaves the row count unchanged.
- A block set with no `text` block creates no Message row.
- The migration over a seeded conversation is idempotent and resumable.
- `rendered_transcript`'s existing assertions pass against stored rows.

**Acceptance criteria**

- Given a Task the migration has converted, when its conversation is rendered, then it matches what
  the same Task rendered before the migration.
- Given a Message whose prose was folded, when it is rendered, then it is character-identical to what
  that Message rendered from its text blocks.
- Given the migration is run twice, when it finishes, then no activity is duplicated.
- Given blocks arriving out of order or twice, when the conversation is read, then it matches what
  assembly produces today.
- Given a tool call and its result, when both have arrived, then there is one activity whose status
  moved from in progress to completed or failed.
- Given a thinking block, when the conversation is read, then it is an activity and never a message
  bubble.
- Given a tool result of any size, when its activity is written, then `payload` holds all of it; and
  when a snapshot is built, then the detail it carries is at most 2,000 characters.
- Given a tool call expanded in the UI, when its result is fetched, then it is byte-identical to what
  `message_content` held before the migration.
- Given a Task with activities, when the T3 shell surface reads it, then it returns the same messages
  and activities it returns today.
- Given a stored thinking activity, when it reaches the T3 surface, then it is sent as
  `task.progress`.
- Given a Sandbox activity, when it reaches the T3 surface, then its kind ends in neither `.started`
  nor `.completed`.
- Given a Sandbox reporting the same status twice, when the second arrives, then no second activity
  is written and the event row is still recorded.
- Given a Sandbox event outside Decision 10's set, when it is admitted, then it appears in the Events
  tab and in no conversation.
- Given a stored Message that is a tool-result carrier, a prose-less tool call, a stop marker or an
  error row, when the migration finishes, then it does not exist and no conversation renders a gap
  where it was.
- Given a turn stopped by the user, when the conversation is read, then no Message says so and the
  turn's status is `stopped`.
- Given a Sandbox that fails fatally, when the conversation is read, then the exit code, terminated
  reason and disruption cause appear on one Sandbox activity and in no Message.
- Given a Task with no activities, when its statistics are read, then they say it predates the change
  rather than reporting zero tool calls.

**What only a person can judge**

- That a conversation rendered from activities is the same conversation it was before.
- Whether 2,000 characters is the right snapshot cap, now that it is the only thing between a re-sent
  snapshot and an uncapped stored payload.
- Whether Sandbox statuses in the conversation read as useful context or as noise once collapsed.
- Whether an old conversation saying it predates the change is a satisfying answer or reads as a
  broken page.
- Whether a turn that ended in a non-Sandbox error is legible from its status alone, or whether
  Decision 10's exclusion of the `agent` error subtypes has to be reopened.
- Whether losing message skeletons from the Events tab timeline is acceptable in use.
- Whether the reclaim's stall is tolerable at the hour it is run, and whether the rollback procedure
  is one an operator can follow under pressure.

## Decisions

| # | Decision | Settled as |
|---|---|---|
| 1 | Where does the arc start? | Tool-result de-duplication only |
| 2 | How is the block table's space reclaimed? | Batched backfill to NULL, then `VACUUM FULL`, in that order |
| 3 | How much of the arc does the spec cover? | All of it, every step buildable |
| 4 | Is the Message a projection? | **Amended.** No — it stays original |
| 4b | How does a rebuild run? | **Withdrawn** by Decision 12 |
| 5 | Do message skeletons leave `task_events`? | Yes. ADR-0003's persistence clause withdrawn |
| 6 | How does the cut-over work? | **Withdrawn** by Decision 12 |
| 6b | What about a conversation that straddles the cut-over? | **Withdrawn** with Decision 6 |
| 7 | What happens to PR #176? | Merge it — the counter allocator is the part needed |
| 8 | Does the Message go slim, with `task_activities` beside it? | Yes, and it is the focus |
| 9 | Do existing Tasks get activities? | No |
| 10 | Which event types become activities? | Tool calls, tool results, Sandbox status changes and errors |
| 11 | One activity per call or two, and does thinking stay? | One, carrying its outcome. Thinking stays |
| 12 | Does the block log survive Decision 8? | No. Dropped before it is built |
| 13 | Does `message_content` become a `text` column, and is it folded? | Yes to both — user and assistant only |
| 14 | Where does a run outcome go? | Nowhere. It is not a Message and needs no new kind |
| 15 | Where does an outbound external claim live? | On the call that made it, not on a Message |
| 16 | Is the 2,000-character cap a column? | Yes — cut once at write time, in `detail` |
| 17 | What is the module called, and what is in it? | `task_activities`, with all of its SQL in `dao.rs` |

### 1. Where does the arc start?

**Tool-result de-duplication only.** Stop writing `task_message_content_blocks.tool_result_content`,
backfill the existing values to NULL, move nothing else.

The column is 1,104 MB of a 1,861 MB table. All three assembly reads select it — `message/dao.rs:1080`,
`:1186`, `:1258` — and every consumer then filters on `show_in_ui`, which is `false` on every one of
those rows (`agent_handlers.rs:1236`). So the full 1,104 MB is fetched off disk, sent over the wire and
discarded on every conversation load. The same bytes are already in `message_content`, which is what the
UI actually renders a tool result from.

The rows themselves cannot simply be deleted: `message/dao.rs:1155` finds a folded tool-result sibling
by the `tool_use_id` on its `tool_result` block row, so deleting them breaks conversation assembly.
NULLing the content column is the change that is safe on its own.

Rejected: doing the whole arc in one change, which has no reviewable first step; and starting with the
activities table, which leaves the duplication in place for however long that takes.

### 2. How is the block table's space reclaimed?

**Batched backfill to NULL, then `VACUUM FULL`, in that order.**

The table is heap 624 MB + TOAST 1,038 MB + indexes 199 MB. `tool_result_content` is `extended`
storage, so nearly all of that TOAST relation is this one column; NULLing it is what empties the
relation. `VACUUM FULL` rewrites live tuples only, so run *after* the backfill it rewrites 624 MB of
heap and 199 MB of indexes with an essentially empty TOAST. Run before, or interleaved, it rewrites
the full 1.8 GB for nothing.

The backfill rewrites 458,562 heap tuples, so the heap grows before vacuum catches up — batching is
what bounds that. Autovacuum is healthy here (13 runs, 8,334 dead tuples against 2.61 M live).

Rejected: leaving it to autovacuum, which strands the space in a TOAST relation whose only remaining
writer is 147 MB of slowly-growing text; `pg_repack`, which buys a lock-free rewrite at the price of
owning a production extension install for a one-off (1.5.1 is available and not installed); and never
backfilling.

**Cost accepted:** ACCESS EXCLUSIVE on `task_message_content_blocks` for the length of the rewrite.
Conversation assembly reads that table, so every conversation in the deployment stalls for the
duration. This is a maintenance window, not a deploy step.

### 3. How much of the arc does the spec cover?

**All of it, to a depth an implementer can work from.** The de-duplication, the activities table, the
slimming of the Message, and the skeleton removal.

Rejected: step one alone, which defers the design questions rather than answering them; and the arc
with its tail written as direction, which produces a document nobody can build the second half of.

The arc got *shorter* over the interview, not longer. As originally decided it ran to a block log and
a rebuildable projection. Decision 12 removed both, so it now ends at two typed tables and two dropped
tables. The commitment this decision makes — that every step is buildable from the document — is
unchanged.

### 4. Is the Message a projection? (amended)

**No. The Message stays original.** It was first settled as a projection rebuildable from the block
log. Decision 12 removed the log, so there is nothing to project from and nothing behind the Message.

What survives from the original reasoning is the part that was never about the log: a Message's
identity is its uuid, and everything outside the conversation continues to name it that way.

### 4b. How does a rebuild run? (withdrawn)

**Withdrawn by Decision 12.** It had been settled as a function plus a runbook, with no automation
behind it. With no log there is no rebuild, and the fact that the agreed answer was already "a manual
procedure nobody has automated" is part of what made Decision 12 easy.

### 5. Do message skeletons leave `task_events`?

**Yes. No message data stays in `task_events` at all.** 163,141 `agent.message` rows are deleted and
none is written again; the table becomes lifecycle-only.

ADR-0003 has two clauses. Its persistence clause — that an `agent.message` skeleton is stored — is
withdrawn. Its relay clause — that the skeleton is the shape relayed to adapters — survives, and
becomes the whole ADR. ADR-0003 must be edited to say so.

**Cost accepted:** the Events tab timeline no longer shows messages among the lifecycle events. That
timeline is the one reader of those rows.

Rejected: keeping the skeleton purely for the Events tab, which keeps a second write on the hot path
and a second place message data lives, for a timeline that the conversation itself now orders
correctly.

### 6. How does the cut-over work? (withdrawn)

**Withdrawn by Decision 12.** There is no log, so there is no cut-over. What existing Tasks get is
Decision 9.

### 6b. What about a conversation that straddles the cut-over? (withdrawn)

**Withdrawn with Decision 6.** There is no cut-over to straddle.

### 7. What happens to PR #176?

**Merge it.** Its counter allocator — `INSERT … ON CONFLICT DO UPDATE … RETURNING`, which holds its
row until commit — is exactly what gives an activity its position inside the same transaction as the
Message it belongs to. The log table it was going to feed is gone; the allocator is the part that was
needed.

`backend/tests/ui_rpc_thread_test.rs` needs a `task test:integration` pass before it merges — the
plain `cargo test` line does not build it.

Rejected: growing its table into the block log, pruned or unpruned, which Decision 12 removed the
purpose of; and closing the PR, which would mean rewriting the allocator here.

### 8. Does the Message go slim, with a `task_activities` table beside it?

**Yes, and it is the focus of the spec.** A Message becomes role and prose. Tool calls, tool results,
thinking and Sandbox lifecycle become activities.

This is not a new model. `ui_rpc/projection.rs:514` already states it as the difference between how a
reader sees a conversation and how Moatless stores one, and `rendered_transcript` (`:560`) already
computes the split on every read for the paired T3 Code client. Decision 8 stores what Moatless
already computes. That reframes the whole change: `rendered_transcript` is the specification to match,
not a call site to update.

Rejected: keeping everything in one table with a discriminator column, which leaves every read
filtering; and computing the split in the frontend, which leaves the second consumer computing it
separately and differently.

### 9. Do existing Tasks get activities?

**No.** A conversation written before the migration shows its messages and nothing the agent did.

This was asked twice. The first time, a backfill also meant reconciling a log that deliberately
started mid-history, so a backfilled row and a live row would have had different provenance with no
single sentence describing either. Decision 12 removed that entanglement and Decision 11 halved the
row count, so it was re-asked with the cheaper option genuinely on the table — and answered the same
way.

**Cost accepted:** 4,962 existing conversations show their messages and none of their tool calls,
thinking or Sandbox lifecycle. 1,859 of those Tasks were written to in the last thirty days and 688 in
the last seven. The content is not deleted — nothing reads it, and after Decision 13 it is dropped
with the column.

**And a second consequence, accepted as part of this decision rather than reopened:** task statistics
for those Tasks go to zero, because tool counts and failure counts come from `tool_result_is_error`
and `tool_use_id` on the block table (`task/statistics.rs:84`) and its replacement is a row those
Tasks do not have. Statistics must therefore say the Task predates the change rather than report zero
tool calls as a fact.

**What it buys, and the reason it survives re-asking:** the block table can still be dropped. Both of
its surviving readers exist to render tool calls — the statistics counters, and the sibling fold at
`message/dao.rs:1155`. Once an old conversation does not render tool calls, neither reader has
anything to ask of an old Task. "No backfill" plus "keep rendering old tool calls" would have required
a permanent branch onto `message_content`, which is the read path this whole change exists to remove.

Rejected: backfilling every Task, ~532,000 rows written from `message_content`; and backfilling lazily
per Task on first read, which has the same source and the same code but spreads the cost and leaves
two provenances live at once.

### 10. Which event types become activities?

**Tool calls, tool results, and Sandbox status changes and Sandbox errors. Nothing else.**

A closed list, not a filter over the event log. Excluded on purpose: `taskTurn`, every `agent`
subtype, `server`, and `workspace_synced` — which is a Sandbox event but not a status change. The
Events tab keeps all of them and keeps its own job.

Sandbox activities carry a collapse rule: a status identical to the previous one writes nothing.
92,165 of 139,595 Sandbox rows today restate the status before them; deduped and excluding
`workspace_synced`, 139,595 becomes 46,741. The rule is not new to this codebase — the lifecycle
intent path already has the equivalent `IS DISTINCT FROM` guard around `record_session_change`
(`sandbox/lifecycle/dao.rs:290`). The event path simply never had it.

Rejected: including the `agent` error subtypes, which would put backend-internal failures in the
conversation alongside tool calls; and deriving activities from the event log by filter, which makes
the set open-ended and couples the conversation to every future event type.

### 11. One activity per call, or two — and does thinking stay?

**One, carrying its own outcome. Thinking stays as a fourth kind.**

t3's runtime emits three separate rows per tool call — `tool.started`, `tool.updated`,
`tool.completed` — each keyed on its own event id (`ProviderRuntimeIngestion.ts:670`, `:625`, `:648`)
and each upserted under that id (`ProjectionPipeline.ts:1012`). But its client drops the first and
folds the second into the third (`apps/web/src/session-logic.ts:633`, `:781`), rendering one entry per
call. Moatless's own T3 surface already stores the folded form conceptually:
`"kind": if outcome.is_some() { "tool.completed" } else { "tool.updated" }` (`projection.rs:797`).
Decision 11 stores the entry that both clients actually render.

Moatless does have a call-start event — ADR-0002 has the Sandbox publish every block as its own
`agent.message`, so the `tool_use` block *is* a start. What it lacks is t3's *contentless* start: the
Moatless event carries the full input, so it maps to `tool.updated`, not `tool.started`.

Thinking stays because dropping it would be a behaviour change the T3 surface would notice
(`projection.rs:729` sends it as `task.progress`), and it costs almost nothing: only the label is
stored, since nothing renders the thought.

Volume at today's history size: 459,933 tool calls + 25,692 thinking + 46,741 collapsed Sandbox rows ≈
**532,000 activities**, against ~967,000 under two-rows-per-call.

Rejected: two rows with the T3 projection re-folding on read, which stores the shape neither client
wants; and two rows with the T3 contract changed to match, which breaks a shipped client for a storage
convenience.

### 12. Does the block log survive Decision 8?

**No. It is dropped before it is built, and `task_message_content_blocks` goes with it.**

The log was designed when a Message was the only home for every block. After Decisions 8 and 11 every
block type has a typed home of its own — text in the Message, the call and its result in one activity,
thinking in an activity. The log stopped being the record and became a second copy of things that each
already have a place.

**Two binding conditions.** First, the activity holds the tool result *in full*: 30.1% of results
exceed 2,000 characters and the Moatless frontend renders all of it
(`generic-tool-call.tsx:124`, no truncation). Second, thinking keeps only its label, which costs
nothing because no surface renders the thought.

**Consequences.** `task_message_content_blocks` can be dropped whole rather than emptied — that was
Decision 1's rejected option, rejected on sequencing and never on merit, and it becomes the end of the
arc rather than the start of it. Decisions 4, 4b and 6b are reversed or withdrawn. Decision 7 keeps
its allocator and loses its log table. And the Problem statement comes back in a smaller form: a write
that lands wrong is wrong for good, because per-block typed rows are an insert each and the
read-modify-write splice survives only for text.

Storage falls rather than rises: ~2,768 MB of typed rows against 3,072 MB of blocks today, with no
second copy anywhere.

Rejected: keeping the log as specified, which is a full second copy of roughly 3,000 MB growing at the
rate the conversation grows, bought for a rebuild path Decision 4b had already agreed would be a
function and a runbook with no automation; and keeping the log for text only, which covers a fifth of
the conversation and makes "rebuild the conversation" stop being a true sentence.

**Replay is what is being given up**, deliberately. A writer bug cannot be repaired by replaying — only
by a migration written against the typed tables, which is exactly the situation today. The ingress
outbox is not a substitute: it is transient by design (ADR-0006), and Decision 5 removes message data
from `task_events`, so after this work no durable record of an arriving block exists outside these two
tables.

### 13. Does `message_content` become a `text` column, and is it folded?

**Yes to both, and only `user` and `assistant` Messages survive the fold.**

The column was never in question. After Decision 8 a Message is role and prose, and after Decision 12
nothing left in `message_content` lacks a typed home. A jsonb array of one element per block describes
a shape that no longer exists. It becomes `task_messages.message_text`, `text` — which is exactly what
the other consumer of this conversation already has: `projection_thread_messages.text` is one flat
string holding no blocks, no tool calls and no tool results.

The fold is `string_agg` over the `text` blocks in `sort_order` — what `rendered_transcript` already
computes on every read. Unlike an activity backfill it invents nothing: no kind mapping, no status
inference, no ids. That is why "no backfill" from Decision 9 is not simply repeated here; and it could
not be, because an unfolded row renders **blank**, where a Task with no activities still renders.

**The narrowing to user and assistant is not a new rule.** It is the rule both readers already apply:
`projection.rs:604` drops a `system` row outright, `projection.rs:610` drops a `user` row carrying tool
results, every branch drops an empty message, and `message/transform.rs:184` and `:196` make the same
two exclusions for the frontend. Decision 13 stops storing what both of them throw away.

**Result:** 2,627,130 Messages become **281,670** — 10.7%, holding 96 MB of prose where 3,080 MB of
jsonb stands today. The 153 rows holding the literal text `{}` are deleted rather than folded. The
narrowing also removes the "39% of Messages carry no prose" problem rather than living with it: those
rows are not kept with an empty string, they do not survive. A Message row exists when somebody said
something.

Rejected: writing `message_text` forward-only with a reader fallback that folds `message_content` when
it is null — no migration and no window, at the price of `message_content` being permanent, 3,080 MB
never reclaimed, and the jsonb walk this change exists to delete surviving as a reader branch nobody
can remove. And rejected: dropping `message_content` with no fold, for symmetry with Decision 9, which
empties 4,962 conversations.

### 14. Where does a run outcome go?

**Nowhere. It is not a Message and it needs no new activity kind.** `result` and `error` rows stop
being written, and the 1,299,303 stored ones are deleted.

**This is the policy this codebase already states.** `stop_is_reported` (`message/models.rs:35`)
already restricts the `Result` row to error stops, and gives the reason in full: the other stop
reasons are routine lifecycle, *"the reader already knows: the task's live status and turn state say
so, and for externally-bound tasks a 'Stopped by user request' line is pure noise on Slack or
Linear."* Decision 14 finishes that thought by deleting the rows written before it and removing
`stop_marker_text` with its call site.

**The one line worth keeping is already an event.** `k8s/pod_watcher.rs:803` publishes "Sandbox
terminated: …" — with exit code, terminated reason, node, disruption cause and container logs — as a
`SandboxStatus::Error` event carrying that identical `detailed_message` in both the `message` and
`error` fields of its payload. Decision 10 already admits Sandbox errors as activities. The Message
row is a duplicate of an event the design stores anyway; dropping it removes a copy, not a fact.

**And the turn's outcome does not live on the deleted row.** It is `task_turns.status`, its own table
since `backend/migrations/0071_task_turns.sql:13`, and it is already what the read path selects
(`ui_rpc/dao.rs:369` takes `turn.status`, never the Message's copy).

**Both volumes are incidents, not workload**, which is worth stating so the saving is not oversold:
1,163,898 of the 1,165,256 `result` rows were written in April 2026 across 395 Tasks — 2,946 per Task
— and 137,193 of the 141,497 `error` rows in February 2026 across three Tasks. Outside those two
months the pair runs a few hundred rows a month. The reason to drop them is that they are not
conversation, not that they are large.

**Cost accepted:** a stop that is an error but *not* a Sandbox failure loses its line — "Recovery
abandoned after 3 attempts", "Stopped after an error", "The agent stopped without reporting a result".
A few thousand rows, written by the backend rather than the pod watcher. Decision 10 deliberately
excluded the `agent` error subtypes from activities, so nothing picks these up. What remains is the
turn's status and the Events tab, which keeps every one of them: a reader sees that the turn ended in
error and opens the Events tab to see why. Reopening Decision 10's exclusion is the fix if that turns
out to matter in use.

Rejected: a fifth activity kind, `run_outcome` — it fits Decision 8's rule and would get the collapse
rule for free, but it reopens Decision 10's closed kind set to build a writer for content that is
already an admitted event. And rejected: keeping them as Messages with a third role, which is cheapest
to build and makes "only user and assistant" false the day it is written, leaving `task_messages` at
roughly 1.59 M rows of which most is two repeated sentences.

### 15. Where does an outbound external claim live?

**On the activity for the call that made it.** `task_activities` gains `external_event_id`,
`external_adapter` and `external_source_name`, and `github_pr/shell_reconcile.rs` records the claim
against `(task_id, tool_use_id)` instead of against a Message uuid.

**The old carrier stopped existing.** When the agent runs `gh pr comment`, the reconciler claims the
id the command printed so the webhook echo of that same comment is recognised as the agent's own and
not delivered back as a new Message. It claimed it by writing three columns on the calling assistant
`task_messages` row (`set_external_source_metadata`, an `UPDATE … WHERE uuid = $1`). Under Decision 13
a turn that only ran the command said nothing and writes no Message row, so that `UPDATE` matched
nothing and the claim was silently lost — the echo would then arrive as a new inbound Message and the
agent would answer its own comment. `gh_pr_comment_tags_tool_result_with_external_id` caught this
against a real database; it is the one failure in this work that review had not found.

**The call is the right carrier on its own terms.** The call is what made the comment, it always has
a row, and its row is keyed by the `tool_use_id` the reconciler already has in hand. Nothing had to be
threaded through to reach it.

**The reads follow the write.** `has_any_external_event_id`, `get_external_event_ids` and
`find_task_id_by_any_external_event_id` now consult `task_activities` as well as `task_messages` —
both, not either, because inbound claims still land on the Message the delivery created and those rows
are not going anywhere. `transform.rs::claimed_external_source` gives the UI the same chip it had, read
off the turn's activities, for both the prose and the tool-only turn.

Rejected: writing a Message row solely to hold the claim, which reintroduces the empty rows Decision 13
deletes and puts a row in the conversation that nobody said. And rejected: keeping the claim on the
Message and having the reconciler create one when none exists, which is the same thing with a
conditional in front of it.

### 16. Is the 2,000-character cap a column?

**Yes. `task_activities.detail`, cut once by `settle` when the result arrives.** The whole result stays
in `payload`.

**The cost of the cap being a response rule is that it is paid per send, not per result.** The T3
surface re-sends its whole work log every time the conversation moves. A `Read` of a large file is
therefore fetched, parsed and truncated again on every message of that conversation, to produce the
same 2 kB each time. Cutting at write time produces it once.

**`settle` is the only correct place for it.** It is where the result arrives, and it is the one
statement that has the result in hand. `upsert` deliberately leaves `detail` alone for the same reason
it leaves `status` and the result alone: a redelivered `tool_use` block must not blank the outcome of a
call that has already come back.

**The mapping is where the cut itself lives** — `mapping::detail`, beside `result_text`, so the pure
function that decides what a block *is* also decides what a preview of it looks like, and both are
unit-tested at the same seam.

Cost accepted: the T3 read still selects `payload`, so it still fetches the whole result off disk. What
`detail` buys today is the repeated truncation, not the repeated detoast. Removing the detoast means
giving that surface its own read that selects `payload` minus its bulky keys, which is a separate change
and is not made here.

Rejected: capping `payload` itself, which loses the Moatless frontend's full render. And rejected:
capping in `summary`, which is the collapsed label and is compared for equality by the Sandbox collapse
rule.

### 17. What is the module called, and what is in it?

**`task_activities`, and all of its SQL is in `dao.rs`.** `activities.rs` and the old `dao.rs` are one
file. `mapping.rs`, `models.rs` and `pruner.rs` are unchanged. Importers say
`use crate::task_activities::dao::{self as activities, Activity}`, so call sites still read
`activities::settle`; the two files that import it are themselves `dao.rs`, and a bare `dao::` inside
one would read as self-reference.

**The module was named for the record it started as and outgrew it.** `task_conversation` was written
to hold one thing — the durable record that a conversation changed. It now holds the rows a
conversation is made of, and those are the bulk of it.

**The `task_conversation_*` tables keep their names, and are still needed.** Both are load-bearing, and
neither is expressible as a query over `task_activities`:

- `task_conversation_position` is the counter every write allocates from, in the mutation's own
  transaction. It orders activities *and* Messages against each other, and `current_head` reads it as
  the authority for where a conversation has reached. A `MAX(position)` over any table is wrong here:
  the change log is pruned to a window, so a `MAX` over it drops back to a position long since passed,
  and a `bigserial` is wrong for the reason the sibling module already documents — an id assigned at
  INSERT and made visible at COMMIT can be read out of order.
- `task_conversation_changes` is what a client polls to learn the conversation moved. It records a
  Message-affecting write, which includes writes with no activity at all: a person's message, and a
  session change that touches only the Turn or the Sandbox's status.

They are named for the conversation because that is what they order — not only its activities. The
module owning them is named for the rows that dominate it.

## Out of scope

- **Replay and event sourcing.** Decision 12 gave both up deliberately. There is no log and nothing to
  rebuild from.
- **Turn, Agent and Server events.** They stay in the Events tab and appear in no conversation.
- **The `agent` error subtypes as activities.** Excluded by Decision 10 and re-affirmed as the accepted
  cost of Decision 14.
- **Backfilling activities for existing Tasks.** Decision 9.
- **Streaming partial assistant messages.** `StreamEvent(PartialAssistantMessage)` stays parsed and
  unforwarded.
- **The `tone` column t3 carries.** `kind` plus `is_error` says the same thing in this codebase's
  vocabulary; the mapping to t3's tones stays in `projection.rs`.
- **Changing the T3 wire contract.** Decision 11 explicitly rejected doing so.
- **Retention for `task_conversation_changes`.** Noted below, not solved here.

## Open questions

- **~~Nothing here has been run against PostgreSQL.~~ Answered: it has been, and it found one real
  bug.** `task test:integration` was executed against a PostgreSQL 16 built from `.deb` archives inside
  the sandbox. Every conversation suite is green — `task_turn_dao_test` 72/72, `ui_rpc_command_test`
  17/17, `ui_rpc_shell_test` 12/12, `event_ingress_test` 9/9, `message_handler_test` 9/9,
  `closed_terminal_test` 8/8, `agent_handlers_per_block_test` 7/7, `task_listing_position_test` 12/12,
  `ui_rpc_thread_test` 2/2, `agent_handlers_hooks_test` 1/1. The run found three failures caused by
  this work: two test helpers still reading `task_message_content_blocks`, and the lost outbound claim
  that became Decision 15. All three are fixed. The 23 that remain are pre-existing and unrelated —
  missing `api_keys.scopes`, an unreachable GitHub App stub, `integration_connections_owner_scope_chk`,
  empty Linear issue bindings, the delivery worker's own suites, a `task_events.event_type` rename from
  migration `0015`, a date-dependent assertion, and one in `task/dao.rs`, which this work does not
  touch.
- **The claim on a user Message is still a mutable column.** `agent_session_id IS NULL` is what
  "pending" means, and it is cleared by conditional `UPDATE`s. Whether that moves in this work or stays
  a column is not decided.
- **Answering a question records no listing change.** A person's answer settles the `AskUserQuestion`
  activity through `append_activities`, which records a change only when the write *creates* an
  interactive call — so the sidebar's waiting flag flips to false without the row being re-sent. The
  same gap existed before this change (the carrier row was a `system` Message, which moved no listing
  row either), so it is not a regression; it is simply still there.
- **~~`get_subagent_aggregates` reads rows Decision 13 deletes.~~ Answered: a third table.** Usage is
  admitted against the Message's uuid in `task_message_usage` (migration `0118`,
  `activities::admit_usage`), and `get_subagent_aggregates` sums that instead of `task_messages`. A turn
  that only called tools reports almost all of a subagent's tokens and has no Message row to hang them
  on, so neither the row nor the activity was the right carrier. Step 2 of the runbook backfills it
  before step 3 deletes anything.
- **~~An uncapped `payload` makes the snapshot read detoast what it discards.~~ Half answered by
  Decision 16.** The repeated *truncation* is gone: the preview is a column, cut once. The repeated
  *detoast* is not, because both surfaces still read through one query that selects `payload` whole.
  Removing it means giving the T3 path its own read — `payload` minus `input`, `result` and
  `typedResult`, plus `detail` — which saves the socket and the parse but not the disk fetch, since
  PostgreSQL detoasts to evaluate the subtraction. Worth doing if it shows in latency; still not worth
  pre-empting.
- **`task_conversation_changes` grows without bound in practice.** Retention is by position from each
  Task's head and only 6 of 4,965 Tasks exceed the retained window, so it bounds the pathological tail
  and never fires for the rest — roughly 122,000 rows a week.
- **A greenfield second opinion was asked for and never run.** The request to have a separate agent
  assess this as if starting fresh is still outstanding.

## Ticket breakdown

None.
