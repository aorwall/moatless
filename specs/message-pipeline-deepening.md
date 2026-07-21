# Spec — Message pipeline deepening

> Refines ADR-0002 (server-owned message assembly); recorded as ADR-0003
> (`docs/adr/0003-agent-message-skeleton-and-affected-parent-assembly.md`). This
> revision supersedes ADR-0003's *subagent-relations* decision (server-side
> nesting delivered inside the parent Message upsert) with a **lazy subagent**
> model; the ADR is updated to match when this lands.

## Problem Statement

Three problems sit in the same pipeline — the path from an agent's output to a
rendered Message in the Chat.

**As a Chat user**, when the agent runs a tool mid-Turn, the tool's result does
not appear under its tool call as it returns. The call renders "running" and the
result only fills in when the whole Turn finishes. On a Turn with several tool
calls I watch a column of resultless calls, then they all populate at once — so I
can't follow what the agent is doing while it works.

**As a maintainer/operator**, a Message's content is stored and shipped twice.
The raw `agent.message` **source event** carries the full content into the event
log *and* the same content is assembled into the Message store. The event log
never prunes, so the largest payloads accumulate forever as a second copy of what
the Message store already holds; and every connected client receives that content
twice on the live wire — once as the raw source event, once as the assembled
**Message upsert** (a derived event).

**As a Chat user with subagents**, when the agent spawns parallel subagents, the
Chat needs their transcripts attached to the right `Agent` tool call. A naive
server-side fix — assembling the whole subagent subtree into the parent Message
and re-emitting it on every subagent step — makes each subagent event re-ship the
full, growing transcript on the live wire (`O(N²)` bytes over a run) and bloats
the top-level Messages read with content most of which is never expanded.

The root cause of the first two is shared: after ADR-0002 moved Message assembly
server-side, the assembler resolves the changed row by its own identity. A row
that folds into another Message (a `tool_result` inlined into the assistant
Message that called it) has no standalone assembled Message, so its upsert is
skipped — the parent never refreshes live. And the raw source-event content, now
needed only transiently to build the Message store, is still persisted and
relayed in full. The third is a design choice about **where** subagent content
lives on the wire and the read path.

## Solution

**As a Chat user**, a top-level tool's result renders live under its tool call as
it returns, because a change to a folded row (a `tool_result`) refreshes the
**parent** assistant Message that owns it — not just standalone Messages.

**As a maintainer/operator**, a Message's content is written once, to the Message
store, and reaches the Chat through one assembled stream. The raw `agent.message`
source event becomes a lightweight **skeleton** on the persisted log and the
client wire; its full content is a transient ingestion detail the backend consumes
once to build the Message store. Clients receive content once (the Message
upsert), the event log stops storing a second copy, and existing fat rows are
backfilled to skeletons.

**As a Chat user with subagents**, subagent transcripts are **lazy**: the
top-level Messages read carries only a small per-`Agent`-call **summary**
(message count, token count, status), and the collapsed `Agent` card renders from
that summary alone. The full subagent transcript is fetched **on demand** when I
expand the card, from a dedicated endpoint. Live, a subagent's activity emits a
tiny **progress** event (the same summary shape) that keeps the collapsed card's
count and tokens current without ever streaming subagent content; when a card is
expanded, its transcript refreshes from the endpoint as progress ticks arrive. No
subagent content rides the firehose, and no growing subtree is re-shipped.

The first two are one change: the read path and the live stream cross a single
deepened Message-assembly seam that owns affected-parent resolution (for
top-level tool results), and the source event is slimmed everywhere it is
persisted or relayed. The third is single-sourced against the same summary shape,
shared by the read path and the live progress event.

## User Stories

1. As a Chat user, I want a top-level tool's result to appear under its tool call as soon as it returns, so that I can follow the agent's progress during a Turn.
2. As a Chat user, I want multi-tool Turns to fill in results incrementally, so that I am not left staring at a column of resultless "running" calls until the Turn ends.
3. As a Chat user, I want the streamed Message to match what I see after a reload, so that live and read views never disagree.
4. As a Chat user, I want my own message to still appear instantly and reconcile without duplicating, so that the optimistic behavior established in ADR-0002 is preserved.
5. As a Chat user, I want a collapsed `Agent` tool call to show how many messages the subagent has produced and how many tokens it has spent, so that I can gauge subagent activity without expanding it.
6. As a Chat user, I want the subagent message count and token count on a collapsed `Agent` card to update live as the subagent works, so that a running subagent visibly makes progress.
7. As a Chat user, I want to expand an `Agent` tool call to read its full subagent transcript, fetched on demand, so that the detail is there when I want it but not shipped when I don't.
8. As a Chat user, I want an expanded subagent transcript to keep updating as the subagent works, so that I can watch a subagent live while it is expanded.
9. As a Chat user, I want a collapsed `Agent` card to show the correct count, tokens, and status even on a cold page reload of a finished task, so that the summary is not blank until I interact.
10. As a Chat user, I want subagent transcripts attached to the correct `Agent` tool call, so that parallel subagent work reads clearly regardless of whether I loaded the Task live or on open.
11. As a Chat user, I want the file tree to refresh promptly after the agent edits a file, so that I see the agent's changes without a manual refresh.
12. As a maintainer, I want a Message's content stored once, so that the event log does not carry a permanent second copy of what the Message store already holds.
13. As a maintainer, I want the `agent.message` source event persisted as a metadata skeleton, so that the event log's largest payloads stop accumulating in a table that never prunes.
14. As a maintainer, I want the Events/diagnostics timeline to keep showing that an assistant Message was emitted (its role, tool names, block count), so that lifecycle debugging is unaffected while content lives in the Chat.
15. As a maintainer, I want the `moat events` diagnostic output to show the skeleton and link to content by the Message's `uuid`, so that operators can still cross-reference to the Chat.
16. As an operator, I want each connected client to receive Message content once on the live wire, so that streaming does not double-ship the same content as a source event and a derived event.
17. As an operator, I want the live wire to never carry subagent transcript content, so that a chatty or wide subagent fan-out does not flood the firehose that also carries lifecycle events.
18. As an operator, I want existing fat source-event rows slimmed by a backfill, so that historical storage is reclaimed in a table with no retention policy.
19. As a maintainer, I want read and live Message assembly for top-level Messages to cross one seam, so that a change to assembly is made and tested in one place and the two paths cannot drift.
20. As a maintainer, I want the subagent summary shown on the read path and the live progress event to be the same shape, so that a collapsed card has exactly one source of truth reconciled by construction.
21. As a maintainer, I want the frontend to render pre-computed subagent summaries rather than derive them by scanning every subagent message, so that the collapsed card no longer depends on having the whole transcript loaded.
22. As a maintainer, I want tool-based cache invalidation to read a `toolNames` field from the source event, so that a transport reaction stays on the source event and does not couple to the Chat's rendering stream.
23. As a developer of the Chat, I want the frontend to keep rendering (not assemble), so that ADR-0002's server-owned-assembly boundary is preserved and strengthened.
24. As an AFK/agent implementer, I want the correctness fix (live tool results) verifiable before the destructive backfill runs, so that the risky migration lands only after behavior is proven.
25. As a Chat user, I want assembly to load the affected Message together with its folded siblings precisely, so that a long Turn's results are not missed by a fixed newest-page window.
26. As a maintainer, I want the source event's full content to remain available transiently to the backend, so that the Message store can still be built from it during ingestion.
27. As a reviewer, I want the change to reference ADR-0002/0003 and the domain glossary, so that its intent is legible against the recorded decisions.
28. As a maintainer, I want readers to tolerate both source-event shapes only transiently, so that after the backfill exactly one shape remains and dual handling is removed.
29. As a maintainer, I want the raw per-block emission log's removal recorded as an accepted trade-off, so that a future reviewer understands forensic reconstruction of "what the Sandbox sent, in order" is intentionally gone.
30. As a product owner, I want the queued-message cancel affordance removed in this change, so that the message routes are simplified; the pending-message list becomes view-only.

## Implementation Decisions

### Top-level assembler correctness (unchanged from ADR-0003)

- **One Message-assembly seam for top-level Messages.** Both the paginated read
  path and the live derived stream cross a single server-side assembler. It owns:
  loading the affected Message together with its folded siblings, file-context
  hydration, shaping to the UI-facing form, and **affected-parent resolution**.
  The pure row-transform stays an internal seam behind it.
- **Affected-parent resolution.** When the changed row folds into another Message
  (a `tool_result`/`system` row inlined into the assistant Message that called
  it), the derived Message upsert targets the **parent** assistant Message, keyed
  by the parent's `uuid`. A standalone Message still upserts by its own `uuid`.
  This is what makes top-level tool results render live under their call.
- **Dedicated affected-Message loader.** Replace the fixed newest-page assembly
  window with a precise fetch of the affected Message plus its `tool_result`/
  system siblings, its contexts, and its content blocks.

### Lazy subagents (revises ADR-0003's subagent decision)

- **No subagent nesting in the Message.** The `UiMessage.subagentsByToolUseId`
  field and all server-side subagent nesting/grouping are **removed**. Subagent
  content is never delivered inside the parent Message upsert.
- **Subagent summary on the `Agent` tool-use content block.** Each `Agent`
  `tool_use` block carries a small, derived summary:

  ```
  SubagentSummary { messageCount: u32, tokens: u32, status: "running" | "completed" | "error" }
  ```

  - `messageCount` = `COUNT(*)` of the subagent's rows (rows whose `parent_uuid`
    equals the `Agent` tool-use id).
  - `tokens` = `SUM(input_tokens + output_tokens)` across the subagent's assistant
    rows.
  - `status` derived from the **outer** `Agent` tool_result: present + `is_error`
    → `error`; present → `completed`; absent → `running`.

  The summary is *derived aggregate metadata*, not content. The frontend renders
  the collapsed `Agent` card as a pure read of `block.subagentSummary`.
- **Top-level read excludes subagent rows.** `GET /messages` returns only
  top-level Messages. Subagent rows carry `parent_uuid` (the spawning `Agent`
  tool-use id) and are filtered out in SQL (`parent_uuid IS NULL`), so pages fill
  with top-level rows and the keyset cursor stays clean — no empty pages. One
  batched aggregate (`GROUP BY parent_uuid` over the page's `Agent` tool ids)
  injects each `subagentSummary` post-transform via a shared helper used by both
  the read path and the live top-level emit.
- **On-demand subagent transcript endpoint.** A new
  `GET /api/v1/tasks/{task_id}/tool-calls/{tool_use_id}/messages` — the sibling of
  the existing `tool-calls/{tool_use_id}` detail route — returns the assembled,
  fold-resolved subagent transcript (`{ messages: UiMessage[] }`) by running the
  same row-transform over the rows whose `parent_uuid` equals the tool-use id. No
  pagination initially (bounded per call, fetched on demand).
- **Live subagent progress event.** A subagent `agent.message` persist emits a
  new `message` / `subagentProgress` derived event — `{ toolUseId, messageCount,
  tokens, status }`, **absolute values** — and **no** parent Message upsert. It
  rides the existing `message.*` UI-only lane (gated out of `task_events`,
  relayed live). The frontend **writes through** the absolute summary onto
  `block.subagentSummary` of the cached parent Message (idempotent — a replayed
  or duplicated tick is a no-op; self-heals after reconnect).
- **Expand-driven transcript refresh.** Expanding an `Agent` card fetches the
  transcript endpoint (query keyed by `toolUseId`); each `subagentProgress` for
  that id triggers a debounced (~300ms) refetch of that query. Collapsing leaves
  the query cached. No subagent content ever crosses the firehose.

### Source-event skeleton (unchanged from ADR-0003)

- **Source-event skeleton.** The `agent.message` source event — top-level *and*
  subagent — is projected to a metadata skeleton wherever it is **persisted** or
  **relayed to clients**. The ingestion path that builds the Message store keeps
  the full content; the full content is never persisted to the event log nor
  relayed to clients. Skeleton shape (this shape *is* the decision):

  ```json
  { "message": { "role": "assistant" }, "uuid": "…", "blockCount": 3, "toolNames": ["Bash", "Edit"] }
  ```

  `role` nested under `message` keeps the timeline's existing role reader working
  for both old and new rows; `toolNames` is a small derived list, not content.
- **One shared projection, applied at every persist/relay point.** A single pure
  projection reduces the source event to the skeleton; applied at each place the
  event is written to the log or broadcast to clients, and nowhere on the
  ingestion path that builds the Message store. It must never touch the Redis
  full-content path the ingestion consumer reads.
- **Tool-invalidation reads `toolNames`.** The files-invalidation reaction reads
  the skeleton's `toolNames` field (a field read, no content parse) and stays on
  the source event. The derived Message upsert remains purely the Chat's
  rendering concern.
- **Backfill.** A batched, idempotent migration slims existing fat `agent.message`
  rows to the skeleton, dropping their raw per-block content. The assembled
  content remains in the Message store. Batched by id range to avoid long locks on
  a large, unpruned table; gated on the payload still being fat.
- **Content is single-sourced.** After this change a Message's content lives once
  (Message store); the event log and client wire carry only the skeleton; the
  Chat receives top-level content only via the Message upsert and subagent content
  only via the on-demand transcript endpoint.

### Removed: queued-message cancel

- **`DELETE /api/v1/tasks/{task_id}/messages/{message_id}` is removed**, together
  with the now-sole `message.deleted` emit and the frontend queued-cancel
  affordance (the composer's pending-message ✕ and the `message.deleted` →
  remove-row handler). The pending-message list becomes view-only. This is a
  deliberate cleanup carried in this change, independent of the subagent rework.

## Testing Decisions

A good test asserts **external behavior at the highest seam** — what a client
observes on the event stream / read path, or what the assembler returns for a set
of rows — never internal wiring. Prefer existing seams; the fewest, highest seams
possible.

- **Seam 1 — agent-Turn assertion over the live stream + read path (highest,
  end-to-end).** Drive a real Turn that uses a top-level tool and spawns a
  subagent, and assert on the live derived events and the read path:
  - the **parent** assistant Message's Message upsert arrives **after** its
    `tool_result`, carrying the inlined result, **before** the Turn-final
    reconcile (the correctness fix);
  - the `agent.message` source event on the stream and in the events history is
    the **skeleton** (role + `toolNames` + `blockCount`, no full content);
  - the top-level `GET /messages` response **excludes** subagent rows and each
    `Agent` tool block carries a `subagentSummary`;
  - subagent activity emits a `message` / `subagentProgress` event with absolute
    `{ toolUseId, messageCount, tokens, status }` and **no** parent re-upsert;
  - `GET …/tool-calls/{tool_use_id}/messages` returns the assembled subagent
    transcript;
  - (regressions from ADR-0002) no `message`-type rows are persisted, and sending
    a message returns the persisted `uuid`.

  Prior art: the agent-tier Rust integration tests that drive Turns and assert
  Message behavior (message acknowledgment, subagent tool tracking, agent
  execution flow) and the env-agnostic cluster Playwright suite's
  agent-build-page spec.

- **Seam 2 — the pure assembler / loader unit test (internal seam).** Feed rows
  and assert:
  - a `tool_result` inlines into its parent and affected-parent resolution returns
    the parent's `uuid`;
  - the affected-Message loader returns the target plus its folded siblings;
  - the top-level transform excludes subagent rows (rows carrying `parent_uuid`);
  - the transcript assembly (transform over `parent_uuid = tool_use_id`) returns
    the subagent's folded Messages;
  - the summary aggregate produces the expected `messageCount` / `tokens` /
    `status`.

  Prior art: the existing Message row-transform unit tests and the Message
  DAO/handler tests.

- **Seam 3 — backfill migration check (targeted).** Seed a fat `agent.message`
  row, run the backfill, and assert the row is reduced to the skeleton shape and
  the assembled content in the Message store is untouched. Prior art: the DAO
  tests that insert `task_events` rows directly.

Frontend note: no new frontend test seam. Subagent grouping is deleted server-side
(its frontend grouping test is removed — behavior moves behind Seam 1/2). The
collapsed-card summary write-through and the expand-driven refetch are covered
behind Seam 1/2 (the summary shape and progress event are asserted there). The
tool-invalidation change is covered by extending the existing event-handlers unit
test to feed a `toolNames` skeleton.

## Out of Scope

- **Frontend event→cache projection consolidation** (the deferred "collapse the
  live projection" candidate — one deep in-process projection module replacing the
  smeared cache-policy helpers). Compatible follow-up; the `toolNames`-on-skeleton
  and `subagentProgress` write-through here land in its territory without conflict.
- **`task_events` retention/pruning.** This change reduces per-row size; the
  append-forever growth of the log is a separate, orthogonal concern.
- **The Sandbox emission / Redis stream payload.** Full content still travels
  Sandbox → Redis → the backend event handler (which needs it to build the
  Message store). Only the *persisted* and *client-relayed* copies are slimmed.
- **Pagination of the subagent transcript endpoint.** Returned whole initially;
  keyset pagination can be added later without a contract change.
- **Streaming an expanded subagent transcript message-by-message.** Expanded
  transcripts refresh by debounced refetch on progress ticks, not by streaming
  content; a per-`toolUseId` live subscription is a possible future refinement.
- **Moving assembly back to the frontend** — explicitly not done; ADR-0002's
  server-owned boundary is preserved and strengthened.

## Further Notes

- Refines ADR-0002 and is recorded against ADR-0003. This revision keeps ADR-0003's
  skeleton and affected-parent decisions and **supersedes its subagent-relations
  decision** (server-side nesting delivered in the parent upsert) with the lazy
  model above; the ADR body and its "Consequences" for subagent relations are
  updated to match when this lands. The two `CONTEXT.md` glossary deltas — a source
  event's content is a transient ingestion payload / its persisted-and-relayed form
  is a skeleton, and a Message upsert targets the affected parent — land with the
  code, not ahead of it. A third glossary note records that subagent transcripts
  are lazy (summary on the read path + progress on the wire; content on demand).
- **Why lazy over nesting:** nesting the subagent subtree into the parent Message
  and re-emitting it on every subagent step re-ships the full growing transcript on
  the live wire (`O(N²)` bytes over a run) and bloats the top-level read with
  content most of which is never expanded. The lazy model ships each fact once: a
  small absolute summary on the wire, full transcript only on demand.
- **Accepted trade-off (forensic log):** the raw per-block emission log is removed,
  so "what the Sandbox sent, and in what order" is no longer reconstructable from
  persisted state. The assembled Message in the Message store is the canonical
  record, and the raw log had no reader — so the forensic loss is accepted.
- **Accepted trade-off (expanded liveness):** an expanded subagent transcript
  updates by debounced refetch rather than message-by-message streaming — a small
  lag on a user-initiated detail view, in exchange for never streaming subagent
  content.
- **Sequencing:** land and verify the top-level assembler correctness fix (live
  tool results) before the destructive backfill runs. Migrate the source-event
  content consumers off the raw payload before the relay is slimmed, or those
  reactions break. Extend the browser monitor used during ADR-0002 verification to
  assert a top-level tool **result** renders live under its call. Measure the
  `task_events` storage delta across the backfill.
- **Manual verification precedent:** the ADR-0002 seam was verified end-to-end
  against the local k8s deployment (live `message.upsert` streaming, no-persist
  gate, `POST /messages` returns `uuid`, and a headless browser confirming DOM
  updates within ~2ms of each upsert). The same harness extends to the new
  behaviors above, plus: a collapsed `Agent` card's count/tokens advancing on
  `subagentProgress`, and expanding it fetching the transcript.
