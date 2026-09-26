-- Temporary nonprod report: per Turn, the assistant Messages and tool calls
-- the conversation tables hold against the trajectory's. Phase 4 of
-- `.plans/harness-switching.md` deletes it; it must show no row for a week
-- before phase 4 lands.
--
--   psql "$DATABASE_URL" -v since="'2026-09-26'" -f scripts/trajectory-parity.sql
--
-- Tool calls are counted per Task rather than matched by id: Codex's
-- `task_activities.tool_use_id` is the app-server item id, not the rollout's
-- `call_id`.
WITH tasks AS (
    SELECT DISTINCT task_id
      FROM trajectory_sessions
     WHERE created_at >= :since::timestamptz
),
-- A subagent session's lines belong to no Turn of the Task's own.
own_blocks AS (
    SELECT s.task_id, e.turn_number, e.harness_item_id, b
      FROM trajectory_sessions s
      JOIN tasks USING (task_id)
      JOIN trajectory_events e ON e.session_id = s.id
     CROSS JOIN LATERAL jsonb_array_elements(e.content -> 'blocks') b
     WHERE s.parent_event_id IS NULL
       AND e.content ->> 'role' = 'assistant'
       AND NOT coalesce((e.content ->> 'injected')::boolean, false)
),
recorded_texts AS (
    SELECT task_id, turn_number, count(DISTINCT harness_item_id) AS n
      FROM own_blocks
     WHERE b ->> 'type' = 'text' AND b ->> 'text' <> ''
     GROUP BY 1, 2
),
message_texts AS (
    SELECT m.task_id, m.turn_number, count(*) AS n
      FROM task_messages m
      JOIN tasks USING (task_id)
     WHERE m.message_role = 'assistant'
       AND m.parent_uuid IS NULL
       AND coalesce(m.message_text, '') <> ''
     GROUP BY 1, 2
),
recorded_calls AS (
    SELECT task_id, count(*) AS n
      FROM own_blocks
     WHERE b ->> 'type' = 'tool_use'
     GROUP BY 1
),
activity_calls AS (
    SELECT a.task_id, count(*) AS n
      FROM task_activities a
      JOIN tasks USING (task_id)
     WHERE a.kind = 'tool_call' AND a.parent_uuid IS NULL
     GROUP BY 1
)
SELECT 'assistant responses with text' AS difference, task_id, turn_number,
       format('trajectory %s, task_messages %s', coalesce(t.n, 0), coalesce(m.n, 0))
           AS detail
  FROM recorded_texts t
  FULL JOIN message_texts m USING (task_id, turn_number)
 WHERE coalesce(t.n, 0) <> coalesce(m.n, 0)
UNION ALL
SELECT 'tool calls', task_id, NULL,
       format('trajectory %s, task_activities %s', coalesce(r.n, 0), coalesce(a.n, 0))
  FROM recorded_calls r
  FULL JOIN activity_calls a USING (task_id)
 WHERE coalesce(r.n, 0) <> coalesce(a.n, 0)
 ORDER BY 2, 3, 1;
