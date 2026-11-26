-- Give every historic turn its `task_turns` row. 0071 seeded only the turn
-- open at rollout, so an older Task's earlier turns exist only as
-- `conversation_messages.turn_number`. A turn a rewind moved to
-- `task_rewound_turns` stays there, rather than coming back as live.
-- `ended_at` counts the turn's own events too: an activity after it makes the
-- reaper's `list_settled_turns_with_later_activity` treat the run as live.
-- Aggregate `task_events` once and join, rather than a subquery per turn:
-- only `(task_id)` indexes it, so per-turn lookups rescan every task's events.
INSERT INTO task_turns (task_id, turn_number, status, started_at, ended_at)
SELECT h.task_id, h.turn_number, 'completed', h.started_at,
       GREATEST(h.ended_at, e.last_event)
  FROM (
      SELECT m.task_id, m.turn_number,
             MIN(m.created_at) AS started_at, MAX(m.created_at) AS ended_at
        FROM conversation_messages m
       WHERE m.turn_number IS NOT NULL
         AND NOT EXISTS (
             SELECT 1 FROM task_turns t
              WHERE t.task_id = m.task_id AND t.turn_number = m.turn_number)
         AND NOT EXISTS (
             SELECT 1 FROM task_rewound_turns r
              WHERE r.task_id = m.task_id AND r.turn_number = m.turn_number)
       GROUP BY m.task_id, m.turn_number
  ) h
  LEFT JOIN (
      SELECT task_id, turn_number, MAX(created_at) AS last_event
        FROM task_events
       WHERE turn_number IS NOT NULL
       GROUP BY task_id, turn_number
  ) e ON e.task_id = h.task_id AND e.turn_number = h.turn_number
    ON CONFLICT (task_id, turn_number) DO NOTHING;
