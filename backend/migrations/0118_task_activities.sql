-- What the agent *did* on the way to what it said, as rows.
--
-- Spec: `.plans/event-sourced-conversation.md`. A Message is something a person
-- or the agent said — role and prose. An activity is a tool call carrying its
-- own outcome, a stretch of thinking, a Sandbox status change or a Sandbox
-- error. Both readers already draw that line and redraw it on every read; this
-- stores what they compute.
--
-- This migration creates and adds. The two drops are left to the operator
-- runbook (`docs/runbooks/conversation-storage-migration.md`) after a go/no-go,
-- because those are the steps with no rollback.

-- One row per tool call, stretch of thinking, or Sandbox lifecycle change.
CREATE TABLE IF NOT EXISTS task_activities (
    -- Derived from the source, never generated: `{message_uuid}:{block_index}`
    -- for a block, the event id for a Sandbox activity. That is what makes
    -- redelivery a no-op rather than a duplicate — the same block computes the
    -- same key.
    activity_id  text   PRIMARY KEY,
    -- No foreign key, as on `task_conversation_changes` and for the same
    -- reason: a record must outlive what it names.
    task_id      text   NOT NULL,
    -- From `task_conversation_position`, allocated in the writing transaction.
    -- This is what interleaves activities with the Messages they happened on.
    position     bigint NOT NULL,
    -- Provenance, not a foreign key. The Message row may not exist — an
    -- assistant turn that only called tools said nothing and creates none — and
    -- a Sandbox activity happened on no Message at all.
    message_uuid text,
    -- The spawning `Agent` tool-use id for a subagent's activity, matching
    -- `task_messages.parent_uuid`. NULL for the Task's own conversation.
    parent_uuid  text,
    -- Closed set: `tool_call`, `thinking`, `sandbox_status`, `sandbox_error`. A
    -- value outside it is a bug, not an extension point, which is why this is
    -- not an enum type: adding a value must be a code change that fails to
    -- compile, not a migration.
    kind         text   NOT NULL,
    -- `in_progress` until a tool call's result lands, then `completed` or
    -- `failed`. This is what makes one row per call work.
    status       text   NOT NULL,
    -- Load-bearing. A result arrives on a *later* Message than its call, so it
    -- cannot derive the call's `{message_uuid}:{block_index}` key and finds the
    -- row by this instead.
    tool_use_id  text,
    -- The collapsed label: the tool's name, or the Sandbox status.
    summary      text   NOT NULL,
    -- Tool name, item type, status, command, and the tool result **whole**. The
    -- Moatless frontend asks for one activity at a time and renders all of it.
    payload      jsonb  NOT NULL DEFAULT '{}'::jsonb,
    -- The first 2,000 characters of that result's visible output, plus the
    -- marker saying it was cut.
    --
    -- The T3 surface re-sends its whole work log every time the conversation
    -- moves, so it carries a preview rather than the output. Cutting it once
    -- here, rather than on every send, is what lets that surface read a column
    -- bounded by 2 kB instead of re-cutting a result that has no bound at all.
    -- NULL until the call comes back, and on every activity that is not a tool
    -- call.
    detail       text,
    -- Same axis as `task_events.is_error`.
    is_error     boolean NOT NULL DEFAULT false,
    -- What this call created on the other side, when it created something.
    --
    -- `gh pr comment` posts a comment; the id it prints is then claimed so the
    -- webhook echo of that same comment is recognised as the agent's own and not
    -- delivered back as a new Message. The claim belongs on the call rather than
    -- on a Message: a turn that only ran the command said nothing and writes no
    -- Message, the call always has a row, and the call is what made the comment.
    external_event_id    text,
    external_adapter     text,
    external_source_name text,
    created_at   timestamptz NOT NULL DEFAULT NOW()
);

-- The dedup gate's global lookup: which Task, if any, already claims this id.
CREATE INDEX IF NOT EXISTS task_activities_external_idx
    ON task_activities (external_event_id)
    WHERE external_event_id IS NOT NULL;

-- The conversation read: every activity of one Task, in stream order.
CREATE INDEX IF NOT EXISTS task_activities_task_position_idx
    ON task_activities (task_id, position);

-- The result update. Unique because one call is one row — this is the
-- constraint that makes `settle` a single statement with no ambiguity about
-- which row it means. Partial, because a thinking or Sandbox activity has no
-- tool-use id and several NULLs are not a conflict.
CREATE UNIQUE INDEX IF NOT EXISTS task_activities_tool_use_idx
    ON task_activities (task_id, tool_use_id)
    WHERE tool_use_id IS NOT NULL;

-- A subagent transcript is assembled on demand behind the `Agent` call that
-- spawned it, so its activities are fetched by that call's id.
CREATE INDEX IF NOT EXISTS task_activities_parent_idx
    ON task_activities (task_id, parent_uuid)
    WHERE parent_uuid IS NOT NULL;

-- Reading a changed Message's activities back, by the Message they happened on.
CREATE INDEX IF NOT EXISTS task_activities_message_idx
    ON task_activities (task_id, message_uuid)
    WHERE message_uuid IS NOT NULL;

-- A Message's prose, as prose.
--
-- Nullable: the column is added to 2.6 M existing rows, and a nullable add is a
-- catalogue-only change where a `NOT NULL DEFAULT ''` would rewrite the whole
-- 4,143 MB table. NULL is also what the runbook's fold reads as "not folded
-- yet", so the predicate that drives it is the progress it makes.
ALTER TABLE task_messages ADD COLUMN IF NOT EXISTS message_text text;

-- The splice buffer for a Message still being assembled.
--
-- Prose arrives one block at a time, out of order and sometimes twice.
-- `message_text_parts[i] = $x` is subscripted assignment, which enlarges the
-- array and creates it from NULL, so a block lands in the same slot however it
-- arrives.
-- `message_text` is then `array_to_string(parts, E'\n\n')`, which skips NULL
-- holes left by blocks that are not prose.
--
-- Not a generated column: `array_to_string` is STABLE, not IMMUTABLE, so
-- PostgreSQL will not have it.
ALTER TABLE task_messages ADD COLUMN IF NOT EXISTS message_text_parts text[];

-- `message_content` stops being written before it stops existing. The writer
-- binds `message_text` now, and a `jsonb NOT NULL` column with no default would
-- reject every insert from the moment that lands.
ALTER TABLE task_messages ALTER COLUMN message_content DROP NOT NULL;

-- Where a subagent's token usage lives once its Messages are gone.
--
-- `get_subagent_aggregates` (`message/dao.rs`) puts a token count on an `Agent`
-- tool call by summing `assistant_usage_*` over `task_messages`. Those sums live
-- almost entirely on assistant turns that only called tools — turns that said
-- nothing, and that runbook step 3 deletes. This is the open question the spec
-- left against the migration, answered by taking its own option (b): lift the
-- usage into a table keyed on the Message's uuid, so the aggregate survives the
-- row it was computed from.
--
-- One row per Message that reported usage. Admitted once, by primary key, for
-- the same reason `task_messages` admits it once: a CLI that repeats usage on
-- every per-block source event must not count it twice.
CREATE TABLE IF NOT EXISTS task_message_usage (
    message_uuid  text PRIMARY KEY,
    task_id       text NOT NULL,
    -- The spawning `Agent` tool-use id, or NULL for the Task's own turns.
    parent_uuid   text,
    input_tokens  integer NOT NULL DEFAULT 0,
    output_tokens integer NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS task_message_usage_parent_idx
    ON task_message_usage (task_id, parent_uuid)
    WHERE parent_uuid IS NOT NULL;
