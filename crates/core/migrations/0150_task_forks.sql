-- Forking a Task: where a fork's cut point lives, and the id a Codex cut needs.
--
-- A fork holds its own copy of the parent's conversation (see the row copy in
-- `task/fork.rs`). What a copy cannot carry is the agent's own session: that is
-- branched in the pod, on the fork's first turn, from the three anchors below.

CREATE TABLE IF NOT EXISTS task_forks (
    -- The fork. Primary key, so a Task is forked from at most one parent.
    task_id              TEXT PRIMARY KEY REFERENCES tasks(task_id) ON DELETE CASCADE,
    -- The parent, denormalised from the `forked_from` relation so the delivery
    -- path reads one table. No foreign key, deliberately: a fork outlives the
    -- Task it branched from, and deleting the parent must not take the fork's
    -- ability to resume with it.
    source_task_id       TEXT NOT NULL,
    -- The parent `task_turns.turn_number` the cut keeps — the caller's
    -- selector, and what the two anchors below were resolved from. NULL when
    -- the request named no turn and the cut is at head.
    at_turn              INTEGER,
    -- The parent's `agent_session_id` at the cut. Also the sentinel: fork
    -- intent rides deliveries only while the fork's own latest
    -- `agent_session_id` still equals this. Once the fork's first turn reports
    -- a session, the two differ and the fork resumes like any other Task —
    -- which is why there is no `applied_at` here to be consumed by a first turn
    -- that dies before a session lands.
    source_session_id    TEXT NOT NULL,
    -- Claude's cut anchor: the last agent-written `task_messages.uuid` at or
    -- before `at_turn`, which is the same uuid the CLI writes on the transcript
    -- line. The pod copies the transcript up to and including this line.
    -- NULL for a Codex fork.
    source_message_uuid  TEXT,
    -- Codex's cut anchor, passed to `thread/fork` as `lastTurnId` ("fork
    -- through, inclusive"). Copied from the parent's `task_turns.agent_turn_id`.
    -- NULL for a Claude fork, and for a fork at head, which needs no id.
    source_agent_turn_id TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The delivery path's lookup is by primary key. This one is for the reverse
-- question — "what was forked off this Task" — which the parent's detail view
-- asks.
CREATE INDEX IF NOT EXISTS task_forks_source_idx ON task_forks (source_task_id);

-- The harness's own id for a turn.
--
-- Codex mints one per `turn/start` and the harness holds it in process for the
-- life of the turn; nothing persisted it, so no past turn could be named to
-- `thread/fork`. It rides the run's `session_init` event and is written onto the
-- turn by `handle_agent_session_init` (`set_agent_turn_id`), first-writer-wins so
-- a re-delivered event cannot overwrite it.
--
-- NULL for Claude, whose transcript is keyed by message uuid and carries no
-- turn id at all, and NULL on every turn that ran before this shipped — which
-- is why forking a Codex Task at such a turn is refused rather than silently
-- widened to the whole thread.
ALTER TABLE task_turns ADD COLUMN IF NOT EXISTS agent_turn_id TEXT;
