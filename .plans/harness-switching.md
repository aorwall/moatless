# Harness switching and a Trajectory-backed conversation

Specification and implementation plan. Self-contained: every decision, table,
contract and phase needed to build it is in this file.

- [Part 1 — Specification](#part-1--specification)
- [Part 2 — Implementation plan](#part-2--implementation-plan)

---

# Part 1 — Specification

## 1. What changes

1. **A Task can switch between the Claude Code and Codex harnesses between
   turns** and keep its whole conversation. The new harness receives the earlier
   turns as real turns in its own session file, transcoded from the other
   harness's format.
2. **The conversation has three sources of truth and one derived view.**
   - The **Trajectory** records what the harness wrote: every line of every
     session file, verbatim, plus its harness-neutral content.
   - The **inbox** records what was sent to the agent, by whom, through which
     channel, and whether it has been delivered.
   - **`task_events`** records what happened to the Task: sandbox, turn and agent
     lifecycle, including errors.
   - The **conversation read model** is what the Chat renders. One **Projector**
     writes it from the three sources, and nothing else does.
3. The streaming path that fills `task_messages` today (`agent.message` →
   event outbox → `handle_agent_message`) is removed. `task_messages`,
   `task_activities` and `task_message_content_blocks` are replaced.
4. **One name per concept** for what runs a Task (section 2.1).

## 2. Decisions

| # | Question | Decision |
|---|---|---|
| 1 | How the new harness receives the prior conversation | **Transcode.** The other harness's turns (text, tool calls, tool results) become real turns in the target session file. The target's own turns stay verbatim. The other harness's reasoning is dropped. |
| 2 | How a switch appears in the Trajectory | **Materialise at switch.** A new session of the target harness whose first lines are the transcoded prefix, stored once as its own events. The lineage is a new `transcoded_from_event_id`, never `forked_from_event_id`. |
| 3 | Where the Chat reads from | **Three sources, one derived conversation** (section 1). The sandbox tails session and subagent files and ships each line as it appears. Recording is always on. |
| 4 | Where transcoding runs | **In the sandbox**, in `write_out` at the first turn after a switch. The ordinary tail records the result. |
| 4a | History recorded before the read model | **Frozen.** Every `task_messages`/`task_activities` row, with a Trajectory or not, is copied once into the conversation read model, marked `frozen`. The Projector never rebuilds or deletes them, with one exception: a line of the turn open at the freeze (`conversation_frozen.open_since`) may settle a frozen tool call it answers. |
| 5 | How a switch is requested | An explicit Agent profile: the T3 instance picker (`instanceId`), `agent_profile` on `PATCH /tasks/{id}`, or `moat tasks update --agent-profile`. A model alone never switches. |
| 6 | Switch while a turn runs | Accepted. The running turn finishes on the old harness; the next turn runs the new one. |
| 7 | No credential for the target harness | Refused at request time, with the create-time message. |
| 8 | Switching back (A → B → A) | The new A session forks A's own latest session verbatim and transcodes only B's lines since. |
| 9 | Rewind or fork to a turn the other harness ran | The result runs the Task's current harness; the history is transcoded. |
| 10 | Compaction inside the part being transcoded | Transcode from the last compaction: its summary becomes one user text turn. |
| 11 | Claude subagent transcripts | Recorded as their own sessions; not transcoded. |
| 12 | No Trajectory, or a gap | Switching is refused on a frozen Task. A gap fails the first turn after the switch with a visible error. |
| 13 | Naming | **Harness** means exactly `claude-code` or `codex`. What `agent_type` holds today is renamed **Agent profile**. |

### 2.1 Harness and Agent profile

`CONTEXT.md` already defines **Agent harness** as the executable family and
lists "Agent type" and treating `claude-code-tui`/`-cli`/`-fable` as harnesses
under *Avoid*. The code does not follow it yet:

| Today | Holds | Becomes |
|---|---|---|
| `AgentType(String)` (`types/src/agent.rs:260`), every `agent_type` column, field and env var | a catalog entry id: `claude-code`, `claude-code-tui`, `claude-code-cli`, `claude-code-fable`, `codex` | **`AgentProfile`** / `agent_profile` — a harness plus its transport, default model and credentials |
| `AgentKind` (`tasks/src/agent/catalog.rs:17`) and `trajectory::Harness` (`tasks/src/trajectory/models.rs:9`) — two types for one fact | `ClaudeCode`, `Codex` | **one `Harness` enum** in `moatless-types`, serialised `claude-code` / `codex` |
| `AgentEntry` (`catalog.rs`) | a profile's definition | `AgentProfileEntry`; `kind` becomes `harness` |
| `HARNESS_USER_TYPE` (`"harness"`) | a prompt the sandbox wrote to wake its own agent | inbox `origin = 'sandbox'` (4.2); the constant is deleted with `task_messages` |

The catalog owns the profile → harness mapping. Nothing else stores or derives
it: code that needs a Task's harness asks the catalog for its profile's harness.
`trajectory_sessions.harness` is a different fact — the format of that file —
and stays.

A switch is a change of Agent profile. It is a **harness switch** when the new
profile's harness differs; a profile change within one harness (e.g.
`claude-code` → `claude-code-fable`) resumes verbatim and transcodes nothing.

## 3. Architecture

```
 Clients                     Backend                                   Sandbox pod
 ───────                     ───────                                   ───────────
 T3 / REST / CLI  ──(1)──▶  task_inbox (pending)
 Adapters, Loops               │
                               │(2) Turn runner: GetPendingMessages
                               ▼
                         PendingAgentDelivery ───────(3)──────────▶  Member coordinator
                         {inbox_ids, agent_profile}                   │ rebuild harness if the
                                                                      │ profile changed
                                                                      ▼
                                                                  write_out: ReadTrajectory
                                                                  {resume | fork | switch}
                                                                      ▼
                                                                  Harness CLI ──(4)──▶ session files
                                                                                          │
                         AppendTrajectory  ◀─────────(5)─────────  Tail + classifier ◀────┘
                           │  one transaction:                     (harness format → neutral
                           │   insert trajectory_events             content, here only)
                           │   Projector → conversation_*
                           │   advance task_conversation_changes
                           ▼
                     conversation_messages / _activities ──(7)──▶ T3 (Message upsert)
                           ▲
 task_events ──────(6)─────┘  errors, turn results
```

1. A send writes one `task_inbox` row (`pending`). The Projector writes the
   user Message in the same transaction.
2. The Turn runner reads pending inbox items by `status`.
3. The delivery carries the inbox ids and the Task's current Agent profile. The
   Member's coordinator rebuilds its harness when the profile differs from the
   one it runs (between turns only), then `write_out` prepares the session file.
4. The CLI appends lines to its session file (Claude also to
   `subagents/agent-*.jsonl`).
5. The tail classifies each complete line into harness-neutral content and ships
   it. The backend inserts the events and runs the Projector in one transaction.
   `AcknowledgeMessages` marks the delivered inbox items, as today.
6. Lifecycle events still go to `task_events`; errors and turn completions are
   projected from there.
7. T3 reads the read model and follows `task_conversation_changes`, as it does
   today.

**Where each harness's format is known.** Only in the sandbox's `SessionFormat`
implementations: reading a line (`read_line`), rewriting ids (`retarget`),
writing lines (`render`). The backend stores `payload` verbatim for replay and
reads only the neutral `content`. No backend code branches on a harness's line
shape.

## 4. Data model

Migration numbers are the next free ones when each phase lands (the last today
is `0172_task_turn_triggered_by.sql`). Line references are to `origin/main` at
`480ab0d9`.

### 4.1 Harness-neutral line content (`trajectory_events.content`)

The classifier's output, and the only part of a line the backend interprets.

```jsonc
{
  "role": "user" | "assistant",
  "blocks": [ /* CanonicalContentBlock: text | tool_use | tool_result | thinking */ ],
  // New. The harness wrote this line itself: Codex environment and instruction
  // messages; Claude command, hook and system-reminder output. Never shown and
  // never transcoded.
  "injected": false,
  // New. On the first line of a model response. Claude repeats the response's
  // usage on each of its lines; the tail drops it where `harness_item_id`
  // equals the previous line's.
  "usage": { "input_tokens": 0, "output_tokens": 0,
             "cache_read_tokens": 0, "cache_write_tokens": 0 },
  // New. On a compaction line; its summary text is in blocks.
  "compaction": false,
  // New. The model that wrote an assistant line, when the harness records it.
  "model": "claude-opus-4-5",
  // New. On a line that restates part of a call already recorded, not a call
  // of its own: the Codex app-server's `CommandExecution` item for a command
  // an `exec` call ran. Names that call's `tool_use` id; never transcoded.
  "derived_from": "call_7o2cP988tSqkhfkN2WCacs8H",
  // New. The harness recorded that the turn's final response finished here:
  // a Claude assistant text line with `stop_reason` `end_turn` (its thinking
  // line repeats the stop reason and does not count), or Codex's
  // role-less `task_complete` line. Recovery closes a turn as answered only
  // on this; assistant text alone may precede a tool call not yet written.
  "ends_turn": true
}
```

A Codex `CommandExecution` line carries no call id; the tail names the latest
call still open in the session, and rebuilds the open calls from the skipped
lines when it restarts. Codex names its model on `turn_context`, not on a
reply, so the tail sets the latest one as `model` on each Codex assistant line.

A housekeeping line (session metadata, turn context, file snapshots) has a null
`role` and no `blocks`. `kind` keeps the harness's own line type for diagnostics only.

### 4.2 `task_inbox` (new)

Every message addressed to the agent. Sender and channel are typed columns.
`origin` decides which of them are set, and one `CHECK` per origin states the
exact shape, so a row cannot claim one origin and carry another's sender.

```sql
CREATE TABLE task_inbox (
    id                    uuid PRIMARY KEY,
    task_id               text NOT NULL,  -- no FK: a fork's Message outlives its ancestor
    created_at            timestamptz NOT NULL DEFAULT now(),

    -- Content, as sent. The Chat shows this, not the CLI's copy of it.
    text                  text NOT NULL,
    file_ids              text[] NOT NULL DEFAULT '{}',
    command_name          text,
    delivery_options      jsonb,          -- today's task_messages.sdk_metadata
    answers_tool_use_id   text,           -- an answer to a parked AskUserQuestion / ExitPlanMode

    origin                text NOT NULL
                          CHECK (origin IN ('user', 'task', 'adapter', 'system', 'sandbox', 'unknown')),

    -- Sender
    sent_by               text,           -- Moatless user
    source_task_id        text,           -- the Task that sent it (moat send --task)
    external_sender_id    text,           -- adapter-side id (Slack U0…); resolves via user_provider_ids
    external_author       text,           -- the handle the external system shows

    -- Channel (adapter origin only)
    adapter               text,           -- slack | linear | github_pr | …   (was channel_provider)
    external_message_id   text,           -- adapter's id for the message     (was channel_external_id)
    external_subject_key  text,           -- thread or subject, e.g. owner/repo#N
    external_source_name  text,           -- what matched the subscription    (was channel_source_name)

    -- Delivery
    status                text NOT NULL DEFAULT 'pending'
                          CHECK (status IN ('pending', 'delivered', 'cancelled', 'rewound')),
    delivered_at          timestamptz,
    rewound_at            timestamptz,
    run_id                text,
    turn_number           integer,
    session_id            text,           -- the agent session the delivery ran in; recovery resumes it

    CONSTRAINT task_inbox_sender_check CHECK (CASE origin
      WHEN 'user'    THEN sent_by IS NOT NULL
                          AND num_nonnulls(source_task_id, adapter, external_message_id,
                                           external_sender_id, external_author) = 0
      WHEN 'task'    THEN source_task_id IS NOT NULL
                          AND num_nonnulls(adapter, external_message_id,
                                           external_sender_id, external_author) = 0
      WHEN 'adapter' THEN adapter IS NOT NULL AND external_message_id IS NOT NULL
                          AND source_task_id IS NULL
      WHEN 'system', 'sandbox'
                     THEN num_nonnulls(sent_by, source_task_id, adapter, external_message_id,
                                       external_sender_id, external_author) = 0
      WHEN 'unknown' THEN status IN ('delivered', 'rewound')   -- backfilled history only
    END),
    CHECK (adapter IS NOT NULL OR num_nonnulls(external_subject_key, external_source_name) = 0),
    CHECK ((status IN ('delivered', 'rewound')) = (delivered_at IS NOT NULL)),
    CHECK (status IN ('delivered', 'rewound') OR num_nonnulls(run_id, turn_number, session_id) = 0),
    CHECK ((status = 'rewound') = (rewound_at IS NOT NULL))
);

CREATE UNIQUE INDEX task_inbox_external_message_idx
    ON task_inbox (task_id, adapter, external_message_id) WHERE external_message_id IS NOT NULL;
CREATE INDEX task_inbox_pending_idx ON task_inbox (task_id, created_at) WHERE status = 'pending';
CREATE INDEX task_inbox_sent_by_idx ON task_inbox (sent_by);
CREATE INDEX task_inbox_turn_idx ON task_inbox (task_id, turn_number);
CREATE INDEX task_inbox_latest_queued_idx
    ON task_inbox (task_id, created_at DESC) WHERE status IN ('pending', 'delivered');
```

`rewound`, `rewound_at` and the dropped task FK arrive in phase 5 (0178). A
rewind deletes the delivered items past its cut, except one a fork's inherited
Message reads: that one turns `rewound` and keeps its turn and delivery, so the
fork's `rebuild` still replays it (4.9a). Deleting a Task deletes the items no
other Task's conversation reads.

The origin `CHECK` is named `task_inbox_sender_check`: Postgres auto-names the
column `CHECK` on `origin` `task_inbox_origin_check`.

An adapter send may carry `sent_by` too: the Moatless user an external sender
resolved to. `unknown` exists only for delivered history whose sender did not
record. The backfill moves pending rows only, so a pending row with no recorded
sender is `system`, which the `CHECK` allows while pending.

Column origins:

| `task_inbox` | From `task_messages` |
|---|---|
| `id`, `task_id`, `created_at` | `uuid`, `task_id`, `timestamp` |
| `text`, `file_ids` | `message_text`, `attachments` |
| `delivery_options` | `sdk_metadata` |
| `origin` | set by each send path (section 5.5); backfill: `user_type = 'harness'` → sandbox, `source_task_id` → task, `channel_provider` and `channel_external_id` → adapter, `sent_by` → user, otherwise system |
| `sent_by`, `external_sender_id`, `external_author`, `source_task_id` | same names |
| `adapter`, `external_message_id`, `external_source_name` | `channel_provider`, `channel_external_id`, `channel_source_name` |
| `external_subject_key`, `command_name` | same names |
| `origin = 'sandbox'` | `user_type = 'harness'` (`HARNESS_USER_TYPE`, `types/src/message.rs:35`) |
| `status = pending` | `message_type = 'user' AND agent_session_id IS NULL` (`message/dao.rs:899`) |
| `delivered_at`, `turn_number`, `run_id`, `session_id` | stamped by `AcknowledgeMessages` today (`message/dao.rs:1052`); `session_id` was `agent_session_id` |

An inbox id is a `uuid`, so every send path now refuses a client message id
that is not one. A queued Message whose id is not a UUID takes
`md5('task_inbox:' || task_id || ':' || uuid)::uuid` in the backfill, and the
rows naming it (activities, usage, conversation changes, feedback) follow.

`sandbox` is a prompt the sandbox wrote to wake its own agent: a finished
background command, a `ScheduleWakeup` prompt, a restart note
(`EnqueueSandboxMessage`, `message/send.rs:114`). Nobody said it: no conversation
shows it, and it never decides who triggered a turn (4.5a). `system` is a prompt
the platform sent, which the Chat does show.

### 4.3 `trajectory_sessions` (changed)

```sql
ALTER TABLE trajectory_sessions
    -- The other harness's tip this session's prefix was transcoded from.
    -- Lineage only: chain() never follows it.
    ADD COLUMN transcoded_from_event_id uuid REFERENCES trajectory_events (id),
    -- First seq the CLI itself wrote. Lines below it are the transcoded prefix:
    -- recorded, never projected.
    ADD COLUMN own_from_seq bigint,
    -- A subagent session: the parent's tool-call event that spawned it.
    ADD COLUMN parent_event_id uuid REFERENCES trajectory_events (id),
    -- `own_from_seq` is NULL until the session's first append sets it.
    ADD CHECK (own_from_seq IS NULL OR transcoded_from_event_id IS NOT NULL),
    ADD CHECK (parent_event_id IS NULL
               OR num_nonnulls(transcoded_from_event_id, forked_from_event_id) = 0);
```

Plus a constraint trigger: `forked_from_event_id` must point into a session with
the same `harness`. It spans two rows, so it cannot be a `CHECK`.

A top-level session can carry both links. On a switch back (A → B → A) the new A
session has `forked_from_event_id` = A's earlier tip and
`transcoded_from_event_id` = B's tip.

### 4.4 `trajectory_events`

`id`, `session_id`, `seq` (`UNIQUE (session_id, seq)`, the line's index in its
file, prefix included), `parent_event_id`, `kind`, `harness_item_id`,
`harness_turn_id`, `turn_number`, `payload` (verbatim line), `content`
(section 4.1). Only the tail writes it; a row is never updated. The session a
turn ran in is `SELECT DISTINCT session_id … WHERE turn_number = $n` over the
Task's top-level sessions; no other table stores it.

New: `ord bigint NOT NULL`, from one sequence, is the lineage order. An `id` is
a UUIDv7 from the writing backend's clock, so ids from two replicas need not
follow write order; an event is inserted only after its parent and any tip it
was transcoded from. Existing rows are numbered in `id` order.

### 4.5 Renames (section 2.1)

```sql
ALTER TABLE tasks            RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE task_turns       RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE sandboxes        RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE task_events      RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE loop_configs     RENAME COLUMN agent_type TO agent_profile;
-- These five are every agent_type column left; the channel, route and
-- template tables that had one are dropped.
```

- `tasks.agent_profile` is the profile the **next** turn runs; written at create
  and by a switch.
- `task_turns.agent_profile` is the profile that ran that turn.
- `sandboxes.agent_profile` is the profile a replacement pod boots its first
  harness with. A switch of the Task that owns the Sandbox writes it; a joined
  Member's switch leaves it. A Member's profile for a turn is the delivery's,
  never the pod's. `sandboxes.use_tmux` stays pod shape.

### 4.5a `task_turns` trigger (kept, source moved)

`0172` added `triggered_by_task_id` and `trigger_decided`. A `created_by` child
notifies its parent only for turns the parent triggered (#1011). The rule stays;
its source moves from `task_messages` to `task_inbox`:

- The first `mark_delivered` for a turn decides it, in the same transaction:
  `triggered_by_task_id` = `source_task_id` of the earliest non-`sandbox` item of
  that delivery, NULL when a person or the platform opened it.
- A delivery of only `sandbox` items (a wake) takes the previous turn's
  trigger, so the parent still hears about the turn its request armed.
- Later deliveries into an open turn leave it.

```sql
ALTER TABLE task_turns
    ADD CHECK (trigger_decided OR triggered_by_task_id IS NULL);
```

The trigger stays stored, not derived from the inbox: it is decided once, from
the first delivery, and the wake fallback reads the previous turn's decision.

### 4.6 `task_forks` (changed)

```sql
ALTER TABLE task_forks ADD COLUMN source_event_id uuid REFERENCES trajectory_events (id);
-- after every reader has moved (phase 5):
ALTER TABLE task_forks
    DROP COLUMN source_session_id,
    DROP COLUMN source_message_uuid,
    DROP COLUMN source_agent_turn_id;
```

`source_event_id` is the cut, as one event; its session and harness are read
through it. "The fork's first turn has not run" is not stored: it is "the fork
Task has no top-level `trajectory_sessions` row with an event". This replaces the
`source_session_id` sentinel (`0150_task_forks.sql`).

A frozen Task has no event to name, so forking it is refused (5.2).

Nothing backfills `source_event_id`, and phase 5 (0179) makes it `NOT NULL`: a
row without one is deleted with the legacy anchors, so that Task's next turn
starts a fresh session. "Has not run" is scoped to the fork: no own top-level
session event after `task_forks.created_at`, which a rewind resets to its own
time.

### 4.7 `task_events` (writers changed)

Only the rename in 4.5. `agent.message` is no longer written; existing rows stay
as history.

### 4.8 Conversation read model (new; replaces `task_messages` and `task_activities`)

Written only by the Projector. Every non-frozen row names exactly one source, and
each `kind` fixes which columns it carries. The DDL is
`crates/core/migrations/0178_conversation_read_model.sql`; its shape:

| Table | Holds |
|---|---|
| `conversation_messages` | `uuid` (4.9), `task_id`, `position`, `author` (`user`/`agent`/`system`), `kind` (`text`/`error`/`turn_result`), `text`, `text_parts`, `parent_tool_use_id`, `turn_number`, `harness`, `model`, `usage`, and one source: `inbox_id` (`ON DELETE CASCADE`), `trajectory_event_id` or `task_event_id`. A frozen row carries its other `task_messages` columns in `legacy` jsonb. |
| `conversation_activities` | `activity_id` (4.9), `position`, `ordinal` (block index within the Message), `message_uuid`, `parent_tool_use_id`, `kind`, `status`, `tool_use_id`, `summary`, `payload`, `detail`, `is_error`, and one source: `trajectory_event_id` (+ `result_event_id`), `inbox_id` or `task_event_id` |
| `conversation_usage` | One model response's tokens: live rows keyed `(task_id, trajectory_event_id)`, frozen rows by their mapped `message_uuid`; exactly one of the two |
| `conversation_frozen` | Per Task: `frozen_at`, and the turn open then (`open_turn`, `open_since`) for D4a's exception |

Differences from the first draft, forced by the code:

- A subagent's row names its spawning call by `parent_tool_use_id`, not a
  `parent_uuid`, and positions itself within that transcript; the unique
  position index covers top-level rows only.
- A user Message stores no text: the reader joins `task_inbox`. Session, agent
  session and agent mode are derived at read, not stored.
- `author = 'agent'` with a `task_event_id` is a reply sent through a binding
  (`task_events` `task`/`binding_reply_sent`), enforced by a trigger.
- Activity kinds widen to `tool_call`, `thinking`, `user_input_requested`,
  `plan_updated`, `user_input_resolved` (source: the inbox item),
  `sandbox_status`, `sandbox_warning`, `sandbox_error` and
  `context_window_updated`, each with its own source `CHECK`.
- Activities carry no echo-suppression claim: the echo check reads
  `task_inbox` and `task_outbound_events`.
- Indexes for the shell listing: the newest user Message, the newest agent
  model per turn, and the newest queued inbox item.

`task_conversation_position` and `task_conversation_changes` stay unchanged;
the Projector advances them.

### 4.9 Deterministic ids

UUIDv5 (RFC 9562) in one fixed Moatless namespace, so a rebuilt conversation has
the same ids and no id encodes its source as a string prefix. Every derived id
names the Task whose conversation holds the row (`task_id` below), because a
fork projects the same source rows as its source does (4.9a):

| Row | Id |
|---|---|
| User Message, the Task's own inbox item | `task_inbox.id` |
| User Message, an inherited inbox item | `uuidv5(ns, "message:" ‖ task_id ‖ ":" ‖ task_inbox.id)` |
| Agent Message | `uuidv5(ns, "message:" ‖ task_id ‖ ":" ‖ first trajectory_event_id of the folded run)` |
| Error / turn result Message | `uuidv5(ns, "message:" ‖ task_id ‖ ":" ‖ task_events.event_id)` |
| Tool or thinking activity | `uuidv5(ns, "activity:" ‖ task_id ‖ ":" ‖ trajectory_event_id ‖ ":" ‖ block_index)` |
| Sandbox activity | `uuidv5(ns, "activity:" ‖ task_id ‖ ":" ‖ task_events.event_id)` |
| User-input-resolved activity | `uuidv5(ns, "activity:" ‖ task_id ‖ ":" ‖ task_inbox.id ‖ ":user-input-resolved")` |
| Frozen row | its `task_messages.uuid`; a legacy id that is not a uuid is mapped once, in the migration, to an md5 name-based uuid of `"legacy:" ‖ id` with its version nibble set to 3, so it never equals a Projector id (version 5). Every row naming it (activities, usage, feedback) follows |

### 4.9a A fork's inherited conversation

A fork's conversation is its source's conversation up to the cut, projected
from the source's own sources rather than copied, followed by the fork's own
rows. Walking `task_forks.source_event_id` back gives one segment per ancestor:
the ancestor Task, the cut event in it, and the cut turn `n` (that event's
`turn_number`). Each segment contributes, in the ancestor's order:

| Source in the ancestor | Inherited when |
|---|---|
| `task_inbox` items with `origin <> 'sandbox'`, `delivered`, or `rewound` after the cut | `turn_number <= n`, created at or before the cut |
| Trajectory events on the chain | at or before the cut event |
| `task_events` errors, turn results and sandbox status | `turn_number < n` |

An inherited row keeps its source columns (`inbox_id`, `trajectory_event_id`,
`task_event_id` name the ancestor's row) and takes the fork's `task_id`,
position and 4.9 id. An inherited activity carries no echo-suppression claim
(`adapter`, `external_message_id` NULL), as `copy_conversation` clears
`external_event_id` today. Provenance is derived, not stored: a Message whose
`inbox_id` names another Task's item is inherited, and
`GET /tasks/{id}/messages` serves that Task as its `source_task_id`, as the copy
sets it today. The cut turn's own result is not inherited: the fork records
that turn as `stopped` (phase 5).

An ancestor's later rewind leaves the fork whole: the items it inherited stay
`rewound` (4.2). Deleting an ancestor keeps them too, so the fork's `rebuild`
still replays them.

### 4.10 Removed

| What | When |
|---|---|
| `task_messages`, `task_activities`, `task_message_content_blocks` | Phase 5 |
| `task_forks.source_session_id`, `source_message_uuid`, `source_agent_turn_id` | Phase 5 |
| `agent.message` (sandbox emission, outbox and its `event_ingress_outbox` table, `task_events` row, handler) | Phase 4 |
| `FF_TRAJECTORY_RECORDING`, `FF_TRAJECTORY_RESUME`, `MOATLESS_RESUME_FROM_TRAJECTORY` | Phase 1 |
| `AgentType`, `AgentKind`, `trajectory::Harness`, every `agent_type` name | Phase 1a |

## 5. Contracts

### 5.1 gRPC (`crates/sandbox-proto/proto/sandbox/v1/sandbox.proto`)

Renamed fields keep their numbers, so the wire format is unchanged.

```proto
message TrajectoryLine {
  string kind = 1;
  optional string harness_item_id = 2;
  optional string harness_turn_id = 3;
  string payload = 4;
  string content_json = 5;   // section 4.1
}

message AppendTrajectoryRequest {
  string task_id = 1;
  string harness = 2;
  string harness_session_id = 3;
  int64 first_seq = 4;
  repeated TrajectoryLine lines = 5;   // at most 256 per request
  optional int32 turn_number = 6;
  // New. Only on the first append of a session that has a transcoded prefix.
  optional int64 own_from_seq = 7;
  // New. On a subagent session's appends until one is accepted; read only
  // when the append creates the session. FAILED_PRECONDITION while the
  // spawning tool call is not recorded, and the tail retries.
  optional SubagentParent subagent_parent = 8;
}

message SubagentParent {
  string parent_harness_session_id = 1;
  string tool_use_id = 2;   // the spawning Agent/Task call; resolved to parent_event_id
}

message ReadTrajectoryRequest {
  string task_id = 1;
  string harness = 2;
  oneof target {
    string harness_session_id = 3;
    bool fork = 4;
    // New. The Task's harness differs from its latest top-level session's.
    bool switch = 5;
  }
}

message ReadTrajectoryChunk {
  oneof body {
    SessionHeader header = 1;
    string line = 2;               // verbatim, in seq order (the same-harness chain)
    TranscodeItem transcode = 3;   // new: rendered by the sandbox after the lines
  }
}

message TranscodeItem {
  string content_json = 1;  // section 4.1, never injected
  optional int32 turn_number = 2;
  string source_harness = 3;
}

message PendingAgentDelivery {
  repeated string inbox_ids = 1;   // was message_uuids
  // …
  string agent_profile = 5;        // was agent_type
  // …
  // New. The session the turn continues is the other harness's: the Task's
  // head (the pod opens a switch session, ReadTrajectory { switch }), or with
  // `fork` the session the cut is in. A pod that cannot write it fails the turn.
  bool switch_harness = 14;
}

message AcknowledgeMessagesRequest {
  string task_id = 1;
  repeated string inbox_ids = 2;   // was message_uuids
  // …
}

message ExecuteAgentRequest { /* agent_type → agent_profile, same number */ }
message JoinTaskRequest     { /* agent_type → agent_profile, same number */ }
```

`ReadTrajectory { switch }` creates the switch session the way
`insert_fork_session` (`trajectory/dao.rs:242`) creates a fork session: under an
advisory lock, reusing the session with the same `transcoded_from_event_id` while
it has recorded nothing. A retried read returns the same session.

`AcknowledgeMessages` marks only `pending` items. An item already `delivered`
is left alone and the call succeeds, as the `agent_session_id IS NULL` guard
makes it today.

### 5.2 REST

`agent_type` is renamed `agent_profile` in `api-schemas` (create, read, list).
T3 and the CLI are the only clients and ship in the same release.

`PATCH /tasks/{id}` (`UpdateTaskMetadataRequest`) gains:

```json
{ "agent_profile": "codex", "model": "gpt-5.2-codex" }
```

| Condition | Response |
|---|---|
| `agent_profile` equals the current one | 200, no change |
| Unknown profile | 400 |
| Target profile's credential missing | 400, the create-time message (`missing_credential_hint`) |
| `model` not offered by the target profile | 400 |
| Harness switch on a frozen Task (no Trajectory) | 409 `"This Task was created before recording and cannot switch harness"` |
| Fork (or rewind by fork) of a frozen Task, on `POST /tasks/{id}/fork` | 409 `"This Task was created before recording and cannot be forked"` |
| Target is a Claude Code profile whose `use_tmux` differs from the Sandbox's | 409 `"This Sandbox cannot run <label>"` |
| `model` of another profile without `agent_profile` | 400, as today (`task/handlers.rs:500-506`) |

`GET /tasks/{id}/messages` keeps its response fields through phases 3–5: `role`
(from `author`), `content`, `turnNumber`, `isQueued` (from `task_inbox.status`),
`sourceTaskId`, `model`. The E2E suite reads exactly these, so it guards the
cutover unchanged (Test plan, T0).

### 5.3 T3 (UI-RPC)

`crates/t3code/src/commands.rs::selection_for_existing`. An `instanceId` names a
harness (`claudeAgent`, `codex`), not a profile. For another harness than the
Task's, the backend switches to that harness's first catalog profile, and for
Claude Code the one whose `use_tmux` matches the Sandbox, so a T3 switch never
answers the transport 409:

| Command | Behaviour |
|---|---|
| `thread.turn.start` with another harness's `instanceId` | Switch, then start the turn on it |
| `thread.meta.update` with another harness's `instanceId` | Switch. A refusal is surfaced |
| `thread.fork` with another harness's `instanceId` | The fork runs that harness, without the source's model; otherwise the source's profile and model |
| A bare model slug with no `instanceId` | Never switches (today `:862-864` ignores it) |

### 5.4 CLI

`moat tasks update <id> --agent-profile <id> [--model <m>]`
(`crates/cli/src/commands/tasks/dispatch.rs`, `TaskAction::Update`). Every
existing `--agent`/`--agent-type` option is renamed `--agent-profile`.

### 5.5 Inbox origin per send path

| Send path | `origin` |
|---|---|
| T3 / REST send by a signed-in user | `user` |
| `moat send --task` from another Task | `task` |
| Adapter ingress (`event_ingress/admission.rs`, `relay.rs`) | `adapter` |
| Child turn notices to a parent (`notify_parent_of_child_outcome`, `agent_handlers.rs`) | `task`, `source_task_id` = the child; id is today's UUIDv5 of (child, parent, turn) |
| Schedule firings and adapter-bound Loop runs (they carry an adapter and an event id) | `adapter` |
| Loop runs with no adapter, platform prompts | `system` |
| Sandbox wakes (`EnqueueSandboxMessage`): background command done, `ScheduleWakeup`, restart note | `sandbox`; id is today's `agent_send_idempotency_uuid` |

A send path that derives its id makes `submit` idempotent: a redelivered event
finds the item and returns `None`, and only an insert wakes the Sandbox, as
#1004 does today.

## 6. Interfaces

Signatures only; bodies are the implementer's.

### Types (`crates/types`)

```rust
/// The executable family that runs an Agent. The only harness type.
pub enum Harness { ClaudeCode, Codex }            // "claude-code" | "codex"

/// A catalog entry: a harness plus transport, default model and credentials.
pub struct AgentProfile(pub String);              // was AgentType
```

### Backend (`crates/tasks`)

```rust
// crates/tasks/src/agent/catalog.rs
pub fn harness_of(profile: &AgentProfile) -> Option<Harness>;

// crates/tasks/src/task/switch.rs — refusals are `TaskError::BadRequest` (400)
// and `TaskError::Conflict` (409), the variants every task route answers with.
impl TaskSwitchService {
    /// Sets the Agent profile the Task's next turn runs. A running turn is untouched.
    pub async fn switch_profile(
        &self, scope: &ScopeContext, task_id: &str, profile: &str, model: Option<&str>,
        credentials: SwitchCredentials<'_>,
    ) -> Result<SwitchOutcome, TaskError>;
    /// The profile a switch to `harness` picks, for T3 (5.3).
    pub async fn profile_for(&self, sandbox_id: Option<&str>, harness: Harness) -> Result<&'static str, TaskError>;
}

// crates/tasks/src/inbox/ — crate-private: `persistence-export` refuses a public
// signature naming a transaction. Hosts reach the inbox through `Messages`
// (`pending_user`, `acknowledge_messages`) and `MessageSendService`.
impl InboxService {
    /// `None` when an item with this id exists: a redelivery, which must not wake again.
    pub(crate) async fn submit(&self, tx: &mut PgTransaction, item: NewInboxItem) -> Result<Option<InboxItem>, sqlx::Error>;
    /// Oldest first, at most `limit`.
    pub(crate) async fn pending(&self, task_id: &str, limit: u32) -> Result<Vec<InboxItem>, sqlx::Error>;
    /// Marks the `pending` ones among `ids`; already-delivered ids are skipped.
    /// Decides the turn's trigger (4.5a) from the items this call moved.
    pub(crate) async fn mark_delivered(&self, tx: &mut PgTransaction, task_id: &str, ids: &[Uuid], run: &RunStamp) -> Result<u64, sqlx::Error>;
}
// No `cancel`: no send path or UI cancels a queued item yet, so `cancelled`
// has no writer; readers already skip it.

// NewInboxItem carries the origin as a sum type, so the CHECK in 4.2 is never
// the first place a bad combination is caught.
pub enum InboxOrigin {
    User { sent_by: String },
    Task { source_task_id: String, sent_by: Option<String> },
    Adapter { channel: AdapterChannel, sender: ExternalSender, sent_by: Option<String> },
    System,
    Sandbox,
}

// crates/tasks/src/conversation/projector.rs — the only writer of conversation_*.
impl Projector {
    pub async fn on_inbox(&self, tx: &mut PgTransaction, item: &InboxItem) -> Result<(), ProjectError>;
    pub async fn on_lines(&self, tx: &mut PgTransaction, session: &TrajectorySession, events: &[TrajectoryEvent]) -> Result<(), ProjectError>;
    pub async fn on_task_event(&self, tx: &mut PgTransaction, event: &TaskEvent) -> Result<(), ProjectError>;
    /// Deletes the Task's non-frozen rows and replays inbox, chain and task_events,
    /// in pages of `REBUILD_PAGE` (1,000) source rows per transaction.
    pub async fn rebuild(&self, task_id: &str) -> Result<(), ProjectError>;
}

// crates/tasks/src/trajectory/ (existing, changed)
impl TrajectoryOperations {
    pub async fn append(&self, req: AppendRequest) -> Result<AppendOutcome, AppendError>;  // + own_from_seq, subagent parent, Projector
    /// The cut event and its session's harness; `None` for no Trajectory.
    pub async fn resolve_cut(&self, task_id: &str, at_turn: Option<i32>) -> Result<Option<Cut>, sqlx::Error>;
    /// S4: the head's own session, or a switch session with its transcode items.
    pub async fn open_switch_session(&self, task_id: &str, harness: Harness) -> Result<Option<OpenedSession>, sqlx::Error>;
    /// S7: a verbatim fork session, or a switch session transcoded from the cut.
    pub async fn open_fork_session(&self, task_id: &str, harness: Harness, cut: Uuid) -> Result<Option<OpenedSession>, sqlx::Error>;
}
impl TrajectoryDao {
    /// Idempotent like `insert_fork_session`.
    pub async fn insert_switch_session(&self, task_id: &str, to: Harness, forked_from: Option<Uuid>, transcoded_from: Uuid) -> Result<TrajectorySession, sqlx::Error>;
}
// trajectory/lineage.rs, pure over the rows of one lineage walk:
/// The other harness's own lines after `since`, from the last compaction on.
fn transcode_items(rows: &[LineageRow], since: Option<Uuid>, to: &str) -> Vec<TranscodeItem>;
```

### Sandbox (`crates/sandbox`)

```rust
// agent/harness/trajectory.rs
pub trait SessionFormat {
    // existing: harness, session_path, locate, retarget, latest
    /// Classifies one line into section 4.1's content; the only reader of a harness's line shape.
    fn read_line(&self, line: &str) -> ClassifiedLine;
    /// Renders harness-neutral items as session lines of this format, in order.
    fn render(&self, items: &[TranscodeItem], ctx: &RenderContext) -> Vec<String>;
}

pub struct RenderContext<'a> {
    pub harness_session_id: &'a str,
    pub cwd: &'a Path,
    pub model: Option<&'a str>,
    /// The file's lines before the rendered ones: Claude chains `parentUuid`
    /// onto the last with a `uuid`; Codex writes `session_meta` when empty.
    pub prior: &'a [&'a str],
    pub timestamp: &'a str,
}
// Tool naming is each format's own mapping in `harness/transcode.rs`; there is
// no shared ToolMap type.

pub enum WriteTarget { Resume(String), Fork, Switch }

pub struct Written {
    pub harness_session_id: String,
    pub line_count: i64,
    pub own_from_seq: Option<i64>,           // Some on Switch
}

// agent/coordinator/service.rs
impl CoordinatorService {
    /// Rebuilds the Member's harness through a `HarnessFactory` closure over
    /// `build_harness` when `profile` differs from the one it runs. `false`
    /// while a turn runs: the delivery is not acknowledged and comes back.
    async fn ensure_profile(&mut self, profile: Option<&str>) -> bool;
}
```

## 7. Projector rules

Every conversation row comes from one of these, written in the transaction that
wrote its source. The Projector reads `content` (section 4.1), never `payload`
or `kind`.

| Source | Becomes | Rule |
|---|---|---|
| `task_inbox` row inserted | User Message, `uuid = inbox id`, positioned now | As today, a queued message takes its position at send |
| Inbox item `delivered` | Same Message, `turn_number` set | Records a change; the position does not move |
| Trajectory user line with text blocks | Nothing | The inbox owns what was sent; the CLI's copy is never shown |
| `content.injected` | Nothing | |
| Agent text blocks | Agent Message | Consecutive text of one assistant turn folds into one Message, as `handle_agent_message` folds it today; `uuid` is the line's `harness_item_id` (Claude's response id, Codex's item id), else the event id |
| `tool_use` block | `tool_call` activity, `in_progress` | |
| `tool_result` block | Completes the call's activity | Found by `tool_use_id`; sets `detail`, `status`, `result_event_id`, and the echo-suppression claim |
| `thinking` block | `thinking` activity, label only | As today; the full text stays in the Trajectory. Codex reasoning becomes one too |
| Line with `derived_from` | Its `tool_call` activities, with `message_uuid` = the Message of the call it names | Shown under that call; the call's own activity stays |
| Line with no `role` and a `usage` | Usage admitted against its `uuid` | Codex's token counts |
| `Skill` `tool_use` block | The Task's `skill-<name>` tag | After commit, as today |
| Line with `seq < own_from_seq` | Nothing | The transcoded prefix |
| `content` NULL | Nothing | Housekeeping |
| Subagent session line | Message or activity with `parent_uuid` = spawning tool call | Served through the existing on-demand Subagent transcript fetch; records no conversation change, as today |
| `agent.error`, sandbox failure in `task_events` | Error Message, or `sandbox_error` activity | As today's handlers map them |
| Sandbox status in `task_events` | `sandbox_status` activity | |
| Turn completed in `task_events` | Turn result Message | `usage` summed from the turn's `content.usage` |
| `task_inbox` row with `origin = 'sandbox'` | Nothing | Nobody said it; it still opens a turn and counts for recovery |
| Backend error result (`persist_error_result`, `message/send.rs:487`), e.g. a failed wake | Error Message, `uuid` = the event id | The `task.error` row and the Message commit in one transaction |
| Frozen row | Left alone | Never rebuilt, never deleted by `rebuild` |
| Interactive call (`AskUserQuestion`, `ExitPlanMode`) projected live | Its activity, and clears every follower's settled mark | As `append` did on `task_messages` |
| Fork creation | The inherited rows, before any own row | Projected from each ancestor's sources up to its cut (4.9a), with the fork's ids; the ancestor's rows are neither read nor shared |

After a commit, the append publishes a `message.upsert` for each top-level
Message it changed and a `message.subagentProgress` for each subagent
transcript. These reach the SSE stream and, through the dispatcher and never
persisted, the Adapters subscribed to them.

An Adapter subscriber reads only these derived events, never Trajectory
content, so it inherits every rule above. It keys what it has posted by the
`tool_use` id, and a Message's text by its uuid and how much of the text it
has posted, since later blocks fold into it; 4.9 makes both deterministic, so a
replayed or re-projected upsert posts nothing twice. The
Linear subscriber keeps that ledger on its in-process turn gate, and posts
only while the gate is open.

## 8. Semantics

| # | While / When | The system shall |
|---|---|---|
| S1 | When a profile change is requested | set `tasks.agent_profile` and `model`; the next turn's delivery carries the new profile and the Member rebuilds its harness before `write_out` |
| S2 | When a switch is requested while a turn runs | let the running turn finish on the old profile; queued inbox items run on the new one |
| S3 | When the new profile has the same harness | resume the latest session verbatim; transcode nothing |
| S4 | When the first turn after a harness switch starts | open a switch session: fork the latest session of the target harness verbatim if one exists, transcode every own line of the other harness since, write the file, and record the prefix with `own_from_seq` |
| S5 | When the transcoded part contains a compaction | transcode from the last compaction, its summary as one user text item |
| S6 | When the chain has a gap | fail that turn with a visible error; do not start the CLI on a partial history |
| S7 | When a fork or rewind cuts in a session of the other harness | open the fork session of the current harness, transcoded from the cut |
| S8 | When a line is appended | insert it and project it in one transaction; a failure rolls back both and the tail resends |
| S9 | When a conversation is rebuilt | produce the same ids and rows as incremental projection; positions are reassigned and `task_conversation_changes` signals a reset |
| S10 | While the tail is `Behind` or `Diverged` | reseed from `ReadTrajectory` and continue; only if the reseed fails, stop that session's recorder and publish `agent.recording_stopped` to `task_events`, which fails no turn. A subagent file is never reseeded |

## 9. Invariants

1. `seq` is the line's index in its session file, including a transcoded prefix,
   so `write_out` reproduces the file byte for byte.
2. `forked_from_event_id` points only into a session of the same harness;
   `chain` never follows `transcoded_from_event_id`.
3. The tail is the only writer of `trajectory_events`; a row is never updated.
4. The Projector is the only writer of `conversation_*` (the one-off frozen copy
   excepted).
5. Only `SessionFormat` implementations read or write a harness's line format.
6. The catalog is the only owner of profile → harness.

## 10. Design principles

| Principle | How the spec meets it |
|---|---|
| 1 One owner | Projector owns `conversation_*`; tail owns `trajectory_events`; inbox owns sent text; catalog owns profile → harness. Derivable facts are not stored: a turn's session, a fork's source session, whether a fork has run. |
| 2 One-way dependencies | The backend decides *which* lines a session starts with; the sandbox decides *how* they are written. The pod's profile is only the boot default; the Task's profile travels with each delivery. |
| 3 Hide decisions | `SessionFormat` hides each file format and its tool naming; `Projector` is one concrete struct. |
| 4 One name | Harness / Agent profile (section 2.1); `adapter` / `external_message_id` / `external_source_name` shared by inbox and activities; `inbox_ids` on the wire. |
| 5 General parts | A profile change within a harness and across harnesses take one path; the switch session reuses the fork session's idempotent creation. |
| 7 Invalid states | Per-origin and per-kind `CHECK`s; `InboxOrigin` sum type; the same-harness fork trigger; no `status`/`kind` free text. |
| 8 Fail fast | Gap fails the turn; missing credential, frozen Task and transport mismatch are refused; backfilled senders are `unknown`, not guessed. |
| 9 Translate at the boundary | Harness line shapes are read only in the sandbox classifier; the backend sees neutral content. |
| 10 Integration failures | Append + projection atomic and resent; deterministic ids make projection idempotent; switch session creation and `AcknowledgeMessages` are idempotent; appends, `pending` and `rebuild` are bounded. |
| 11 Standard | UUIDv5 (RFC 9562) for derived ids. |

---

# Part 2 — Implementation plan

## Phase order

| # | Phase | Depends on | Ships |
|---|---|---|---|
| 0 | Transcoding spike | — | Fixtures and the tool map; no production code |
| 1a | Naming | — | `Harness` / `AgentProfile` everywhere; no behaviour change |
| 1 | Complete, always-on recording | — | Recording for every turn, per-line shipping, subagent sessions, neutral classifier |
| 2 | Profile and harness switching | 0, 1a, 1 | The user-facing feature |
| 3 | Inbox | 1 | `task_inbox` as the source of user messages |
| 4 | Projector cutover | 1, 3 | Agent rows from the Trajectory; `agent.message` removed |
| 5 | Conversation read model | 4 | New tables, readers moved, old tables dropped |
| 6 | Cleanup | 5 | Glossary, AGENTS.md, arch rules |

Phase 2 needs only the recording, so switching ships before the pipeline is
rebuilt. Phases 3–5 can run alongside phase 2. Each phase leaves `main` with one
writer per table, and removes its own temporary scaffolding before it closes.

## Phase 0 — Transcoding spike

- Hand-write a Claude JSONL and a Codex rollout from a recorded two-turn
  conversation in the other format. Resume each with the real CLI and confirm
  the model uses the history (ask what only the history answers).
- Settle `ToolMap`: Claude `Bash`/`Read`/`Edit`/`Write`/`Grep`/… against Codex
  `shell`/`apply_patch`/custom tools. A tool with no counterpart becomes a text
  item describing the call and its result.
- Confirm what each CLI requires of a line it did not write: Claude
  `uuid`/`parentUuid`/`sessionId`/`cwd`; Codex `session_meta`, `turn_context`,
  `call_id` pairing.
- Confirm the compaction lines (Claude summary, Codex `compacted`) and S5.

**Exit:** fixtures under `crates/sandbox/tests/fixtures/transcode/` (one
conversation, both formats, both directions) and the `ToolMap`. If either CLI
refuses a synthesised session, stop: decision 1 depends on it.

**Outcome:** both CLIs accept a synthesised session and answer from it,
including across a compaction and through the text fallback (Claude Code
2.1.281, Codex 0.156.1). The ToolMap, what each CLI requires of a line it did
not write, the injected-line shapes and both compaction shapes are in
`crates/sandbox/tests/fixtures/transcode/README.md`, which phase 2 implements
against. Two facts the spec did not have:
- Claude's compaction keeps a `preservedSegment` (the last assistant reply)
  verbatim before its `compact_boundary`. The summary covers it, so S5 starts at
  the `isCompactSummary` line and does not transcode that segment.
- Codex tolerates an unpaired call or output, and Claude a missing `cwd` or
  `sessionId`; Claude needs the `parentUuid` chain and Codex a `session_meta`
  first line, or they lose the history or refuse the file.

## Phase 1a — Naming

One mechanical PR, no behaviour change; the PR body says so, so a reviewer reads
it apart from the phases that follow.

- `moatless-types`: `Harness` enum; `AgentType` → `AgentProfile`.
- Delete `AgentKind` (`tasks/src/agent/catalog.rs:17`) and
  `trajectory::Harness` (`tasks/src/trajectory/models.rs:9`); both use the
  shared `Harness`. `AgentEntry` → `AgentProfileEntry`, `kind` → `harness`;
  add `catalog::harness_of`. `crates/t3code`'s `UiRpcAgentKind` is the same
  fact and goes too.
- Sandbox: `resolve_agent_harness` (`agent/compose.rs`) returns `Harness`;
  `TaskContext.agent_type` → `agent_profile`; `AGENT_TYPE` env → `AGENT_PROFILE`.
  The sandbox's own runtime struct `Harness` becomes `HarnessRuntime`, since
  arch-rules refuses two workspace types named `Harness`.
- Migration (`0173_agent_profile.sql`): the renames in 4.5, and the top-level
  `agentType` → `agentProfile` key of stored `StreamEvent` payloads in
  `task_events.payload` (`text` in older deployments, `jsonb` in newer), so no
  reader keeps the old key.
- LOOP.md frontmatter lives in user repositories the migration cannot reach:
  `LoopFrontmatter` takes `agentProfile` and accepts `agentType` (its keys are
  camelCase) as a deprecated serde alias, with a test. Task templates were
  dropped (0088) and need none.
- Proto renames in 5.1 (same field numbers); `api-schemas` and OpenAPI; CLI
  flags; T3 (`soaplabs/t3code`) in the same release.

Verification: `rg -w 'agent_type|AgentType|AgentKind|agentType|AGENT_TYPE'`
matches only migrations, the frontmatter alias and its test, and the dated
`docs/frontend-audit-2026-07.md`; full clippy floor; the E2E suite (Test plan,
T0 regression set) green.

## Phase 1 — Complete, always-on recording

Sandbox:
- `trajectory_tail.rs`: replace the 2 s `FLUSH_INTERVAL` poll (`:24`) with a
  file watch (`notify`) plus a fallback poll, as
  `claude-agent-sdk/src/tmux/tailer.rs` does at 100 ms (`:562`). Ship each
  complete line as soon as it is written, at most 256 per append.
- On `Behind`/`Diverged` (`:150-177`) reseed from `ReadTrajectory` (S10).
- Attach the recorder in tmux mode too.
- Record `subagents/agent-*.jsonl` (found as `tmux/tailer.rs:306-308, 607-610`
  finds them) as sessions with `subagent_parent`, named
  `<parent session>:agent-<id>`; the spawning call is the `toolUseId` in the
  `agent-<id>.meta.json` beside each file.
- `read_line` fills `injected`, `usage` and `compaction` (4.1) for both formats.

Backend:
- Migration: `trajectory_sessions.parent_event_id` and its `CHECK` (4.3),
  against `forked_from_event_id` only; phase 2 adds `transcoded_from_event_id`
  to it.
- `trajectory/operations.rs::append`: create subagent sessions.
- Remove `FF_TRAJECTORY_RECORDING` (`crates/core/src/feature_flags.rs`,
  `grpc_server/state.rs`) and `MOATLESS_RESUME_FROM_TRAJECTORY`
  (`trajectory_tail.rs:414`), and `FF_TRAJECTORY_RESUME` (#1009: `core/src/feature_flags.rs`, `server/src/feature_flags.rs`, `grpc_server/credentials.rs::default_resume_from_trajectory`, `tests/shared/profile.ts`); resume always reads the Trajectory, with no Workspace opt-out.

Verification:
- Unit: per-line shipping, reseed, subagent session creation, `read_line`
  classification against recorded lines of both harnesses (injected lines,
  usage, compaction).
- DB (`trajectory/dao_db_tests.rs`): subagent linkage and its `CHECK`.
- A temporary nonprod parity report (`scripts/trajectory-parity.sql`): per
  turn, assistant and tool items in `task_messages` against
  `trajectory_events.content`. It must show zero
  differences for a week before phase 4; phase 4 deletes it.

## Phase 2 — Profile and harness switching

Request surfaces:
- `switch_profile` in `crates/tasks/src/task/switch.rs` (section 6), credential
  check through the catalog's `credential_keys`/`provider_token_providers` and
  `CredentialSync::ensure_fresh_codex_for` (`credential_sync/service.rs:75`),
  which refuses only when no Codex secret (`OPENAI_API_KEY`, `CODEX_AUTH_JSON`)
  is stored;
  transport check against `sandboxes.use_tmux`.
- REST (5.2), T3 (5.3), CLI (5.4).

Turn start:
- The Turn runner sends the Task's `agent_profile` on every delivery (it already
  sends it per delivery).
- `CoordinatorService::ensure_profile` rebuilds the Member's harness through
  `build_harness` (`agent/compose.rs:62`) when the profile changed; `spawn_agent`
  (`:101`) keeps using the pod's profile only for the boot Member's first build.
- The backend decides the switch, not the pod: the delivery carries
  `switch_harness` when the Task's head, or a fork's cut, is a session of the
  other harness (`grpc_server/messages.rs`); `write_out` then picks
  `WriteTarget::Switch` (or `Fork`), and fails the turn when it cannot write it.
  A failed trajectory read fails `GetPendingMessages`, which the pod retries.
- A pod that switches to Codex after it booted writes the missing `auth.json`
  on its next credential refresh.

Sandbox transcoding:
- `SessionFormat::render` for both formats, against the phase 0 fixtures.
- `write_session` (`harness/trajectory.rs:453-526`) handles `Switch`: read
  `ReadTrajectory { switch }`, write the verbatim lines, render and append the
  `TranscodeItem`s, return `own_from_seq`; the tail sends it on the first append.
- A gap → `DATA_LOSS` → visible turn error (S6), through the existing path
  (`grpc_server/trajectory.rs:108-149`). A switch whose write-out fails for any
  other reason is a visible turn error too: the CLI never starts on a fresh
  session that has lost the history.

Backend:
- Migration: `transcoded_from_event_id`, `own_from_seq`, their `CHECK`s and the
  same-harness fork trigger (4.3); `task_forks.source_event_id` (4.6), with no
  backfill (4.6).
- `insert_switch_session`, `transcode_items` (S4, S5).
- Fork creation refuses a frozen source with 409 (5.2).
- `resolve_cut` returns an event; `insert_fork_session` transcodes across
  harnesses (S7); `task/rewind.rs:118` reads the cut session's `harness` instead
  of the Task's, and refuses a Codex cut with no agent turn id only when the
  Trajectory cannot resolve it.
- Until phase 4 the stream still fills `task_messages`. A transcoded prefix is
  never streamed, so the Chat shows no duplicates.

Verification:
- Unit: `render` both directions; switch-back forks verbatim (D8); compaction;
  `ensure_profile` rebuilds only on change.
- DB: the fork trigger rejects a cross-harness `forked_from_event_id`; a retried
  switch read returns the same session.
- Real-CLI tier (`trajectory_e2e.rs`) and E2E (`harness-switch.spec.ts`): see
  the Test plan, T2.

## Phase 3 — Inbox

- Migration: `task_inbox` (4.2). Backfill every pending user row
  (`agent_session_id IS NULL`) as `pending`, with `origin` from 5.5's backfill
  rule, and the acknowledged user rows of an `active` or `interrupted` turn as
  `delivered` with their session and turn: recovery replays from the inbox.
- Create `conversation/projector.rs` with `on_inbox` only, writing the user
  `task_messages` row for now. Every send path calls `InboxService::submit` with
  its `InboxOrigin` (5.5) and the Projector in one transaction, and writes no
  Message itself: `message/send.rs`, `message/handlers.rs`,
  `event_ingress/admission.rs`, `event_ingress/relay.rs`,
  `loops/src/bindings/runtime.rs`, the CLI send path.
- Turn runner: `message/enricher.rs::build_pending_deliveries` (`:136`),
  `message/dao.rs::get_pending_user` (`:899`), `message/recovery_dao.rs` read
  the inbox by `status`, bounded. `AcknowledgeMessages` calls `mark_delivered`.
- Turn trigger (4.5a): port `stamp_turn_trigger` (`message/dao.rs:502`) into
  `mark_delivered`, reading `task_inbox.origin`/`source_task_id` instead of
  `task_messages.user_type`/`source_task_id`; add the `task_turns` `CHECK`.
  `find_interrupted_turn` (`recovery_dao.rs:113`) keeps replaying only an
  `active` or `interrupted` turn (#1003, #1005).
- Sandbox wakes: `enqueue_sandbox_message` (`message/send.rs:114`, served by
  `grpc_server/messages.rs`) submits an `origin = 'sandbox'` item.
- Child notices: `notify_parent_of_child_outcome` (`agent_handlers.rs`) submits
  an `origin = 'task'` item with its derived id; it still skips a turn the
  parent did not trigger (#1011) and waits out `backgroundCommands` (#1008).
- Adapter lookups by external id move to `task_inbox.external_message_id`. The
  adapter crates hold no SQL; they reach it through
  `message/external_events_dao.rs`, which reads the inbox first, then
  `task_messages` (delivered history), then outbound events.
  `event_ingress/admission.rs` and `relay.rs` already send through
  `send_message_flow` and write no Message themselves.
- `mark_delivered(tx, task_id, ids, &RunStamp)`: `RunStamp` carries `run_id`,
  `session_id` and `turn_number`. The Projector's `on_delivered` stamps the
  projected user Message (`sent_at`, session, turn) in the same transaction.
  The trigger is decided from the items that call moved to `delivered`, so a
  repeated acknowledgement decides nothing.
- `set_session_id_for_task` no longer claims a Message whose item is still
  pending; only `mark_delivered` delivers.
- `InterruptedTurn.task_message_id` goes (nothing read it). Rewind deletes the
  inbox items above the cut, as it deletes the Messages.

Verification: DB tests for every `CHECK` (one rejected row per origin rule) and
the unique index; adapter re-sync idempotence; a repeated `AcknowledgeMessages`
succeeds and changes nothing; the recovery tests in `message/send.rs` (e.g.
`a_queued_wake_becomes_a_recoverable_interrupted_turn`),
`message/dao_turn_trigger_db_tests.rs` and
`server/tests/integration_surface/closed_terminal_test.rs` against the inbox; a
redelivered child notice inserts nothing and wakes nobody.

**Exit:** no code outside the Projector inserts a user `task_messages` row.

## Phase 4 — Projector cutover

- Implement `on_lines` (section 7), still writing
  `task_messages`/`task_activities`, ported from `handle_agent_message`
  (`server/src/event_handler/agent_handlers.rs:827`) and the activity mapping.
  Called from `append`, in its transaction. The live notices, tool-call
  metrics and skill tags follow the commit, in the `AppendTrajectory` handler.
- `persist_error_result` (`message/send.rs:487`) writes a `task.error` event
  and its Error Message in one transaction (`on_task_error`). The other
  `task_events` rows keep their handlers until phase 5.
- The lifecycle handlers stay: `handle_agent_completed`/`handle_agent_error` still
  close the turn and notify the parent; only `handle_agent_message` goes.
- A Codex code-mode `exec` call is one `custom_tool_call` in the rollout, but
  the app-server reports each `tools.exec_command` it runs as its own item.
  The sandbox reads each `CommandExecution` line as a `codex.shell` call and
  its result with `derived_from` (4.1). The Projector shows them under the
  `exec` call, which keeps its own activity, and transcoding skips them.
- No per-Task gate: every line appended after the deploy is projected. A Codex
  turn in flight across the deploy may show its commands twice. An old sandbox
  still emits `agent.message` until it ends. The dispatcher persists those as
  plain `task_events` rows and nothing handles them.
- Turn recovery reads progress from an assistant Trajectory line after the
  last `recovery_started` (`recovery_dao.rs`), not from an `agent.message` row.
- The Linear subscriber reads `message.upsert` (section 7) in place of
  `agent.message`.
- Delete `agent.message`: emission in `sandbox/src/agent/coordinator/seam.rs`
  (`:316`, `:692-720`) and `harness/source.rs`; admission, relay and skeleton handling
  (`event_ingress/admission.rs`, `relay.rs`, `server/src/event_handler/skeleton.rs`,
  `dispatcher.rs`); `handle_agent_message`; the `event_ingress_outbox` table.
  Delete the phase 1 parity report.

Verification: a unit test per Projector rule, fed recorded lines from both
harnesses; a test that the Projector never reads `payload` or `kind`; DB test
that append and projection commit or roll back together; T3 projection snapshot
tests (`crates/t3code/src/projection/tests/`) unchanged.

**Exit:** `rg 'agent\.message'` matches only migrations, this plan, the retired
`apps/frontend`, and dated records (`docs/runbooks/`, `docs/frontend-audit-*`,
`plugins/moatless-skills/skills/spec/DESIGN.md`).

## Phase 5 — Conversation read model

- Migration: `conversation_messages`, `conversation_activities` (4.8). Copy
  every `task_messages`/`task_activities` row with `frozen = true` (D4a), and
  every historical user row into `task_inbox` as `delivered` so frozen user
  Messages keep their origin.
- Point the Projector at the new tables, with UUIDv5 ids (4.9); add `rebuild`
  (S9).
- Fork and rewind stop copying: delete `message/fork_dao.rs::copy_conversation`
  (`:105`), `task/fork.rs::copy_conversation` (`:208`),
  `task/rewind_dao.rs::truncate_conversation` (`:28`). A fork's conversation is
  projected from its ancestors' sources (4.9a) at fork creation, and `rebuild`
  replays them; rewind deletes non-frozen rows past the cut and
  re-projects. Replace every read of the `source_session_id` sentinel with 4.6's
  "has the fork recorded an event" query.
- Keep the fork's closed cut turn (#1005, `fork_dao.rs:299-321`): a fork still
  gets one `task_turns` row for the cut turn, `active`/`interrupted` copied as
  `stopped`, with `trigger_decided = true` and no trigger, since nobody awaits a
  fork. It moves out of `copy_conversation` into fork creation and takes the
  turn from `source_event_id` instead of `MAX(task_messages.turn_number)`.
- Move every reader:
  - T3: `transcript_dao.rs`, `activity_dao.rs`, `dao.rs`, `projection/shell.rs`,
    `projection/transcript.rs`, `projection/origin.rs` (origin from
    `task_inbox.origin`).
  - Tasks: `message/{dao,operations,models,transform,error}.rs`,
    `task/{dao,statistics,creation}.rs`, `task_activities/dao.rs`,
    `task_turn/dao.rs`, `feedback/dao.rs`, `ports.rs`, `compose.rs`.
  - Ports: `ports/src/task/transcript.rs`, `pending_work.rs`.
  - Server: `app.rs`, `state.rs`, `compose/ports.rs`.
  - Sandbox control: `compose.rs`, `event_dedup.rs`, `lifecycle/dao.rs`.
  - CLI: `commands/messages.rs`, `commands/feedback.rs`.
  - `sandbox/src/agent/harness/claude/fork.rs`,
    `sandbox-proto/src/event_conversion.rs`.
- Project a task event in the transaction that inserts it
  (`apply_ingress_event`, `insert_event`), not after.
- Rename the `task_activities` module to `activities`.
- Migration: drop the tables and columns in 4.10.
- Deploy: frozen positions are renumbered, so every conversation cursor resets
  once.

Verification: `rg 'task_messages|task_activities'` matches only migrations; full
E2E suite; `rebuild` of a nonprod Task reproduces its rows and ids.

## Phase 6 — Cleanup

- `CONTEXT.md`: add **Agent profile**, **Inbox** and **Projector**; update
  **Agent harness** (switchable between turns, not fixed for the Task's
  lifetime), **Message**, **Chat** and **Subagent transcript** to name their
  sources.
- `crates/tasks/src/conversation/AGENTS.md` (+ `CLAUDE.md` symlink): the
  Projector is the only writer.
- `moatless-arch-rules`: nothing outside `conversation/` writes
  `conversation_*`; nothing outside `agent/harness/` in the sandbox parses a
  session line.
- `docs/deferred-changes.md`: anything knowingly left.

## Test plan

### Where the suite stands (origin/main `d7da7321`)

| Tier | Entry point | Runs in CI | Covers today |
|---|---|---|---|
| Unit and DB | `cargo test -p <crate>`, `task rust:clippy:db-tests` | every PR | DAOs, trigger (`dao_turn_trigger_db_tests.rs`), recovery (`message/send.rs`), `closed_terminal_test.rs` |
| Real-CLI trajectory | `TRAJECTORY_E2E=1 cargo test -p moatless-sandbox --test trajectory_e2e -- --ignored` | no | Record, rebuild, fork, killed-turn resume for both harnesses against `MemoryTrajectoryBackend`; `both_harnesses_give_one_content_shape` |
| API E2E | `scripts/local-e2e/run.sh` (#1010): backend from the checkout, own schema, `SANDBOX_PROVIDER=local`, `local` profile | manual dispatch (`e2e.yml`) | The chains below |

API specs that guard behaviour this change rewrites:

| Spec | Guards | Rewritten in phase |
|---|---|---|
| `agent-execution-flow` (AGENT-01..02.3), `agent-codex-flow` (CODEX-01..05) | Multi-turn Chat per harness | 3, 4, 5 |
| `message-acknowledgment` (MSG-01..09) | Mid-turn messages join the running turn; `isQueued` clears; one turn | 3 |
| `messages` (TASK-12) | User messages readable with no agent run | 3, 5 |
| `sandbox-replacement-ordering` (AGENT-09.1/.2) | A replaced pod resumes before answering | 1, 2 |
| `trajectory-fork-and-recover` (TRAJ-01.1/.2) | Fork sees only its prefix; replaced pod sees all | 1, 2, 5 |
| `agent-stop-recovery` (AGENT-04..06.5) | Stop, restart, history kept | 1, 3 |
| `task-errors` (TASK-09.2/.3, TASK-11) | Profile validation; send to a closed Task | 1a, 3 |
| `task-reply-routing`, `webhook-routing` | Adapter-originated messages | 3 |

Gaps:
- **The trajectory specs never run in CI.** `trajectory` is true only with
  `E2E_TRAJECTORY=1`, and neither `run.sh` nor the `e2e.yml` base env sets it,
  `FF_TRAJECTORY_RECORDING` or `FF_TRAJECTORY_RESUME`, so TRAJ-01 skips.
- The real-CLI tier has no CI job.
- No test covers a child's notice to its parent (#1004/#1011) or a sandbox wake
  end to end. The parent link needs a Task token the suite does not hold, so
  these stay in the DB tier (T3 below).

### T0 — Baseline, one PR before phase 1a

- `run.sh`: start the backend with `FF_TRAJECTORY_RECORDING=1` and
  `FF_TRAJECTORY_RESUME=1`, and pass `E2E_TRAJECTORY=1` to the suite, so TRAJ-01
  runs on every harness run and in CI.
- `e2e.yml`: add a `run_trajectory_cli` input and job running the real-CLI tier
  in `moatless-sandbox-base`, with both credentials, as the transport jobs do.
- Record a green run of the regression set above with both credentials. It is
  the oracle for every later phase: those specs change only where a phase
  renames something they call.

### T1 — Changes to existing tests

| Phase | Change |
|---|---|
| 1a | `moat-api.ts`: `agentType` → `agentProfile`, `--agent-type` → `--agent-profile`; every spec's `agentType:` field; TASK-09.2/.3 names and messages |
| 1 | Delete the `trajectory` capability, `E2E_TRAJECTORY` and T0's `run.sh` flags; TRAJ-01 always runs |
| 4 | A shared `expectConversation(api, taskId)` step, called at the end of every regression chain: message ids unique; per turn, one user Message per delivered item; no user Message for a sandbox wake. It catches a double write while both the stream and the Projector exist |
| 5 | TRAJ-01 also asserts the fork's Chat holds exactly the parent's Messages up to the cut (projected, no longer copied), and the replaced pod's Turns 1–2 are unchanged after Turn 3 (reseed wrote no duplicates, S10) |

### T2 — New tests, by phase

**Real-CLI tier** (`crates/sandbox/tests/trajectory_e2e.rs`,
`support/memory_trajectory.rs` gains `ReadTrajectory { switch }`):

| Phase | Test | Asserts |
|---|---|---|
| 0 | `claude_to_codex_resume_knows_history`, `codex_to_claude_resume_knows_history` | The other CLI accepts the synthesised session and answers from it |
| 0 | `transcoded_tool_call_is_described` | A Claude `Write` in Turn 1 is answerable from Codex (ToolMap, and the text fallback for a tool with no counterpart) |
| 0 | `transcoded_compaction_keeps_the_summary` | A compacted session transcodes from its summary (S5) |
| 1 | `claude_records_a_subagent_session` | `subagents/agent-*.jsonl` becomes a session with `subagent_parent` |
| 1 | `both_harnesses_give_one_content_shape` (extended) | `injected`, `usage`, `compaction` filled for both formats |
| 1 | `lines_ship_before_the_turn_ends` | Lines reach the backend while the turn runs, at most 256 per append |
| 2 | `switch_back_resumes_verbatim` | Claude → Codex → Claude forks the first Claude session verbatim (D8) |

**API E2E** (`tests/api/specs/harness-switch.spec.ts`, new; needs
`claudeCredentials` and `codexConnection`; `moat-api.ts` gains
`switchProfile(taskId, profile, model?)` over `moat tasks update --agent-profile`):

| Scenario | Steps | Asserts |
|---|---|---|
| SWITCH-01 | Claude (`haiku`): Turn 1 writes `release.txt` naming PERIWINKLE | File exists |
| SWITCH-02 | Switch to `codex` (`gpt-5.5`); Turn 2 asks what Turn 1 wrote and where, no tools | Answer names PERIWINKLE and `release.txt`; model starts `gpt-` |
| SWITCH-03 | Switch back to `claude-code`; Turn 3 gives SAFFRON; Turn 4 lists every name | Both names; model starts `claude-` |
| SWITCH-04 | After SWITCH-02, `expectConversation` | Turn 1's Messages appear once: the transcoded prefix was not streamed again |
| SWITCH-05 | Stop the Sandbox after SWITCH-02; send a question | The replacement resumes the Codex session from the Trajectory (AGENT-09 across a switch) |
| SWITCH-06 | `claude-code` → `claude-code-cli` | Same harness: no new transcoded session; answer from history |
| SWITCH-07 | Fork a Claude Task at Turn 1 into `codex` | Fork knows Turn 1 only (S7) |
| SWITCH-08 | Rewind a Task into its Codex turn while on Claude | Answers from the Codex turn's history (`rewind.rs` reads the cut session's harness) |

The spec runs SWITCH-01 to SWITCH-07 as one chain. SWITCH-08 has no REST
rewind to drive it; the DB test `a_rewind_into_a_codex_turn_transcodes_it_for_claude`
(`crates/tasks/src/task/rewind_db_tests.rs`) covers it instead, up to the
session the next turn resumes.

Refusals, with no agent run (`task-errors.spec.ts`): switching to the current
profile answers 200 and changes nothing; an unknown profile answers 400; a
switch to `claude-code-tui` in a running non-tmux Sandbox answers 409 (a local
Sandbox is never tmux).

**Unit, DB and integration** (every PR):

| Phase | Test |
|---|---|
| 2 | `crates/server/tests/integration_surface/`: the REST table in 5.2 row by row, including the missing-credential 400 and both frozen-Task 409s (switch and fork), which the E2E cannot set up |
| 2 | DB: a retried switch read returns the same session; the fork trigger rejects a cross-harness `forked_from_event_id` |
| 3 | DB: one rejected row per origin `CHECK`; the adapter unique index; `mark_delivered` ports every case in `dao_turn_trigger_db_tests.rs`; a turn opened only by `sandbox` items inherits the previous trigger; the `task_turns` trigger `CHECK` |
| 3 | A redelivered child notice inserts nothing and wakes nobody; a notice waits out `backgroundCommands`; a parent that did not trigger the turn gets none (#1004, #1008, #1011) |
| 4 | One unit test per Projector rule, fed the recorded fixtures of both harnesses; the Projector never reads `payload` or `kind`; append and projection commit together; `persist_error_result` yields an Error Message through a `task.error` event |
| 5 | `rebuild` reproduces rows and ids; a fork gets its closed cut turn with no trigger (#1005); a fork of a fork holds each ancestor's rows up to its cut, with ids distinct from every ancestor's and stable across `rebuild`, and no inherited row carries an echo-suppression claim; T3 projection snapshots unchanged |

### Per-phase gate

A phase merges when its unit and DB tests pass, the T0 regression set is green
through `run.sh` with both credentials, and its T2 rows pass. Phases 0 and 2 also
need the real-CLI tier green.

## Verification commands (every phase)

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
task rust:clippy:db-tests
cargo test -p <touched crate>
cargo test -p moatless-arch-rules
cargo run -p moatless-arch-rules --bin arch-report -- --comments --since main
bun run type:check && bun run biome:check     # when T3-facing TypeScript changes
```

E2E: `scripts/local-e2e/run.sh` (the `e2e-testing` skill), per the Test plan.

## Risks

| Risk | Where it bites | Mitigation |
|---|---|---|
| A CLI refuses a session it did not write | Phase 2 | Phase 0 gates it |
| Recording misses lines in an untested mode | Chat goes blank after phase 4 | Phase 1 parity report clean for a week first |
| The classifier misses an injected line shape | Harness context transcoded into the other harness as if the user wrote it | Recorded-line fixtures per harness version, refreshed when a CLI is bumped |
| Projection slows `AppendTrajectory` | Every line | Measure in phase 4; one small upsert per line |
| Positions change on rebuild | T3 cursors | `task_conversation_changes` reset signal; T3 reloads |
| The rename breaks an external API caller | Phase 1a | T3 and CLI are the only clients; ship together |
| The rename breaks a user's LOOP.md | Phase 1a | `agentType` stays a deprecated alias at that parse boundary |

## Settled questions

- **Forking a frozen Task** is refused with 409 (5.2); `source_event_id` stays
  the only anchor and phase 5 deletes `copy_conversation` as planned. Every Task
  created before phase 1 deploys is frozen, since recording was off by default.
- **Names** are the ones this plan uses: "Agent profile" for the catalog entry;
  `task_inbox`, `conversation_messages`, `conversation_activities` for the
  tables. The glossary's **Message** and **Activity** stay the domain words.
