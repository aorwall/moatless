-- task_turns: authoritative per-turn lifecycle state.
--
-- Previously turn openness was DERIVED from the task_events log (a DISTINCT ON
-- query in task_event/dao.rs), making the event stream the source of truth for
-- business state — a lost or delayed terminal event meant stale state,
-- compensated only by reaper heuristics. This table makes turn state a
-- first-class row written transactionally at the gRPC event ingress (see
-- task_turn/dao.rs::apply_ingress_event), so events become projections rather
-- than the source of truth. The Redis event-handler consumer keeps writing the
-- same state as an idempotent fallback during rollout, and the reaper
-- reconciles turns left stuck open from this table instead of the event log.

CREATE TABLE IF NOT EXISTS task_turns (
    task_id      text        NOT NULL,
    turn_number  integer     NOT NULL,
    -- active | completed | error | stopped | interrupted
    status       text        NOT NULL DEFAULT 'active',
    agent_type   text,
    started_at   timestamptz NOT NULL DEFAULT now(),
    ended_at     timestamptz,
    PRIMARY KEY (task_id, turn_number)
);

-- Reaper backstop query: find non-terminal turns older than a cutoff. The
-- partial index keeps it tiny — only open turns are indexed, and the common
-- case is zero rows.
CREATE INDEX IF NOT EXISTS idx_task_turns_open
    ON task_turns (started_at)
    WHERE status IN ('active', 'interrupted');

-- Backfill currently-open turns from the existing task_events log so the reaper
-- and status reads don't lose track of turns that were in flight at rollout.
-- Mirrors the DISTINCT ON query this table replaces: a task's single latest
-- taskTurn event, kept only when it is still `active`.
INSERT INTO task_turns (task_id, turn_number, status, agent_type, started_at)
SELECT latest.task_id,
       COALESCE(latest.turn_number, 1),
       'active',
       latest.agent_type,
       latest.created_at
FROM (
    SELECT DISTINCT ON (task_id)
           task_id, sub_type, turn_number, agent_type, created_at
    FROM task_events
    WHERE type = 'taskTurn'
    ORDER BY task_id, created_at DESC, id DESC
) latest
WHERE latest.sub_type = 'active'
ON CONFLICT (task_id, turn_number) DO NOTHING;
